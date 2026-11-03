"""Long inputs for tools/reference.py: MeCab's 65,535-byte lookahead, unknown-word groups at the
24-character limit, and long runs of each character category.

    python3 tools/gen_long.py > TEXTS.jsonl
"""

import json

cases = []
for n in (23, 24, 25, 26, 49, 50, 51):
    for c in ('ア', 'ヸ', 'x', 'X', '7', 'α', 'д', '☆', '𠀋', 'ー', 'ぁ', '鬱'):
        cases.append(c * n)
        cases.append('東京' + c * n + 'へ')
for n in (65534, 65535, 65536, 70000):
    cases.append('あ' + ' ' * n + 'い')
    cases.append('あ' + ' ' * n)
for n in (21844, 21845, 21846, 30000):
    cases.append('東' + '　' * n + '京')
    cases.append('ア' * n)
    cases.append('x' * 3 * n)
cases.append('です。' * 5000)
cases.append('a' * 65534 + 'あ')
cases.append(' ' * 65533 + 'ア' * 10)
cases.append(' ' * 65534 + 'ア' * 10)
cases.append(' ' * 65535 + 'ア' * 10)
for t in cases:
    print(json.dumps({"text": t}, ensure_ascii=False))
