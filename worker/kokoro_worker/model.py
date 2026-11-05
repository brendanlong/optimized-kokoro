"""Kokoro-82M with a batched forward pass, loaded from safetensors.

The modules are the kokoro fork's (pinned in pyproject.toml); only the forward pass is ours, so
that a batch can hold sentences of different lengths:

- Text side (ALBERT, text encoder, duration predictor) is exact under padding: ALBERT gets an
  attention mask, padded positions are zeroed before every convolution (as the convolution's own
  zero padding would be), and the LSTMs run on packed sequences. Durations therefore match batch 1
  (exactly in fp32; fp16 numerics depend a little on batch size).
- Frame side (F0/N predictor and decoder): the shared LSTM runs packed, then the batch's
  sentences go end to end in one row with a gap of zeros between them (masked.py), so the
  convolutions run once over the row whatever the lengths, and each sentence's audio is what it
  would be alone. `max_batch=1` in the server turns batching off.
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

from . import kernels, masked
from .protocol import max_duration

log = logging.getLogger(__name__)

SAMPLES_PER_FRAME = 600  # 2x upsample in the decoder, then 10 * 6 in the generator, then hop 5
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
    """An untrained KModel with weight norm already folded, ready for a converted state dict."""
    kmodel = _import_kokoro()
    from transformers import AlbertConfig

    from kokoro.istftnet import Decoder
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
    m.decoder = Decoder(
        dim_in=config["hidden_dim"],
        style_dim=config["style_dim"],
        dim_out=config["n_mels"],
        disable_complex=False,
        **config["istftnet"],
    )
    m._fold_weight_norm()
    return m


def _fresh(fn, tag: str):
    """A wrapper around fn with its own code object. torch.compile keeps its graphs per code object,
    so wrappers made here get separate caches."""

    def call(*args, **kwargs):
        return fn(*args, **kwargs)

    name = getattr(fn, "__name__", type(fn).__name__)
    call.__code__ = call.__code__.replace(co_name=f"{tag}_{name}")
    return call


def _packed_lstm(lstm: torch.nn.LSTM, x: torch.Tensor, lengths: torch.Tensor) -> torch.Tensor:
    """Run a batch_first LSTM as if each row were alone; padded outputs are zero."""
    if bool((lengths == x.shape[1]).all()):
        return lstm(x)[0]
    packed = torch.nn.utils.rnn.pack_padded_sequence(x, lengths, batch_first=True, enforce_sorted=False)
    out, _ = lstm(packed)
    return torch.nn.utils.rnn.pad_packed_sequence(out, batch_first=True, total_length=x.shape[1])[0]


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
        # A packed row holds at most as many frames as one chunk may have, so a batch never needs
        # more memory than the longest single request.
        self.frame_budget = frame_budget or max_chunk_samples // SAMPLES_PER_FRAME

        model = build_kmodel(config)
        model.load_state_dict(load_file(os.path.join(model_dir, "model.safetensors")), strict=True)
        model = model.eval().requires_grad_(False).to(self.device)
        if self.half:
            model = model.half()
            # Autocast runs instance_norm in fp32; fp16 affine weights then fail for batch > 1.
            for module in model.modules():
                if isinstance(module, torch.nn.InstanceNorm1d):
                    module.float()
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

        eager = {
            "bert": model.bert,
            "text_cnn": self._text_cnn_eager,
            "f0n": self._f0n_blocks_eager,
            "decoder": model.decoder,
            "f0n_packed": lambda x, s, pk: masked.f0n_blocks(model.predictor, x, s, pk),
            "decoder_packed": lambda asr, f0, n, s, pk: masked.decoder(model.decoder, asr, f0, n, s, pk),
        }
        single = ("bert", "text_cnn", "f0n", "decoder")
        batched = ("bert", "text_cnn", "f0n_packed", "decoder_packed")
        self._single = {k: eager[k] for k in single}
        self._batched = {k: eager[k] for k in batched}
        if compile:
            # Separate graphs for batch 1 and batches > 1: within one torch.compile'd function, the
            # dynamic-batch graph's guards also accept batch 1, so batches of one could run on it
            # instead of on the graph specialized for them.
            self._single = {k: torch.compile(_fresh(eager[k], "single"), dynamic=True) for k in single}
            self._batched = {k: torch.compile(_fresh(eager[k], "batched"), dynamic=True) for k in batched}

    def style(self, voice: int, style_len: int) -> torch.Tensor:
        """Voice pack `voice` at row style_len - 1 (KPipeline's pack[len(ps) - 1])."""
        return self.voices[voice, style_len - 1]

    def _text_cnn_eager(self, ids: torch.Tensor, mask: torch.Tensor) -> torch.Tensor:
        te = self.model.text_encoder
        keep = ~mask[:, None, :]
        x = te.embedding(ids).transpose(1, 2) * keep
        for block in te.cnn:
            x = block(x) * keep
        return x.transpose(1, 2)  # [B, T, C]

    def _f0n_blocks_eager(self, x: torch.Tensor, s: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        p = self.model.predictor
        f0 = x
        for block in p.F0:
            f0 = block(f0, s)
        n = x
        for block in p.N:
            n = block(n, s)
        return p.F0_proj(f0).squeeze(1), p.N_proj(n).squeeze(1)

    def _durations(self, ids, mask, lengths, s, speed, max_dur):
        """Text side for the whole batch: (pred_dur [B, T] with 0 at padding, d [B, T, 640], t_en [B, 512, T])."""
        m = self.model
        keep = (~mask)[..., None]
        fn = self._single if ids.shape[0] == 1 else self._batched
        bert = fn["bert"](ids, attention_mask=keep[..., 0].long())
        d_en = m.bert_encoder(bert)  # [B, T, 512]
        t_en = _packed_lstm(m.text_encoder.lstm, fn["text_cnn"](ids, mask), lengths).transpose(1, 2)

        enc = m.predictor.text_encoder
        s_t = s[:, None, :].expand(-1, ids.shape[1], -1)
        x = torch.cat([d_en, s_t], dim=-1) * keep
        for block in enc.lstms:
            if isinstance(block, torch.nn.LSTM):
                x = _packed_lstm(block, x, lengths)
            else:
                x = torch.cat([block(x, s), s_t], dim=-1) * keep
        d = x  # [B, T, 640]
        x = _packed_lstm(m.predictor.lstm, d, lengths)
        duration = torch.sigmoid(m.predictor.duration_proj(x)).sum(-1) / speed[:, None]
        # The clamp to ceil(50 / speed) never binds (the sum is at most 50); it makes the bound the
        # front checks hold whatever the float rounding.
        pred_dur = torch.round(duration).long().clamp(min=1).minimum(max_dur).masked_fill(mask, 0)
        return pred_dur, d, t_en

    def _groups(self, frames: list[int], waiting: list[bool]) -> list[list[int]]:
        """Split items, most urgent first, into packed rows of at most frame_budget frames.

        A row that holds a chunk a listener is waiting for takes no prefetch after it, so waiting
        chunks decode together, ahead of prefetches, and don't wait for them."""
        groups, group, size, has_waiting = [], [], 0, False
        for i, (f, w) in enumerate(zip(frames, waiting, strict=True)):
            if group and (size + f + masked.GAP > self.frame_budget or (has_waiting and not w)):
                groups.append(group)
                group, size, has_waiting = [], 0, False
            group.append(i)
            size += f + masked.GAP
            has_waiting |= w
        if group:
            groups.append(group)
        return groups

    @torch.inference_mode()
    def run(self, requests: list[Request]) -> Iterator[list[Result]]:
        """Synthesize a batch; yields results group by group as each finishes, most urgent first.

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
            pred_dur, d, t_en = self._durations(ids, mask, lengths, s, speed, max_dur)

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
        for group in self._groups([frames[i] for i in ok], [requests[i].waiting for i in ok]):
            idx = [ok[g] for g in group]
            yield from self._decode_contained(idx, [frames[i] for i in idx], durations, cum, d_t, t_en, ref)

    def _decode_contained(self, idx, frames, *args) -> Iterator[list[Result]]:
        """A packed row couples its sentences: a non-finite value in one spreads to the others, and
        an out-of-memory error fails them all. Then each is decoded alone, so only the one at fault
        fails, and later rows may still fit."""
        try:
            results = self._decode(idx, frames, *args)
            if len(idx) == 1 or all(r.pcm is not None for r in results):
                yield results
                return
            log.warning("packed row of %d had non-finite audio; decoding them one at a time", len(idx))
        except torch.OutOfMemoryError:
            oom = True
        else:
            oom = False
        if oom:
            torch.cuda.empty_cache()  # outside the except, so the traceback no longer holds the row
            if len(idx) == 1:
                yield [Result(idx[0], None, None, "out_of_memory")]
                return
            log.warning("packed row of %d (%d frames) ran out of memory; decoding them one at a time",
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
        n = len(idx)
        with torch.autocast(dev.type, dtype=torch.float16, enabled=self.half):
            x = _packed_lstm(self.model.predictor.shared, en.transpose(1, 2), torch.tensor(frames))
            if n == 1:
                fn = self._single
                f0, nn = fn["f0n"](x.transpose(1, 2), s[:, 128:])
                rows = [fn["decoder"](asr, f0, nn, s[:, :128]).reshape(-1)]
            else:
                fn = self._batched
                pk = masked.Packing(frames, dev, torch.float16 if self.half else torch.float32)
                f0, nn = fn["f0n_packed"](pk.pack(x.transpose(1, 2), width), s[:, 128:], pk.tensors)
                audio = fn["decoder_packed"](pk.pack(asr, width), f0, nn, s[:, :128], pk.tensors)
                rows = pk.unpack_audio(audio)
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
