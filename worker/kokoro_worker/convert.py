"""One-time conversion of hexgrad/Kokoro-82M's pickled weights to safetensors.

The worker never unpickles anything: this runs once, outside the sandbox, and writes
    model.safetensors   the KModel state dict with weight norm already folded
    voices.safetensors  one [510, 256] fp32 tensor per voice
    config.json         copied unchanged
into the output directory, which the worker then mounts read-only.

    uv run python -m kokoro_worker.convert OUT_DIR [--source DIR]

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


def convert(source: str, out: str) -> None:
    from kokoro_worker.model import _import_kokoro, build_kmodel

    os.makedirs(out, exist_ok=True)
    config_path = os.path.join(source, "config.json")
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    # KModel's own loader (torch.load with weights_only=True) reads the original checkpoint and
    # folds weight norm; we save the result, so the worker loads it with no folding step either.
    KModel = _import_kokoro().KModel  # kokoro's model code without its G2P
    model = KModel(repo_id=REPO_ID, config=config, model=os.path.join(source, "kokoro-v1_0.pth")).eval()
    state = {k: v.detach().contiguous().clone() for k, v in model.state_dict().items()}
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
    args = parser.parse_args()
    convert(fetch(args.source), args.out)


if __name__ == "__main__":
    main()
