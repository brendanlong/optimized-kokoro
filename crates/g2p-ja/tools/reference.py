"""Records the reference Japanese G2P (misaki 0.9.4's JAG2P on fugashi + UniDic 3.1.0) for the
parity tests.

Run with the reference venv's Python, from anywhere:
    PY=../../reference/.cache/kfa/.venv/bin/python
    $PY -I tools/reference.py mecab  TEXTS.jsonl > OUT.jsonl   # MeCab's words for each text
    $PY -I tools/reference.py jag2p  TEXTS.jsonl > OUT.jsonl   # JAG2P()(text)[0], or the error
    $PY -I tools/reference.py texts  CORPUS.jsonl... > TEXTS.jsonl  # texts from corpus records

TEXTS.jsonl lines are {"text": ...} (other keys are kept). `mecab` writes
{"text", "morphs": [[surface, reading, char_type, unknown]]}, where reading is cutlet's
`kata2hira(pron or kana or surface)`, char_type MeCab's and unknown whether the word was unknown.
"""

import json
import sys


def texts(paths):
    for path in paths:
        with open(path) as f:
            for line in f:
                if line.strip():
                    r = json.loads(line)
                    yield {"text": r["text"]}


def with_divergences(g2p):
    """Patches JAG2P with the port's documented fixes (docs/divergences.md), for `fixed`. Numbers
    misaki can't read (more than nine digits, digits other than ASCII's, digit surfaces) are read
    by `number_words` straight into phonemes, which MeCab never sees: up to nine digits as
    `Convert` reads them, up to sixteen without a leading zero in units of 万/億/兆, otherwise digit
    by digit. A kana with no reading before a small kana is skipped, and NULs are dropped."""
    import re
    import unicodedata

    import jaconv
    from misaki import cutlet, num2kana

    def units(d):
        n = len(d)
        out = ""
        for start, end, unit in ((max(n - 16, 0), max(n - 12, 0), "ちょう"), (max(n - 12, 0), n - 8, "おく"), (n - 8, n - 4, "まん")):
            group = d[start:end]
            if not group or set(group) == {"0"}:
                continue
            kana = num2kana.remove_spaces(num2kana.len_four(group.rjust(4, "0"), num2kana.hiragana_dict, False))
            if unit == "ちょう":
                for plain, geminate in (("いち", "いっ"), ("はち", "はっ"), ("じゅう", "じゅっ")):
                    if kana.endswith(plain):
                        kana = kana[: -len(plain)] + geminate
                        break
            out += kana + unit
        return out + num2kana.remove_spaces(num2kana.len_four(d[-4:], num2kana.hiragana_dict, False))

    def number_words(d):
        if len(d) <= 9:
            return [num2kana.Convert(d)]
        if len(d) <= 16 and d[0] != "0":
            return [units(d)]
        return [num2kana.Convert(x) for x in d]

    c = g2p.cutlet

    def kana_phonemes(kana):
        hira = jaconv.kata2hira(kana).replace("ゅう", "ゅー").replace("ょう", "ょー")
        return romaji_word(cutlet.Word(hira, hira, 6))

    def number_phonemes(t):
        d = "".join(str(unicodedata.digit(ch)) for ch in t)
        return d, " ".join(kana_phonemes(k) for k in number_words(d))

    numbers = []
    original_convert = cutlet.Convert

    def convert(t):
        if t.isascii() and len(t) <= 9:
            return original_convert(t)
        numbers.append(number_phonemes(t))
        return f"\ue000{len(numbers) - 1}\ue001"

    cutlet.Convert = convert
    romaji_word, single = c._romaji_word, c._get_single_mapping

    def fixed_romaji_word(word):
        if word.char_type == -1:
            return word.hira
        if word.surface.isdigit():
            return number_phonemes(word.surface)[1]
        return romaji_word(word)

    def fixed_single(pk, kk, nk):
        if nk and nk in cutlet.SUTEGANA and kk not in c.table and kk not in cutlet.ODORI and kk != "っ":
            if not (pk and (pk + kk) in c.table) and not ((kk + nk) in c.table):
                return c.table[nk]
        return single(pk, kk, nk)

    c._romaji_word, c._get_single_mapping = fixed_romaji_word, fixed_single

    def call(text):
        # Cutlet.__call__, with each number the reference can't read as one word of phonemes.
        text = text.replace("\0", "")
        if not text:
            return "", None
        numbers.clear()
        text = c._normalize_text(text)
        words = []
        for k, piece in enumerate(re.split("\ue000(\\d+)\ue001", text)):
            if k % 2:
                d, ps = numbers[int(piece)]
                words.append(cutlet.Word(d, ps, -1))
            elif piece:
                words.extend(cutlet.Word(
                    w.surface,
                    jaconv.kata2hira(w.feature.pron or w.feature.kana or w.surface),
                    6 if w.char_type == 7 or not w.is_unk else w.char_type
                ) for w in c.tagger(piece))
        out = "".join(str(tok) for tok in c._romaji_tokens(words))
        ps = re.sub(r"\s+", " ", out.strip()).replace("(", "«").replace(")", "»")
        ps = re.sub(r'(?<![!",.:;?»—…”]) (?=ʔ)|(?<=ʔ) (?!["«“])', "", ps)
        return ps, None

    return call


def main():
    mode, paths = sys.argv[1], sys.argv[2:]
    if mode == "texts":
        for t in texts(paths):
            print(json.dumps(t, ensure_ascii=False))
        return
    import jaconv
    from misaki.ja import JAG2P

    g2p = JAG2P()
    if mode == "fixed":
        g2p, mode = with_divergences(g2p), "jag2p"
    else:
        tagger = g2p.cutlet.tagger
    for path in paths:
        with open(path) as f:
            for line in f:
                if not line.strip():
                    continue
                r = json.loads(line)
                text = r["text"]
                if mode == "mecab":
                    try:
                        r["morphs"] = [
                            [w.surface, jaconv.kata2hira(w.feature.pron or w.feature.kana or w.surface), w.char_type, w.is_unk]
                            for w in tagger(text)
                        ]
                    except Exception as e:
                        r["error"] = f"{type(e).__name__}: {e}"
                elif mode == "jag2p":
                    try:
                        r["phonemes"] = g2p(text)[0]
                    except Exception as e:
                        r["error"] = f"{type(e).__name__}: {e}"
                else:
                    raise SystemExit(f"unknown mode {mode}")
                print(json.dumps(r, ensure_ascii=False))


if __name__ == "__main__":
    main()
