"""Kokoro-82M with a batched forward pass, loaded from safetensors.

The modules are the kokoro fork's (pinned in pyproject.toml); only the forward pass is ours, so
that a batch can hold sentences of different lengths:

- Text side (ALBERT, text encoder, duration predictor) is exact under padding: ALBERT gets an
  attention mask, padded positions are zeroed before every convolution (as the convolution's own
  zero padding would be), and the LSTMs run on packed sequences. Durations therefore match batch 1
  (exactly in fp32; fp16 numerics depend a little on batch size).
- Frame side (F0/N predictor and decoder) runs in groups of similar frame count, padded to the
  longest in the group. The shared LSTM is packed, and masked.py takes the instance norms'
  statistics over real frames and zeroes padding before every convolution, so padding doesn't
  change the audio (`exact=False` drops the masks). `max_batch=1` in the server turns batching off.
"""

import importlib.util
import json
import re
import os
import sys
import types
from dataclasses import dataclass
from typing import Iterator

import torch
import torch.nn.functional as F
from safetensors.torch import load_file

from . import masked
from .protocol import chunk_cap

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
    speed: float


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
        max_chunk_samples: int = 2_880_000,
        frame_budget: int = 16384,
        pad_slack: float = 0.25,
        exact: bool = True,
    ):
        with open(os.path.join(model_dir, "config.json"), encoding="utf-8") as f:
            config = json.load(f)
        self.vocab_size = int(config["n_token"])
        self.vocab_ids = frozenset(int(i) for i in config["vocab"].values())
        self.max_phonemes = int(config["plbert"]["max_position_embeddings"]) - 2
        self.device = torch.device(device)
        self.half = half and self.device.type == "cuda"
        self.max_chunk_samples = max_chunk_samples
        self.frame_budget = frame_budget
        self.pad_slack = pad_slack
        self.exact = exact

        model = build_kmodel(config)
        model.load_state_dict(load_file(os.path.join(model_dir, "model.safetensors")), strict=True)
        model = model.eval().requires_grad_(False).to(self.device)
        if self.half:
            model = model.half()
            # Autocast runs instance_norm in fp32; fp16 affine weights then fail for batch > 1.
            for module in model.modules():
                if isinstance(module, torch.nn.InstanceNorm1d):
                    module.float()
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
            "f0n_masked": lambda x, s, m: masked.f0n_blocks(model.predictor, x, s, m),
            "decoder_masked": lambda asr, f0, n, s, m: masked.decoder(model.decoder, asr, f0, n, s, m),
        }
        self._single = self._batched = eager
        if compile:
            # Separate graphs for batch 1 and batches > 1: within one torch.compile'd function, the
            # dynamic-batch graph's guards also accept batch 1, so batches of one could run on it
            # instead of on the graph specialized for them.
            self._single = {k: torch.compile(_fresh(f, "single"), dynamic=True) for k, f in eager.items()}
            self._batched = {k: torch.compile(_fresh(f, "batched"), dynamic=True) for k, f in eager.items()}

    def style(self, voices: list[tuple[int, float]], style_len: int) -> torch.Tensor:
        """Weighted mix of voice packs at row style_len - 1 (KPipeline's pack[len(ps) - 1]). Weights sum to 1."""
        total = sum(w for _, w in voices)
        index = torch.tensor([v for v, _ in voices], device=self.device)
        weight = torch.tensor([w / total for _, w in voices], device=self.device, dtype=torch.float32)
        return (self.voices[index, style_len - 1] * weight[:, None]).sum(0)

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

    def _durations(self, ids, mask, lengths, s, speed):
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
        pred_dur = torch.round(duration).clamp(min=1).long().masked_fill(mask, 0)
        return pred_dur, d, t_en

    def _groups(self, frames: list[int], order: list[int]) -> list[list[int]]:
        """Split items into decoder batches of similar frame counts.

        Greedy over items sorted by length: a group grows while its padded size stays within
        pad_slack of its real size and within frame_budget. Groups run in the order of their
        most urgent item (lowest position in `order`)."""
        by_len = sorted(range(len(frames)), key=lambda i: frames[i])
        groups, group = [], []
        for i in by_len:
            candidate = group + [i]
            padded = len(candidate) * frames[i]
            real = sum(frames[j] for j in candidate)
            if group and (padded > (1 + self.pad_slack) * real or padded > self.frame_budget):
                groups.append(group)
                candidate = [i]
            group = candidate
        if group:
            groups.append(group)
        rank = {item: r for r, item in enumerate(order)}
        return sorted(groups, key=lambda g: min(rank[i] for i in g))

    @torch.inference_mode()
    def run(self, requests: list[Request]) -> Iterator[list[Result]]:
        """Synthesize a batch; yields results group by group as each finishes, most urgent first.

        `requests` must already be validated, and listed most urgent first."""
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
        speed = torch.tensor([r.speed for r in requests], device=dev)

        with torch.autocast(dev.type, dtype=torch.float16, enabled=self.half):
            pred_dur, d, t_en = self._durations(ids, mask, lengths, s, speed)
        pred_cpu = pred_dur.cpu()
        frames = pred_cpu.sum(1).tolist()
        durations = [pred_cpu[i, : lengths[i]].tolist() for i in range(n)]

        # The cap is checked before the decoder runs, so an over-long request costs only its text side.
        fits = [frames[i] * SAMPLES_PER_FRAME <= chunk_cap(self.max_chunk_samples, r.speed)
                for i, r in enumerate(requests)]
        ok = [i for i in range(n) if fits[i]]
        too_long = [i for i in range(n) if not fits[i]]
        if too_long:
            yield [Result(i, None, None, "too_long") for i in too_long]
        if not ok:
            return

        d_t = F.pad(d.transpose(1, 2), (0, 1))  # [B, 640, T + 1]; column T is zeros, for padding
        t_en = F.pad(t_en, (0, 1))
        cum = pred_dur.cumsum(1)
        for group in self._groups([frames[i] for i in ok], list(range(len(ok)))):
            idx = [ok[g] for g in group]
            try:
                yield self._decode(idx, [frames[i] for i in idx], durations, cum, d_t, t_en, ref)
            except torch.OutOfMemoryError:
                # Only this group fails; smaller groups after it may still fit.
                torch.cuda.empty_cache()
                yield [Result(i, None, None, "out_of_memory") for i in idx]

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
        with torch.autocast(dev.type, dtype=torch.float16, enabled=self.half):
            x = _packed_lstm(self.model.predictor.shared, en.transpose(1, 2), torch.tensor(frames))
            fn = self._single if len(idx) == 1 else self._batched
            if self.exact and min(frames) != width:
                m = masked.masks_for(frames, width, dev, torch.float16 if self.half else torch.float32)
                f0, n = fn["f0n_masked"](x.transpose(1, 2), s[:, 128:], m)
                audio = fn["decoder_masked"](asr, f0, n, s[:, :128], m)
            else:
                f0, n = fn["f0n"](x.transpose(1, 2), s[:, 128:])
                audio = fn["decoder"](asr, f0, n, s[:, :128])
        audio = audio.reshape(len(idx), -1).float()
        finite = torch.isfinite(audio).all(1)
        pcm = (audio * 32767).clamp(-32768, 32767).to(torch.int16).cpu()
        finite = finite.cpu().tolist()
        results = []
        for row, i in enumerate(idx):
            if not finite[row]:
                results.append(Result(i, None, None, "non_finite"))
                continue
            samples = pcm[row, : frames[row] * SAMPLES_PER_FRAME].numpy()
            results.append(Result(i, samples.astype("<i2").tobytes(), durations[i]))
        return results
