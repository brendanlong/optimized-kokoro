"""The answer key: Tolkien Gateway's IPA, curated by config.yaml and converted to misaki phonemes."""

import json
import sys

import yaml

from phon import ipa_to_misaki


def main(raw, config, out):
    cfg = yaml.safe_load(open(config))
    with open(out, "w") as f:
        for line in open(raw):
            r = json.loads(line)
            title = r["page"]
            if "/" in title or r["name"] in cfg["exclude"] or title in cfg["exclude"]:
                continue
            name = cfg["rename"].get(title, r["name"])
            ipa = cfg["ipa_override"].get(title, r["ipa"])
            ipa = " ".join(ipa.split()[: len(name.split())])
            f.write(json.dumps({"name": name, "ipa": ipa, "phonemes": ipa_to_misaki(ipa)}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
