"""A differentiable Kokoro forward pass for fitting voice pack rows: fp32, eager, batch 1, weights frozen.

The same modules as kokoro_worker.model (plain torch convolutions, not kernels.py's), assembled
the way KModel.forward_with_tokens does, but split so a caller can impose durations (teacher
forcing) and get gradients with respect to the style vector. The generator's excitation noise is
off (torch.randn_like -> zeros), so a render is a deterministic function of ids, style and durations.
"""

import json
import os
import sys

import torch
import torch.nn.functional as F
from safetensors.torch import load_file

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kokoro_worker.model import SAMPLES_PER_FRAME, build_kmodel  # noqa: E402

SPACE = 16  # the vocab's id for " "


def noise_off():
    torch.randn_like = torch.zeros_like  # SineGen's excitation noise


def load(models: str, device: str):
    """(model, vocab, voices {name: [510, 256]}) with every weight frozen."""
    with open(os.path.join(models, "config.json"), encoding="utf-8") as f:
        config = json.load(f)
    model = build_kmodel(config)
    model.load_state_dict(load_file(os.path.join(models, "model.safetensors")), strict=True)
    model = model.eval().requires_grad_(False).to(device)
    # cuDNN only runs an LSTM's backward in training mode; these LSTMs have one layer, so
    # training mode changes nothing else (their dropout applies between layers).
    for module in model.modules():
        if isinstance(module, torch.nn.LSTM):
            module.train()
    assert config["vocab"][" "] == SPACE
    voices = {k: v.to(device) for k, v in load_file(os.path.join(models, "voices.safetensors")).items()}
    return model, config["vocab"], voices


def text_side(m, ids: torch.Tensor, ref: torch.Tensor):
    """ids [1, T] with the boundary 0s, ref [1, 256] -> (durations [T] before rounding, d, t_en).

    Only the duration predictor depends on the style, so the rest runs without a graph."""
    with torch.no_grad():
        t_en = m.text_encoder(ids)  # [1, 512, T]
        d_en = m.bert_encoder(m.bert(ids)).transpose(-1, -2)
    s = ref[:, 128:]
    d = m.predictor.text_encoder(d_en, s)  # [1, T, 640]
    x, _ = m.predictor.lstm(d)
    duration = torch.sigmoid(m.predictor.duration_proj(x)).sum(-1)[0]  # speed 1
    return duration, d, t_en


def rounded(duration: torch.Tensor) -> torch.Tensor:
    return torch.round(duration.detach()).clamp(min=1).long()


def decode(m, d, t_en, ref, durations: torch.Tensor):
    """Frame side with the given integer durations [T] -> (audio [600 * frames], F0 [2F], N [2F])."""
    idx = torch.repeat_interleave(torch.arange(len(durations), device=durations.device), durations)
    en = d.transpose(-1, -2)[..., idx]
    f0, n = m.predictor.F0Ntrain(en, ref[:, 128:])
    audio = m.decoder(t_en[..., idx], f0, n, ref[:, :128]).reshape(-1)
    return audio, f0[0], n[0]


def render(m, ids: list[int], ref: torch.Tensor, durations: torch.Tensor | None = None):
    """Phoneme ids without the boundary 0s -> (audio, float durations, int durations, F0, N)."""
    dev = ref.device
    x = torch.tensor([[0, *ids, 0]], device=dev)
    dur, d, t_en = text_side(m, x, ref[None])
    if durations is None:
        durations = rounded(dur)
    audio, f0, n = decode(m, d, t_en, ref[None], durations)
    return audio, dur, durations, f0, n


RESOLUTIONS = [(256, 64), (512, 128), (1024, 256), (2048, 512)]


def stft_loss(x: torch.Tensor, y: torch.Tensor) -> torch.Tensor:
    """Multi-resolution L1 of log STFT magnitudes, averaged over resolutions.

    No spectral convergence term: ||Y - X|| / ||Y|| is smallest for an X scaled below Y wherever
    the two aren't perfectly correlated, and with it the fitted rows came out ~1.4 dB quieter
    than their target."""
    total = 0.0
    for n_fft, hop in RESOLUTIONS:
        win = torch.hann_window(n_fft, device=x.device)
        X = torch.stft(x, n_fft, hop, window=win, return_complex=True).abs().clamp(min=1e-5)
        Y = torch.stft(y, n_fft, hop, window=win, return_complex=True).abs().clamp(min=1e-5)
        total = total + (X.log() - Y.log()).abs().mean()
    return total / len(RESOLUTIONS)


def level_loss(x: torch.Tensor, y: torch.Tensor, floor_db: float = -50.0) -> torch.Tensor:
    """L1 between the RMS envelopes in dB (/10) over 25 ms frames where the target is above floor_db."""
    n = len(y) // SAMPLES_PER_FRAME
    ex = 10 * torch.log10(x[: n * SAMPLES_PER_FRAME].reshape(n, -1).pow(2).mean(1) + 1e-10)
    ey = 10 * torch.log10(y[: n * SAMPLES_PER_FRAME].reshape(n, -1).pow(2).mean(1) + 1e-10)
    loud = ey > floor_db
    if not bool(loud.any()):
        return x.new_zeros(())
    return (ex - ey)[loud].abs().mean() / 10


@torch.no_grad()
def teacher(m, voice: torch.Tensor, phrase: list[int], filler: list[int]) -> dict:
    """The phrase rendered before a filler sentence, with the row for the whole input's length,
    cut after the phrase's last id (its final punctuation): what the phrase sounds like in context."""
    ids = phrase + [SPACE] + filler
    audio, dur, durs, f0, n = render(m, ids, voice[len(ids) - 1])
    keep = len(phrase) + 1  # the leading boundary, then the phrase's ids
    frames = int(durs[:keep].sum())
    return {
        "dur": dur[:keep].float().cpu(),
        "durs": durs[:keep].cpu(),
        "audio": audio[: frames * SAMPLES_PER_FRAME].half().cpu(),
        "f0": f0[: 2 * frames].cpu(),
        "n": n[: 2 * frames].cpu(),
    }


def losses(m, ref: torch.Tensor, phrase: list[int], t: dict) -> dict:
    """The phrase alone with style ref [256] against teacher targets t; aligned with the teacher's
    durations. The trailing boundary keeps the student's own duration and its frames aren't scored."""
    dev = ref.device
    x = torch.tensor([[0, *phrase, 0]], device=dev)
    dur, d, t_en = text_side(m, x, ref[None])
    keep = len(phrase) + 1
    tdurs = t["durs"].to(dev)
    durations = torch.cat([tdurs, rounded(dur[keep:])])
    audio, f0, n = decode(m, d, t_en, ref[None], durations)
    frames = int(tdurs.sum())
    target = t["audio"].to(dev).float()
    head = audio[: frames * SAMPLES_PER_FRAME]
    return {
        "dur": (dur[:keep] - t["dur"].to(dev)).abs().mean(),
        "stft": stft_loss(head, target),
        "level": level_loss(head, target),
        "f0": (f0[: 2 * frames] - t["f0"].to(dev)).abs().mean() / 50,
        "n": (n[: 2 * frames] - t["n"].to(dev)).abs().mean(),
    }
