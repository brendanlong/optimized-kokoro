"""One-time conversion of hexgrad/Kokoro-82M's pickled weights to safetensors.

The worker never unpickles anything: this runs once, outside the sandbox, and writes
    model.safetensors   the KModel state dict with weight norm already folded; the LSTMs'
                        weights replaced by the distilled ConvSeqs' (convseq.py) from --students,
                        the F0/N predictor's by the batch-first ones from --frame-side, and the
                        decoder's by the Vocos-style decoder's (frame.Decoder) from --decoder
    voices.safetensors  one [510, 256] fp32 tensor per voice
    config.json         copied unchanged
into the output directory, which the worker then mounts read-only.

    uv run python -m kokoro_worker.convert OUT_DIR --students STUDENTS_PT --frame-side FRAME_SIDE_PT \
        --decoder DECODER_PT [--source DIR]

The checkpoints come from brendanlong/kokoro's experiments (branch brendanlong/batch-first-decoder):
STUDENTS_PT from lstm_swap/train_te.py, FRAME_SIDE_PT from batch_first/train.py (only its F0/N
predictor is used) and DECODER_PT from vocos/train_adv.py.

With no --source, the files come from the Hugging Face hub at a pinned revision.
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
VOICE_NAME = re.compile(r"^[a-z0-9_]{1,64}$")


def fetch(source: str | None) -> str:
    if source:
        return source
    from huggingface_hub import snapshot_download

    return snapshot_download(
        REPO_ID, revision=REVISION, allow_patterns=["config.json", "kokoro-v1_0.pth", "voices/*.pt"]
    )


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
    want = {"dim": frame.DIM, "layers": frame.LAYERS, "mult": frame.MULT, "head": "spec", "full_band": False}
    config = {**want, **ckpt.get("config", {})}
    if config != want:
        raise SystemExit(f"{path}: decoder config {config}; frame.Decoder is {want}")
    state = ckpt["state"]
    return {"decoder." + k: v.float().contiguous() for k, v in state.items()}


def convert(source: str, out: str, students: str, frame_side: str, decoder: str) -> None:
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
    state.update(student_state(students))
    state.update(frame_side_state(frame_side))
    state.update(decoder_state(decoder))
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
    parser.add_argument("--source", help="directory with config.json, kokoro-v1_0.pth and voices/*.pt")
    parser.add_argument("--students", required=True, help="the distilled ConvSeqs (see above)")
    parser.add_argument("--frame-side", required=True, help="the batch-first F0/N predictor (see above)")
    parser.add_argument("--decoder", required=True, help="the Vocos-style decoder (see above)")
    args = parser.parse_args()
    convert(fetch(args.source), args.out, args.students, args.frame_side, args.decoder)


if __name__ == "__main__":
    main()
