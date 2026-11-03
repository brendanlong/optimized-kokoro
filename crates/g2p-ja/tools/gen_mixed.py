"""Mixed English and Japanese records for reference/dump.py: English sentences with Japanese
runs (lang a and b), and Japanese text with Latin words, numbers and other scripts (lang j).

    python3 tools/gen_mixed.py SEED N ENGLISH.jsonl JAPANESE.jsonl > CORPUS.jsonl

ENGLISH.jsonl and JAPANESE.jsonl are corpus files ({"text": ...} lines), e.g.
reference/corpus/gutenberg.jsonl and reference/corpus/wiki-ja.jsonl.
"""

import json
import random
import re
import sys

JA_WORDS = ['東京', 'こんにちは', 'ありがとう', 'カタカナ', 'ひらがな', '日本語', 'さようなら', 'ラーメン', 'すし', '侍',
            '漢字かな', 'ポケモン', 'ドラえもん', '新幹線', '富士山', 'お茶', 'ちょっと待って', 'ヴァイオリン', 'ウィキペディア',
            'スーパーマーケット', '東京タワー', '第2版', '2001年', '一緒に', 'すごーい', '時々', 'いすゞ', '「こんにちは」', '（にほん）']
LATIN = ['Tokyo', 'NHK', 'iPhone', 'Sony', 'Hello', 'OK', 'DVD', 'the end', "Rock'n'roll", 'e-mail', 'Mt. Fuji', 'café']
OTHER = ['Привет', 'Ελλάδα', '안녕', '😀', 'สวัสดี', 'مرحبا']


def sentences(path, limit):
    out = []
    with open(path) as f:
        for line in f:
            text = json.loads(line)["text"]
            out.extend(s for s in re.split(r'(?<=[.!?。！？])\s*', text) if 4 <= len(s) <= 300)
            if len(out) > limit:
                break
    return out


KANA = re.compile('[\u3041-\u309f\u30a0-\u30ff]')


def japanese_phrase(rng, ja):
    if rng.random() < 0.5:
        return rng.choice(JA_WORDS)
    # Mostly with kana: a Han run without kana is Chinese, which has no G2P here yet.
    while True:
        s = rng.choice(ja)
        a = rng.randrange(len(s))
        phrase = s[a:a + rng.randint(2, 30)]
        if KANA.search(phrase) or rng.random() < 0.2:
            return phrase


def english_record(rng, en, ja):
    words = rng.choice(en).split(' ')
    for _ in range(rng.randint(1, 3)):
        i = rng.randrange(len(words) + 1)
        phrase = japanese_phrase(rng, ja)
        r = rng.random()
        if r < 0.15:
            phrase = f'({phrase})'
        elif r < 0.25:
            phrase = f'“{phrase}”'
        elif r < 0.35 and i > 0:
            words[i - 1] += phrase
            continue
        words.insert(i, phrase)
    return ' '.join(words)


def japanese_record(rng, en, ja):
    s = rng.choice(ja)
    for _ in range(rng.randint(0, 3)):
        i = rng.randrange(len(s) + 1)
        r = rng.random()
        if r < 0.4:
            ins = rng.choice(LATIN)
        elif r < 0.55:
            ins = ' ' + rng.choice(en)[:rng.randint(5, 60)] + ' '
        elif r < 0.75:
            ins = str(rng.choice([7, 42, 100, 2024, 12345, 3.14, 1000000, 9784883191789]))
        elif r < 0.85:
            ins = rng.choice(OTHER)
        else:
            ins = rng.choice(['。', '、', '！', '？', ' ', '　', '…', '「', '」'])
        s = s[:i] + ins + s[i:]
    return s


def main(seed, n, en_path, ja_path):
    rng = random.Random(seed)
    en, ja = sentences(en_path, 20000), sentences(ja_path, 20000)
    for i in range(n):
        if i % 2 == 0:
            lang = 'a' if rng.random() < 0.8 else 'b'
            text = english_record(rng, en, ja)
        else:
            lang, text = 'j', japanese_record(rng, en, ja)
        print(json.dumps({"id": f"mixed-{seed}-{i}", "lang": lang, "text": text}, ensure_ascii=False))


if __name__ == "__main__":
    main(int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4])
