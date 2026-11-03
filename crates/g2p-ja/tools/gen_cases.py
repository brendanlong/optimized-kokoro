"""Generates Japanese test texts for tools/reference.py: numbers, mixed scripts, katakana
loanwords, rare kanji, long vowels, small kana, iteration marks, punctuation, whitespace, long
runs of one character category (MeCab's unknown-word grouping), and random mixtures.

    python3 tools/gen_cases.py SEED N > TEXTS.jsonl
"""

import json
import random
import sys

HIRA = [chr(c) for c in range(0x3041, 0x3097)]
KATA = [chr(c) for c in range(0x30A1, 0x30FB)] + ['ー']
SMALL = list('ぁぃぅぇぉゃゅょゎっァィゥェォャュョヮッヵヶ')
COMMON_KANJI = list('日本語東京大学時代明治江戸年月人生会社国民政府出版新聞電話鉄道駅山川海空雨雪花鳥風月春夏秋冬朝昼夜'
                    '今昔上下左右前後中外内高低長短新古多少早速遠近強弱私彼女子男女父母兄弟姉妹友達先生学校教室')
RARE_KANJI = [chr(c) for c in range(0x3400, 0x4DB6, 97)] + [chr(c) for c in range(0x9F00, 0x9FFF, 13)] + \
    [chr(c) for c in range(0xF900, 0xFA6A, 7)] + [chr(c) for c in range(0x20000, 0x2A6D6, 997)] + list('鬱齟齬魑魅魍魎薔薇檸檬蝋燭躑躅饂飩')
MARKS = list('々ゝゞヽヾ〃')
PUNCT = list('、。，．・：；？！「」『』（）【】《》〈〉〔〕［］｛｝〜～…‥―—－ー‐゛゜゙゚＝＋％＆＠＃＊／＼｜＾＿｀"\'(),.!?:;-~/')
SPACES = [' ', '　', '\t', '  ', ' ', ' ']
LATIN = ['Tokyo', 'ISBN', 'CD', 'DVD', 'iPhone', 'NHK', 'JR', 'x', 'A', 'Ｔｏｋｙｏ', 'ＡＢＣ', 'café', 'naïve']
OTHER = ['Привет', 'Ελλάδα', '안녕하세요', 'สวัสดี', 'مرحبا', '😀', '👍🏻', '©', '®', '™', '°C', '℃', '㎞', '㍿', '①', '❶', '²', '½', 'Ⅳ', '٣٤', '০১', '፩']
HALF_KANA = list('ｱｲｳｴｵｶｷｸｹｺｶﾞｷﾞﾊﾟﾋﾟｬｭｮｯｰﾞﾟ')
EXT = [chr(c) for c in range(0x31F0, 0x3200)]
WORDS = ['ありがとう', 'こんにちは', 'コンピューター', 'ヴァイオリン', 'ウィキペディア', 'ティーカップ', 'ディズニー', 'ファイル',
         'チェック', 'シェア', 'ジェット', 'ツァー', 'フュージョン', 'デュエット', 'トゥモロー', 'クォーター', 'グァテマラ',
         'イェール', 'キェルケゴール', 'ヴュー', 'ヂャ', 'ヷヸヹヺ', 'おかあさん', 'おとうさん', 'ちょっと', 'きっぷ', 'ほんとう',
         'さんぽ', 'しんぶん', 'てんき', 'こんな', 'ぎんこう', 'えいが', 'ゆうびん', 'がっこう', 'いっしょ', 'ーー', 'あーあ', 'すごーい',
         'ねぇ', 'えぇっ', 'んー', 'ん', 'っ', 'ッ', 'ゃ', '時々', '人々', 'いすゞ', 'こゝろ', 'さゞなみ', 'バナナ', 'ミルク']
NUMBER_CONTEXTS = ['{}年', '{}月{}日', '第{}版', '{}円', '{}人', '約{}km', '{}%', '{}〜{}年', '{}～{}', '{}.{}', '{},{}', '{}時{}分', 'ISBN {}', '{}-{}-{}', '{}番']


def number(rng):
    kind = rng.random()
    if kind < 0.3:
        return str(rng.randint(0, 99))
    if kind < 0.6:
        return str(rng.randint(0, 10 ** rng.randint(2, 9)))
    if kind < 0.7:
        return '0' * rng.randint(1, 3) + str(rng.randint(0, 999))
    if kind < 0.8:
        return str(rng.randint(10 ** 9, 10 ** 14))
    if kind < 0.9:
        return ''.join(chr(0xFF10 + int(d)) for d in str(rng.randint(0, 99999)))
    return rng.choice(['0', '10', '100', '1000', '10000', '100000000', '999999999', '1000000000', '20', '300', '600', '800', '3000', '8000', '1001', '10010', '110000'])


def run(rng):
    pool = rng.choice([HIRA, KATA, COMMON_KANJI, RARE_KANJI, list('abcXYZ'), list('ＡＢＣ'), list('αβγ'), list('абв'), PUNCT, ['ー'], SMALL, list('0123')])
    n = rng.choice([1, 2, 3, 20, 23, 24, 25, 26, 30, 50])
    if rng.random() < 0.5:
        return rng.choice(pool) * n
    return ''.join(rng.choice(pool) for _ in range(n))


def piece(rng):
    r = rng.random()
    if r < 0.15:
        ctx = rng.choice(NUMBER_CONTEXTS)
        return ctx.format(*[number(rng) for _ in range(ctx.count('{}'))])
    if r < 0.30:
        return rng.choice(WORDS)
    if r < 0.40:
        return ''.join(rng.choice(HIRA) for _ in range(rng.randint(1, 6)))
    if r < 0.50:
        return ''.join(rng.choice(KATA + SMALL) for _ in range(rng.randint(1, 8)))
    if r < 0.58:
        return ''.join(rng.choice(COMMON_KANJI) for _ in range(rng.randint(1, 4)))
    if r < 0.63:
        return ''.join(rng.choice(RARE_KANJI) for _ in range(rng.randint(1, 3)))
    if r < 0.70:
        return rng.choice(PUNCT)
    if r < 0.74:
        return rng.choice(SPACES)
    if r < 0.79:
        return rng.choice(LATIN)
    if r < 0.83:
        return rng.choice(OTHER)
    if r < 0.86:
        return rng.choice(MARKS)
    if r < 0.89:
        return ''.join(rng.choice(HALF_KANA) for _ in range(rng.randint(1, 5)))
    if r < 0.91:
        return rng.choice(EXT)
    if r < 0.96:
        return run(rng)
    return ''.join(rng.choice(SMALL + ['ー', 'ん', 'っ']) for _ in range(rng.randint(1, 4)))


def main(seed, n):
    rng = random.Random(seed)
    for i in range(n):
        text = ''.join(piece(rng) for _ in range(rng.randint(1, 12)))
        print(json.dumps({"id": f"gen-{seed}-{i}", "text": text}, ensure_ascii=False))


if __name__ == "__main__":
    main(int(sys.argv[1]), int(sys.argv[2]))
