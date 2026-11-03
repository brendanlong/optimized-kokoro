"""Writes every code point's Python string behaviour as JSON lines, for the `every_char_matches_python` test:
    reference/.cache/kfa/.venv/bin/python -I crates/g2p-en/tools/gen_chars.py /tmp/chars.jsonl
    PY_CHARS_JSONL=/tmp/chars.jsonl cargo test -p kokoro-g2p-en every_char
"""

import json
import sys
import unicodedata

with open(sys.argv[1], "w") as out:
    for cp in range(0x110000):
        if 0xD800 <= cp < 0xE000:
            continue
        c = chr(cp)
        digit = int(unicodedata.numeric(c)) if c.isdigit() else None
        row = [c, c.lower(), c.upper(), c.isalpha(), c.isspace(), digit, unicodedata.normalize("NFKC", c)]
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
