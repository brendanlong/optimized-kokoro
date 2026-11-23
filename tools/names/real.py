"""Family and given names of real languages (real_names.tsv, from Wikidata), read by espeak-ng in their own
language.

Each spelling real_names.tsv gives one of LANGUAGES is read by espeak-ng 1.52 with the voice most of its
bearers' countries speak (Mexican names with es-419's s, not Spain's θ), through kokoro-espeak's `read_words`
example on espeakng-loader's copy of espeak's data, the copy the reference pipeline uses; sounds.py writes its
phonemes in Kokoro's. A spelling of no language or another one that the G2P would take for one of these names
(it finds names without their accents: "Nguyen" for "Nguyễn") is listed with no phonemes: it isn't that name.
"""

import collections
import functools
import json
import os
import subprocess
from pathlib import Path

import sounds
from spelling import fold

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
LANGUAGE_NAMES = {"pl": "Polish", "hu": "Hungarian", "cs": "Czech", "sk": "Slovak", "tr": "Turkish", "fi": "Finnish",
                  "de": "German", "fr": "French", "es": "Spanish", "it": "Italian", "pt": "Portuguese", "nl": "Dutch",
                  "sv": "Swedish", "nb": "Norwegian", "da": "Danish", "et": "Estonian", "lv": "Latvian",
                  "lt": "Lithuanian", "sl": "Slovenian", "hr": "Croatian", "ro": "Romanian", "is": "Icelandic",
                  "id": "Indonesian", "ms": "Malay", "sw": "Swahili", "vi": "Vietnamese", "sq": "Albanian",
                  "az": "Azerbaijani", "mt": "Maltese", "mi": "Māori", "ga": "Irish", "bs": "Bosnian",
                  "ht": "Haitian Creole"}
# The languages whose names are read so far.
LANGUAGES = set()
WORLD = "real people's family and given names"
PROMPT = """Below are spellings of real people's family and given names, each followed by the language whose \
speakers mostly bear it. Our text-to-speech reader will say each spelling with that language's pronunciation \
whenever it appears as a word in English text: news, novels, web pages. That's only safe if, in English text, the \
spelling almost always means someone of that name who says it that language's way.

Flag every spelling that is also any of these: an English word (including archaic, technical, or plural forms); a \
name commonly borne by English speakers, or by speakers of another language who say it differently; a well-known \
person, place, brand or product whose name English speakers say differently from that language's way; a word \
commonly borrowed into English; a common word in Spanish, French, Italian or Portuguese; an abbreviation. When in \
doubt, flag it.

Reply with JSON only: an object mapping each flagged spelling, as written without its language, to a short \
reason. Omit spellings you don't flag.

Spellings:
"""
ESPEAK_LOADER = "espeakng-loader==0.2.4"
VOWEL_LETTERS = set("aeiouyáéíóúýàèìòùâêîôûäëïöüÿąęåæøœőűěůãõ")


@functools.cache
def attributed():
    """{spelling: (language, voice)}, language and voice "" where the bearers don't settle on one."""
    out = {}
    for line in open(HERE / "real_names.tsv", encoding="utf-8"):
        if not line.startswith("#"):
            spelling, language, voice, _, _ = line.rstrip("\n").split("\t")
            out[spelling] = (language, voice)
    return out


def label(spelling):
    """A spelling as flag.py shows it, with the language of the name it reads as: "Wojciech (Polish)"."""
    language = attributed().get(spelling, ("", ""))[0]
    if language not in LANGUAGES:
        language = next(lang for s, (lang, _) in attributed().items() if lang in LANGUAGES and fold(s) == fold(spelling))
    return f"{spelling} ({LANGUAGE_NAMES[language]})"


def espeak_data():
    return subprocess.run(["uv", "run", "-q", "--no-project", "--with", ESPEAK_LOADER, "python", "-I", "-c",
                           "import espeakng_loader; print(espeakng_loader.get_data_path())"],
                          capture_output=True, text=True, check=True, cwd="/").stdout.strip()


def read(voice, words, data):
    """(espeak's phonemes, whether it read some of the word in another language) for each word."""
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "kokoro-espeak", "--example", "read_words"],
                   cwd=ROOT, check=True)
    res = subprocess.run([str(ROOT / "target/release/examples/read_words"), voice], input="\n".join(words) + "\n",
                         capture_output=True, text=True, check=True, env={**os.environ, "ESPEAK_DATA": data})
    out = [json.loads(line) for line in res.stdout.splitlines()]
    assert len(out) == len(words)
    return out


def readings(languages):
    """{spelling: (language, voice, espeak's phonemes)} for the names of `languages`. Left out: a name espeak
    reads partly in another language (Vietnamese espeak reads "Baotixita" as English), and one with no vowel
    letter that espeak doesn't read with a syllabic consonant, which it spells ("Vlk" as V-L-K)."""
    names = attributed()
    by_voice = collections.defaultdict(list)
    for spelling, (language, voice) in names.items():
        if language in languages:
            by_voice[voice].append(spelling)
    out, data = {}, espeak_data() if by_voice else None
    for voice, spellings in sorted(by_voice.items()):
        for s, (raw, switched) in zip(spellings, read(voice, spellings, data)):
            if not switched and (VOWEL_LETTERS & set(s.lower()) or "\u0329" in raw):
                out[s] = (names[s][0], voice, raw)
    return out


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; no name takes a plural."""
    out = {s: (sounds.kokoro(raw), False) for s, (_, _, raw) in readings(LANGUAGES).items()}
    kept = {fold(s) for s in out}
    out.update({s: (None, False) for s in attributed() if s not in out and fold(s) in kept})
    return out
