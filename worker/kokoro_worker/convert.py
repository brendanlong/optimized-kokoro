"""One-time conversion of Órë TTS's weights to safetensors for the worker.

Órë TTS is Kokoro-82M distilled for batching (brendanlong/ore-tts on Hugging Face, Apache-2.0):
Kokoro's own weights where they're kept, plus distilled replacements for its LSTMs, its F0/N
predictor and its decoder. The worker never unpickles anything: this runs once, outside the
sandbox, and writes
    model.safetensors   Kokoro's KModel state dict with weight norm folded, and the LSTMs',
                        the F0/N predictor's and the decoder's weights replaced by Órë TTS's
                        (convseq.py, frame.f0n and frame.Decoder)
    voices.safetensors  one [510, 256] fp32 tensor per voice (Kokoro's)
    config.json         Kokoro's, copied unchanged
into the output directory, which the worker then mounts read-only. deploy/ names a conversion by
this file's hash (models_id in deploy/lib.sh), so a change to what it writes, or to how the worker
loads it, must change this file too (a pinned revision, or this note's date: 2026-11-28).

    uv run python -m kokoro_worker.convert OUT_DIR [--source DIR] [--ore DIR]

With no --source and --ore, both come from the Hugging Face hub at pinned revisions. --source is a
directory with Kokoro's config.json, kokoro-v1_0.pth and voices/*.pt; --ore one laid out like the
brendanlong/ore-tts repo (ORE_FILES), for checkpoints not yet published. They're made by
brendanlong/kokoro's experiments (branch brendanlong/batch-first-decoder): the LSTM replacements by
lstm_swap/train_te.py, the predictor by batch_first/train.py and the decoder by vocos/train.py or
train_adv.py, with frame.CONFIG's configuration.
"""

import argparse
import json
import os
import re
import shutil

import torch
from safetensors.torch import save_file

REPO_ID = "hexgrad/Kokoro-82M"
REVISION = "f3ff3571791e39611d31c381e3a41a3af07b4987"
ORE_REPO_ID = "brendanlong/ore-tts"
ORE_REVISION = "bc05253abcc2889fc84bdcdebcb1e644389dcf38"  # the commit tag v0.1 points to
ORE_FILES = {
    "students": "front/lstm-replacements.pt",
    "frame_side": "front/pitch-energy-predictor.pt",
    "decoder": "decoder/original-front-512.pt",
}
VOICE_NAME = re.compile(r"^[a-z0-9_]{1,64}$")


def fetch(source: str | None) -> str:
    if source:
        return source
    from huggingface_hub import snapshot_download

    return snapshot_download(
        REPO_ID, revision=REVISION, allow_patterns=["config.json", "kokoro-v1_0.pth", "voices/*.pt"]
    )


def fetch_ore(ore: str | None) -> dict:
    """ORE_FILES' paths, from --ore's directory or the hub at ORE_REVISION."""
    if not ore:
        from huggingface_hub import snapshot_download

        ore = snapshot_download(ORE_REPO_ID, revision=ORE_REVISION, allow_patterns=list(ORE_FILES.values()))
    return {k: os.path.join(ore, f) for k, f in ORE_FILES.items()}


def student_state(path: str) -> dict:
    """The ConvSeqs' weights from an lstm_swap checkpoint, keyed as in the worker's KModel."""
    from kokoro_worker import convseq

    ckpt = torch.load(path, map_location="cpu", weights_only=True)
    default = {"n_blocks": convseq.BLOCKS, "kernel": convseq.KERNEL, "mult": convseq.MULT}
    for p in convseq.PATHS:
        arch = {**default, **ckpt["arch"].get(p, {})}
        if arch != default:
            raise SystemExit(f"{path}: {p} has arch {arch}; convseq.py builds {default}")
    state = ckpt["state"]
    missing = [p for p in convseq.PATHS if not any(k.startswith(p + ".") for k in state)]
    if missing:
        raise SystemExit(f"{path}: no weights for {missing}")
    return {k: v.float().contiguous() for k, v in state.items()}


F0N = ("predictor.F0.", "predictor.N.", "predictor.F0_proj.", "predictor.N_proj.")


def _fold_weight_norm(state: dict) -> dict:
    """torch's weight_norm parametrization (dim 0) folded into plain weights, as KModel does."""
    out = {}
    for k, v in state.items():
        if k.endswith(".parametrizations.weight.original0"):
            base = k[: -len(".parametrizations.weight.original0")]
            g, w = v, state[base + ".parametrizations.weight.original1"]
            out[base + ".weight"] = g * w / w.norm(dim=tuple(range(1, w.dim())), keepdim=True)
        elif not k.endswith(".parametrizations.weight.original1"):
            out[k] = v
    return out


def frame_side_state(path: str) -> dict:
    """The batch-first F0/N predictor's weights, keyed as in the worker's KModel."""
    state = _fold_weight_norm(torch.load(path, map_location="cpu", weights_only=True)["state"])
    return {k: v.float().contiguous() for k, v in state.items() if k.startswith(F0N)}


def decoder_state(path: str) -> dict:
    from kokoro_worker import frame

    ckpt = torch.load(path, map_location="cpu", weights_only=True)
    # Other heads can have the same parameter shapes, so a strict load wouldn't catch them.
    want = frame.CONFIG
    config = {**want, **ckpt.get("config", {})}
    if config != want:
        raise SystemExit(f"{path}: decoder config {config}; frame.Decoder is {want}")
    state = ckpt["state"]
    return {"decoder." + k: v.float().contiguous() for k, v in state.items()}


def convert(source: str, out: str, ore: dict) -> None:
    from kokoro_worker import convseq
    from kokoro_worker.model import _import_kokoro, build_kmodel

    os.makedirs(out, exist_ok=True)
    config_path = os.path.join(source, "config.json")
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    # KModel's own loader (torch.load with weights_only=True) reads the original checkpoint and
    # folds weight norm; we save the result, so the worker loads it with no folding step either.
    KModel = _import_kokoro().KModel  # kokoro's model code without its G2P
    model = KModel(repo_id=REPO_ID, config=config, model=os.path.join(source, "kokoro-v1_0.pth")).eval()
    replaced = (*(p + "." for p in convseq.PATHS), *F0N, "decoder.")
    state = {k: v.detach().contiguous().clone() for k, v in model.state_dict().items() if not k.startswith(replaced)}
    state.update(student_state(ore["students"]))
    state.update(frame_side_state(ore["frame_side"]))
    state.update(decoder_state(ore["decoder"]))
    build_kmodel(config).load_state_dict(state, strict=True)  # the worker's loader must accept it
    save_file(state, os.path.join(out, "model.safetensors"))

    voices = {}
    voice_dir = os.path.join(source, "voices")
    for name in sorted(os.listdir(voice_dir)):
        if not name.endswith(".pt"):
            continue
        voice = name[:-3]
        if not VOICE_NAME.match(voice):
            raise SystemExit(f"voice name not [a-z0-9_]{{1,64}}: {voice!r}")
        pack = torch.load(os.path.join(voice_dir, name), map_location="cpu", weights_only=True)
        if tuple(pack.shape) != (510, 1, 256):
            raise SystemExit(f"{voice}: unexpected shape {tuple(pack.shape)}")
        voices[voice] = pack[:, 0, :].float().contiguous()
    if not voices:
        raise SystemExit(f"no voices in {voice_dir}")
    save_file(voices, os.path.join(out, "voices.safetensors"))
    shutil.copyfile(config_path, os.path.join(out, "config.json"))
    print(f"wrote {out}: {len(state)} tensors, {len(voices)} voices")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out")
    parser.add_argument("--source", help="directory with Kokoro's config.json, kokoro-v1_0.pth and voices/*.pt")
    parser.add_argument("--ore", help="directory laid out like brendanlong/ore-tts (ORE_FILES)")
    args = parser.parse_args()
    convert(fetch(args.source), args.out, fetch_ore(args.ore))


if __name__ == "__main__":
    main()
