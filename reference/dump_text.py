"""Records the reference server's text functions (everything before G2P) on hard cases.

The book and Wikipedia corpora rarely reach the normalizer's URL, email, phone, unit, money, time
and caps passes, or the tag and SSML handling. This runs those functions directly on hand-written
cases (text_cases.jsonl) plus seeded random combinations of the tokens they react to.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I dump_text.py fixtures/text_cases.jsonl fixtures/text_cases.out.jsonl

Input lines: {"kind": ..., ...}, kinds as below. Output lines add the result, or "error" (the
exception's type) where the reference raises:
    normalize     text, lang, options   -> normalized (join_lines then the normalizer, outside
                                           annotations, as smart_split does), sentences
    smart_split   text, lang, options, max_tokens -> chunks: [text, phonemes] or a pause in
                                           seconds, with a stand-in G2P (phonemes = text, or ""
                                           when it has nothing alphanumeric)
    voices        text, voice           -> segments: [voice, rate, text] from split_by_voice
    checks        text, allow_voice_tags, remove_emoji -> speakable, pause_budget (error text or null)
    emoji         text                  -> stripped
    ssml          text, voice, allow_voice_tags -> translated
"""

import asyncio
import json
import os
import random
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "kfa"))

from loguru import logger  # noqa: E402

from api.src.inference.g2p import Phonemes  # noqa: E402
from api.src.services.text_processing import ssml  # noqa: E402
from api.src.services.text_processing import text_processor as tp  # noqa: E402
from api.src.structures.schemas import NormalizationOptions  # noqa: E402

logger.remove()

OPTION_SETS = {
    "default": {},
    "all": {"unit_normalization": True, "remove_emoji": True},
    "none": {
        "url_normalization": False,
        "email_normalization": False,
        "optional_pluralization_normalization": False,
        "phone_normalization": False,
        "caps_normalization": False,
        "replace_remaining_symbols": False,
    },
    "off": {"normalize": False},
}


def fake_phonemize(text, lang_code):
    return Phonemes(text, text if any(c.isalnum() for c in text) else "")


tp.phonemize = fake_phonemize


def normalized(text, lang_code, options):
    processed = tp.join_lines(text)
    normalizer = tp.get_normalizer(lang_code)
    if tp.settings.advanced_text_normalization and options.normalize and normalizer is not None:
        parts = tp.CUSTOM_PHONEMES.split(processed)
        for index in range(0, len(parts), 2):
            parts[index] = normalizer.normalize(parts[index], options)
        processed = "".join(parts).strip()
    return processed


def guarded(f):
    try:
        return {"result": f()}
    except Exception as e:  # the Rust side reports the same failures as errors
        return {"error": type(e).__name__}


async def run(case):
    kind = case["kind"]
    out = dict(case)
    options = NormalizationOptions(**OPTION_SETS[case.get("options", "default")])
    if kind == "normalize":
        r = guarded(lambda: normalized(case["text"], case.get("lang", "a"), options))
        if "result" in r:
            out["normalized"] = r["result"]
            out["sentences"] = [s for s in tp.sentences(r["result"])]
        else:
            out["error"] = r["error"]
    elif kind == "smart_split":
        chunks = []
        async for text, phonemes, pause in tp.smart_split(
            case["text"], max_tokens=case["max_tokens"], lang_code=case.get("lang", "a"), normalization_options=options
        ):
            chunks.append(pause if phonemes is None else [text, phonemes.phonemes])
        out["chunks"] = chunks
    elif kind == "voices":
        out["segments"] = [list(s) for s in tp.split_by_voice(case["text"], case["voice"])]
    elif kind == "checks":
        opts = NormalizationOptions(remove_emoji=case["remove_emoji"])
        speakable = guarded(lambda: tp.check_speakable(case["text"], case["allow_voice_tags"], opts))
        out["speakable"] = "error" not in speakable
        try:
            tp.check_pause_budget(case["text"])
            out["pause_budget"] = None
        except ValueError as e:
            out["pause_budget"] = str(e)
    elif kind == "emoji":
        out["stripped"] = tp.strip_emoji(case["text"])
    elif kind == "ssml":
        r = guarded(lambda: ssml.translate_ssml(case["text"], case["voice"], case["allow_voice_tags"]))
        out.update({"translated": r["result"]} if "result" in r else {"error": r["error"]})
    else:
        raise ValueError(kind)
    return out


# Tokens the normalizer and tag handling react to, glued together at random.
FRAGMENTS = [
    "1", "12", "123", "1234", "1,234", "12,34", "1,234,567", "1,23,456", "0", "00", "05", "007", "1984", "2000",
    "2010", "1500", "1501", "9999", "10000", "0.5", ".5", "3.14159", "1.0", "2.50", "1.2.3", "10.0.1", "4.2.1.0",
    "1e5", "99999999999999999999", "1" + "0" * 40, "123456789012345678901234567890123456", "٣", "١٢٣", "１２",
    "۴۵", "$", "£", "€", "-", "+", "k", "m", "b", "t", "K", "M", "B", " hundred", " thousand", " million",
    " billion", " trillion", " quadrillion", " Million", "km", "KB", "kb", "MB", "Mbps", "mbps", "GHz", "°C", "°F",
    "µF", "μF", "Ω", "kΩ", "mi/h", "m/s", "s", "ms", "min", "h", "px", "in", "ft", "mph", "atm", "kg", "g", "B",
    " ", " ", " ", "", "-", "--", "---", "—", "–", ".", ",", ":", ";", "/", "(", ")", "(s)", "@", "#", "%", "&", "*",
    "_", "|", "\\", "=", "~", "^", " ", "\t", "\n", "\n\n", "\r\n", "\x1c", " ", "…", "...", "'", "’",
    "‘", "“", "”", "«", "»", "、", "。", "！", "，", "：", "；", "？", "how're", "What’re", "theyre", "youre", "whore",
    "Dr.", "DR.", "Mr.", "MR.", "Ms.", "MRS.", "Mrs.", "etc.", "etc. The", "yeah", "Yea", "YEAH", "NASA", "UNESCO",
    "IV", "XVI", "MCMLXXXIV", "HELLO WORLD", "THE QUICK BROWN FOX", "ABC-DEF", "NVIDIA'S", "Ab", "x", "word", "The",
    "www.", "http://", "https://", "HTTPS://", "example", ".com", ".io", ".co.uk", ".org/path?q=1&r=2",
    ":8080", "localhost", "192.168.0.1", "user.name+tag@", "mail.example.com", "a.b.", "U.S.A.", "e.g. ", "i.e.",
    "p.m.", "AM", "pm", "am", "12:30", "3:05", "10:00", "1:2", "12:30:45", "7:00:01", " PM", "(555)", "555-1234",
    "+1 ", "800.555.0199", "İ", "ı", "ſ", "K", "é", "é", "²", "½", "ß", "Σ", "ΑΒΓ", "😀", "👍🏽", "🇺🇸",
    "1️⃣", "#️⃣", "👨‍👩‍👧", "[pause:1s]", "[pause:0.5s]", "[PAUSE:2S]", "[Kokoro](/kˈOkəɹO/)", "[Hi!](/hˈI/)",
    "[voice:af_sky]", "[rate:1.5]", "[baserate:0.8]", "", "1",
]


def fuzz_text(rng):
    return "".join(rng.choice(FRAGMENTS) for _ in range(rng.randint(1, 12)))


# Characters where Python's \w, \d, \s or case folding differ from Unicode's usual definitions.
ODD_CHARS = (
    "\x1c\x1d\x1e\x1f\x85᠎​⁠﻿́ͅ⃝ःि²³¹½⅓Ⅻↂ〇‿⁀_"
    "٠٩۰۹߀०९০௫๓༣၉០᠕᥆᧐᪀᭙᮰᱀꘠꣐꤀꧐꩐꯰０９𐒠𝟎𝟗🯰İıſKÅΩµμΜͅιΣςσẞßǅǈ"
)


def odd_text(rng):
    parts = []
    for _ in range(rng.randint(1, 10)):
        parts.append(rng.choice(ODD_CHARS) if rng.random() < 0.5 else rng.choice(FRAGMENTS))
    return "".join(parts)


def generated(rng):
    for n in range(3000):
        yield {"kind": "normalize", "id": f"fuzz-normalize-{n}", "text": fuzz_text(rng),
               "options": rng.choice(list(OPTION_SETS))}
    for n in range(1000):
        yield {"kind": "normalize", "id": f"fuzz-odd-{n}", "text": odd_text(rng), "options": rng.choice(list(OPTION_SETS))}
    for n in range(300):
        text = " ".join(fuzz_text(rng) for _ in range(rng.randint(1, 6)))
        yield {"kind": "smart_split", "id": f"fuzz-split-{n}", "text": text, "options": rng.choice(list(OPTION_SETS)),
               "max_tokens": rng.choice([8, 20, 40, 450])}
    tags = ["[voice:af_sky]", "[voice: bm_george ]", "[VOICE:af_bella(2)+af_sky]", "[rate:2]", "[rate: 0.1 ]",
            "[baserate:1.5]", "[rate:9]", "[voice:]", "[pause:3s]", "😀", " ", "hello", "world.", "\n"]
    for n in range(300):
        text = "".join(rng.choice(tags) for _ in range(rng.randint(1, 8)))
        yield {"kind": "voices", "id": f"fuzz-voices-{n}", "text": text, "voice": "af_heart"}
        yield {"kind": "checks", "id": f"fuzz-checks-{n}", "text": text, "allow_voice_tags": rng.random() < 0.5,
               "remove_emoji": rng.random() < 0.5}
    for n in range(200):
        yield {"kind": "emoji", "id": f"fuzz-emoji-{n}", "text": fuzz_text(rng)}


async def main(src, dst):
    with open(src) as f:
        cases = [json.loads(line) for line in f if line.strip()]
    cases.extend(generated(random.Random(20261102)))
    with open(dst, "w") as f:
        for case in cases:
            f.write(json.dumps(await run(case), ensure_ascii=False) + "\n")


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], sys.argv[2]))
