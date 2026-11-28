"""Kokoro-82M, distilled for batching, with a batched forward pass, loaded from safetensors.

The text side is Kokoro's modules (the kokoro fork, pinned in pyproject.toml) with its six LSTMs
replaced by distilled convolution blocks (convseq.py); the frame side, the F0/N predictor and the
decoder, is batch-first (frame.py): Kokoro's predictor blocks with per-frame normalization, and a
Vocos-style decoder in place of Kokoro's. convert.py installs the distilled weights.

- Text side (ALBERT, text encoder, duration predictor) is exact under padding: ALBERT gets an
  attention mask, and padded positions are zeroed before every convolution (as the convolution's
  own zero padding would be). Durations therefore match batch 1 (exactly in fp32; fp16 numerics
  depend a little on batch size).
- Frame side: the batch's sentences are padded to the longest and masked; everything there is
  local, so each sentence's audio is what it would be alone, apart from its last few
  milliseconds (frame.Decoder). `max_batch=1` in the server turns batching off.
- Convolutions and the STFT run through kernels.py (a Triton conv1d, matmul transposed convs and
  STFT) instead of cuDNN and cuFFT, which build a plan per shape: any length, first time or not,
  costs the same.
"""

import importlib.util
import json
import logging
import re
import os
import sys
import types
from dataclasses import dataclass
from typing import Iterator

import torch
import torch.nn.functional as F
from safetensors.torch import load_file

from . import convseq, frame, kernels
from .protocol import SAMPLES_PER_FRAME, max_duration

log = logging.getLogger(__name__)

VOICE_NAME = re.compile(r"[a-z0-9_]{1,64}")


def _import_kokoro():
    """Import kokoro's model modules without running kokoro/__init__.py, which imports misaki (G2P)."""
    if "kokoro" not in sys.modules:
        spec = importlib.util.find_spec("kokoro")
        if spec is None or not spec.submodule_search_locations:
            raise ImportError("kokoro is not installed")
        package = types.ModuleType("kokoro")
        package.__path__ = list(spec.submodule_search_locations)
        sys.modules["kokoro"] = package
    from loguru import logger

    logger.disable("kokoro")
    import kokoro.model

    return kokoro.model


def build_kmodel(config: dict) -> torch.nn.Module:
    """An untrained KModel with weight norm already folded, ConvSeqs in place of its LSTMs and
    frame.Decoder in place of its decoder, ready for a converted state dict. The F0/N predictor's
    AdaIN1d modules hold the batch-first weights; frame.f0n applies them per frame."""
    kmodel = _import_kokoro()
    from transformers import AlbertConfig

    from kokoro.istftnet import SourceModuleHnNSF
    from kokoro.modules import CustomAlbert, ProsodyPredictor, TextEncoder

    # Same modules as KModel.__init__, which would otherwise insist on torch.load-ing a checkpoint.
    m = kmodel.KModel.__new__(kmodel.KModel)
    torch.nn.Module.__init__(m)
    m.repo_id = "hexgrad/Kokoro-82M"
    m.vocab = config["vocab"]
    m.bert = CustomAlbert(
        AlbertConfig(vocab_size=config["n_token"], **config["plbert"]), add_pooling_layer=False
    )
    m.bert_encoder = torch.nn.Linear(m.bert.config.hidden_size, config["hidden_dim"])
    m.context_length = m.bert.config.max_position_embeddings
    m.predictor = ProsodyPredictor(
        style_dim=config["style_dim"],
        d_hid=config["hidden_dim"],
        nlayers=config["n_layer"],
        max_dur=config["max_dur"],
        dropout=config["dropout"],
    )
    m.text_encoder = TextEncoder(
        channels=config["hidden_dim"],
        kernel_size=config["text_encoder_kernel_size"],
        depth=config["n_layer"],
        n_symbols=config["n_token"],
    )
    # Kokoro's harmonic source, at the decoder's 80 frames per second (it upsamples by HOP).
    source = SourceModuleHnNSF(sampling_rate=24000, upsample_scale=frame.HOP, harmonic_num=8, voiced_threshod=10)
    m.decoder = frame.Decoder(source, asr_dim=config["hidden_dim"])
    m._fold_weight_norm()
    convseq.install(m)
    return m


def _fresh(fn, tag: str):
    """A wrapper around fn with its own code object. torch.compile keeps its graphs per code object,
    so wrappers made here get separate caches."""

    def call(*args, **kwargs):
        return fn(*args, **kwargs)

    name = getattr(fn, "__name__", type(fn).__name__)
    call.__code__ = call.__code__.replace(co_name=f"{tag}_{name}")
    return call


@dataclass
class Request:
    ids: list[int]  # phoneme ids, without the boundary 0s
    ref_s: torch.Tensor  # [256] style vector on the model's device
    speed: int  # hundredths
    waiting: bool = True  # a listener is waiting for it now (False: prefetch)


@dataclass
class Result:
    index: int  # position in the list passed to Synth.run
    pcm: bytes | None  # little-endian int16 samples; None if the request failed
    durations: list[int] | None  # frames per id, boundary ids included
    error: str | None = None


class Synth:
    def __init__(
        self,
        model_dir: str,
        device: str = "cuda",
        half: bool = True,
        compile: bool = True,
        max_chunk_samples: int = 1_440_000,
        frame_budget: int | None = None,
    ):
        with open(os.path.join(model_dir, "config.json"), encoding="utf-8") as f:
            config = json.load(f)
        self.vocab_size = int(config["n_token"])
        self.vocab_ids = frozenset(int(i) for i in config["vocab"].values())
        self.max_phonemes = int(config["plbert"]["max_position_embeddings"]) - 2
        self.device = torch.device(device)
        self.half = half and self.device.type == "cuda"
        self.max_chunk_samples = max_chunk_samples
        # A frame-side batch, padded, holds at most as many frames as one chunk may have, so a batch
        # never needs more memory than the longest single request.
        self.frame_budget = frame_budget or max_chunk_samples // SAMPLES_PER_FRAME

        model = build_kmodel(config)
        model.load_state_dict(load_file(os.path.join(model_dir, "model.safetensors")), strict=True)
        model = model.eval().requires_grad_(False).to(self.device)
        if self.half:
            model = model.half()
            # The harmonic source's phase sum needs fp32, and so do the STFT matrices: rebuilt, since
            # half() has already rounded them.
            model.decoder.source.float()
            model.decoder.stft = kernels.MatmulSTFT(frame.N_FFT, frame.STFT_HOP).to(self.device)
        kernels.apply(model)
        self.model = model

        voices = load_file(os.path.join(model_dir, "voices.safetensors"))
        self.voice_names = sorted(voices)
        bad = [n for n in self.voice_names if not VOICE_NAME.fullmatch(n)]
        if bad or not self.voice_names:
            raise ValueError(f"voice names must match [a-z0-9_]{{1,64}}: {bad[:5]}")
        packs = torch.stack([voices[name] for name in self.voice_names])
        if packs.shape[1:] != (510, 256) or not bool(torch.isfinite(packs).all()):
            raise ValueError(f"bad voice packs: {tuple(packs.shape)}")
        self.voices = packs.to(self.device)  # [n_voices, 510, 256] fp32

        self._fn = {
            "bert": model.bert,
            "text_encoder": self._text_encoder_eager,
            "duration": self._duration_eager,
            "shared": model.predictor.shared,
            "f0n": lambda x, s, m_f, m_2f: frame.f0n(model.predictor, x, s, m_f, m_2f),
            "decoder": model.decoder,
        }
        if compile:
            self._fn = {k: torch.compile(_fresh(f, k), dynamic=True) for k, f in self._fn.items()}

    def style(self, voice: int, style_len: int) -> torch.Tensor:
        """Voice pack `voice` at row style_len - 1 (KPipeline's pack[len(ps) - 1])."""
        return self.voices[voice, style_len - 1]

    def _text_encoder_eager(self, ids: torch.Tensor, keep: torch.Tensor) -> torch.Tensor:
        """keep [B, T, 1] -> t_en [B, 512, T], zero at padding."""
        te = self.model.text_encoder
        keep_c = keep.transpose(1, 2)
        x = te.embedding(ids).transpose(1, 2) * keep_c
        for block in te.cnn:
            x = block(x) * keep_c
        return (te.lstm(x.transpose(1, 2), keep) * keep).transpose(1, 2)

    def _duration_eager(self, d_en: torch.Tensor, s: torch.Tensor, keep: torch.Tensor):
        """The duration predictor: d [B, T, 640] (zero at padding) and the summed duration logits [B, T]."""
        enc = self.model.predictor.text_encoder
        s_t = s[:, None, :].expand(-1, d_en.shape[1], -1)
        x = torch.cat([d_en, s_t], dim=-1) * keep
        for i, block in enumerate(enc.lstms):
            x = block(x, keep) if i % 2 == 0 else torch.cat([block(x, s), s_t], dim=-1) * keep
        d = x
        x = self.model.predictor.lstm(d, keep)
        return d, torch.sigmoid(self.model.predictor.duration_proj(x)).sum(-1)

    def _durations(self, ids, mask, s, speed, max_dur):
        """Text side for the whole batch: (pred_dur [B, T] with 0 at padding, d [B, T, 640], t_en [B, 512, T])."""
        m = self.model
        keep = (~mask)[..., None]
        fn = self._fn
        bert = fn["bert"](ids, attention_mask=keep[..., 0].long())
        d_en = m.bert_encoder(bert)  # [B, T, 512]
        t_en = fn["text_encoder"](ids, keep.to(d_en.dtype))
        d, duration = fn["duration"](d_en, s, keep.to(d_en.dtype))
        duration = duration / speed[:, None]
        # The clamp to ceil(50 / speed) never binds (the sum is at most 50); it makes the bound the
        # front checks hold whatever the float rounding.
        pred_dur = torch.round(duration).long().clamp(min=1).minimum(max_dur).masked_fill(mask, 0)
        return pred_dur, d, t_en

    def _rows(self, frames: list[int], waiting: list[bool]) -> list[list[int]]:
        """Split items, most urgent first, into frame-side batches whose padded size (items x the
        longest) is at most frame_budget frames.

        A batch that holds a chunk a listener is waiting for takes no prefetch after it, so waiting
        chunks decode together, ahead of prefetches, and don't wait for them."""
        rows, row, longest, has_waiting = [], [], 0, False
        for i, (f, w) in enumerate(zip(frames, waiting, strict=True)):
            if row and ((len(row) + 1) * max(longest, f) > self.frame_budget or (has_waiting and not w)):
                rows.append(row)
                row, longest, has_waiting = [], 0, False
            row.append(i)
            longest = max(longest, f)
            has_waiting |= w
        if row:
            rows.append(row)
        return rows

    @torch.inference_mode()
    def run(self, requests: list[Request]) -> Iterator[list[Result]]:
        """Synthesize a batch; yields results row by row as each finishes, most urgent first.

        `requests` must already be validated, and listed most urgent first (as the queue orders
        them: priority, then waiting before prefetch, then arrival)."""
        dev = self.device
        n = len(requests)
        lengths = torch.tensor([len(r.ids) + 2 for r in requests])
        width = int(lengths.max())
        ids = torch.zeros(n, width, dtype=torch.long)
        for i, r in enumerate(requests):
            ids[i, 1 : len(r.ids) + 1] = torch.tensor(r.ids)
        ids = ids.to(dev, non_blocking=True)
        mask = torch.arange(width, device=dev)[None, :] >= lengths.to(dev)[:, None]
        ref = torch.stack([r.ref_s for r in requests])
        s = ref[:, 128:]
        speed = torch.tensor([r.speed / 100 for r in requests], device=dev)
        max_dur = torch.tensor([[max_duration(r.speed)] for r in requests], device=dev)

        with torch.autocast(dev.type, dtype=torch.float16, enabled=self.half):
            pred_dur, d, t_en = self._durations(ids, mask, s, speed, max_dur)

        pred_cpu = pred_dur.cpu()
        frames = pred_cpu.sum(1).tolist()
        durations = [pred_cpu[i, : lengths[i]].tolist() for i in range(n)]

        # The cap is checked before the decoder runs, so an over-long request costs only its text side.
        fits = [f * SAMPLES_PER_FRAME <= self.max_chunk_samples for f in frames]
        ok = [i for i in range(n) if fits[i]]
        too_long = [i for i in range(n) if not fits[i]]
        if too_long:
            yield [Result(i, None, None, "too_long") for i in too_long]
        if not ok:
            return

        d_t = F.pad(d.transpose(1, 2), (0, 1))  # [B, 640, T + 1]; column T is zeros, for padding
        t_en = F.pad(t_en, (0, 1))
        cum = pred_dur.cumsum(1)
        for row in self._rows([frames[i] for i in ok], [requests[i].waiting for i in ok]):
            idx = [ok[r] for r in row]
            yield from self._decode_contained(idx, [frames[i] for i in idx], durations, cum, d_t, t_en, ref)

    def _decode_contained(self, idx, frames, *args) -> Iterator[list[Result]]:
        """A batch couples its sentences: a non-finite value in one can spread to the others (through
        the matmuls' shared rows), and an out-of-memory error fails them all. Then each is decoded alone, so only the one at fault
        fails, and later rows may still fit."""
        try:
            results = self._decode(idx, frames, *args)
            if len(idx) == 1 or all(r.pcm is not None for r in results):
                yield results
                return
            log.warning("batch of %d had non-finite audio; decoding them one at a time", len(idx))
        except torch.OutOfMemoryError:
            oom = True
        else:
            oom = False
        if oom:
            torch.cuda.empty_cache()  # outside the except, so the traceback no longer holds the row
            if len(idx) == 1:
                yield [Result(idx[0], None, None, "out_of_memory")]
                return
            log.warning("batch of %d (%d frames) ran out of memory; decoding them one at a time",
                        len(idx), sum(frames))
        for i, f in zip(idx, frames):
            yield from self._decode_contained([i], [f], *args)

    def _decode(self, idx, frames, durations, cum, d_t, t_en, ref) -> list[Result]:
        dev = self.device
        sel = torch.tensor(idx, device=dev)
        width = max(frames)
        positions = torch.arange(width, device=dev).expand(len(idx), -1).contiguous()
        # token spoken at each frame; T (the zero column) past an item's end
        token = torch.searchsorted(cum[sel], positions, right=True)
        en = torch.gather(d_t[sel], 2, token[:, None, :].expand(-1, d_t.shape[1], -1))
        asr = torch.gather(t_en[sel], 2, token[:, None, :].expand(-1, t_en.shape[1], -1))
        s = ref[sel]
        lengths = torch.tensor(frames, device=dev)
        m_f = (positions < lengths[:, None])[:, None, :]  # [n, 1, width]
        with torch.autocast(dev.type, dtype=torch.float16, enabled=self.half):
            fn = self._fn
            keep = m_f.transpose(1, 2).to(en.dtype)
            x = fn["shared"](en.transpose(1, 2), keep) * keep  # [n, width, 512]
            m_f = m_f.to(x.dtype)
            f0, nn = fn["f0n"](x.transpose(1, 2), s[:, 128:], m_f, m_f.repeat_interleave(2, dim=2))
            audio = fn["decoder"](asr, f0, nn, s[:, :128], lengths)  # [n, 1, SAMPLES_PER_FRAME * width]
        rows = [audio[i, 0, : f * SAMPLES_PER_FRAME] for i, f in enumerate(frames)]
        audio = torch.cat(rows).float()
        bounds = [0]
        for f in frames:
            bounds.append(bounds[-1] + f * SAMPLES_PER_FRAME)
        finite = [bool(v) for v in torch.stack([r.isfinite().all() for r in rows]).cpu()]
        pcm = (audio * 32767).clamp(-32768, 32767).to(torch.int16).cpu().numpy()
        results = []
        for row, i in enumerate(idx):
            if not finite[row]:
                results.append(Result(i, None, None, "non_finite"))
                continue
            results.append(Result(i, pcm[bounds[row] : bounds[row + 1]].astype("<i2").tobytes(), durations[i]))
        return results
