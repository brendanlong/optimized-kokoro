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
import re
import json
import os
import subprocess
import unicodedata
from pathlib import Path

import sounds
from spelling import accents_within, fold

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
LANGUAGES = {"pl", "fi", "cs"}
WORLD = "real people's family and given names"
# Too many names to review flags by hand: every spelling flag.py flags is left out. A flagged name is read by
# espeak's English rules (misaki's dictionaries already have the names English says its own way), which read a
# famous foreign name worse than its own language does, so the prompt doesn't flag famous people for being famous.
EXCLUDE_FLAGGED = True
PROMPT = """Below are spellings of real people's family and given names, each followed by the language whose \
speakers mostly bear it. Our text-to-speech reader will say each spelling with that language's pronunciation \
whenever it appears as a word in English text: news, novels, web pages. That's only safe if, in English text, the \
spelling almost always means someone of that name who says it that language's way.

Flag every spelling that is also any of these: an English word (including archaic, technical, or plural forms); a \
name commonly borne by English speakers, or by speakers of another language who say it differently; a place, brand \
or product English text names that isn't that language's (a city elsewhere, a company); a word commonly borrowed \
into English; a common word in Spanish, French, Italian or Portuguese; an abbreviation. When in doubt, flag it.

Don't flag a spelling only because a famous bearer's name is anglicized by English speakers: their own language's \
pronunciation is what we want for them.

Reply with JSON only: an object mapping each flagged spelling, as written without its language, to a short \
reason. Omit spellings you don't flag.

Spellings:
"""
ESPEAK_LOADER = "espeakng-loader==0.2.4"
WORDFREQ = "wordfreq==3.1.1"
# The languages whose pipelines read Latin text as their own (names_skip.json), and how common a word there must be
# (wordfreq's zipf: 2 is once per hundred million words) for a name spelled like it to be read as the word.
PIPELINES = ["es", "fr", "it", "pt"]
SKIP_ZIPF = 2.0
# Where espeak's rules miss a rule of the language: Czech devoices a final v and ř (Jaroslav is -slaf, Kovář
# -aːr̝̊), as espeak already does for its other consonants.
FIXES = {"cs": [(re.compile(r"v(?=$| )"), "f"), (re.compile("r\u031d(?=$| )"), "r\u031d\u030a")]}
# Each language's letters, where its names are read only if they use no others, and spellings its own words
# don't have: a name with them is a loan its rules misread (Swedish Bergholm, Snellman and Ahlroth among Finns,
# German Neuvirth and Fischerová among Czechs).
FOREIGN = {"fi": re.compile(r"th|ph|dt|ae|ehr|^sj|^sv|ström|stedt|roos$|roth$|holm$|hjelm|sköld|skog|lund|strand|vall$|man$"
                            r"|ld$|lm$"),
           "pl": re.compile(r"sch|th|ph|ei|ä|oo|ee|ae|tz|ff"),
           "cs": re.compile(r"sch|th|ph|ei|ee|oo|ae|tz|ff")}
LETTERS = {"fi": set("adeghijklmnoprstuvyäö-"), "pl": set("aąbcćdeęfghijklłmnńoóprsśtuwyzźż-"),
           "cs": set("aábcčdďeéěfghiíjklmnňoóprřsštťuúůvyýzž-")}
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


@functools.cache
def languages_by_fold():
    out = collections.defaultdict(set)
    for s, (language, _) in attributed().items():
        out[fold(s)].add(language)
    return out


def label(spelling):
    """A spelling as flag.py shows it, with the language of the name it reads as: "Wojciech (Polish)"."""
    language = attributed().get(spelling, ("", ""))[0]
    if language not in LANGUAGES:
        language = min(languages_by_fold()[fold(spelling)] & LANGUAGES)
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
    """{spelling: (language, voice, espeak's phonemes)} for the names of `languages`, one word each (Wikidata's
    names of several words are rare and unreviewed: "von Kraemer", "Matuszewski Ryszard"). Left out: a name with
    letters or spellings its language doesn't use (LETTERS, FOREIGN), a name espeak
    reads partly in another language (Vietnamese espeak reads "Baotixita" as English), one it reads with no
    vowel (French "Gees" as ʒ), and one with no vowel letter that espeak doesn't read with a syllabic consonant,
    which it spells ("Vlk" as V-L-K)."""
    names = attributed()
    by_voice = collections.defaultdict(list)
    for spelling, (language, voice) in names.items():
        own = set(spelling.lower()) <= LETTERS[language] if language in LETTERS else " " not in spelling
        foreign = language in FOREIGN and FOREIGN[language].search(spelling.lower())
        if language in languages and own and not foreign:
            by_voice[voice].append(spelling)
    out, data = {}, espeak_data() if by_voice else None
    for voice, spellings in sorted(by_voice.items()):
        for s, (raw, switched) in zip(spellings, read(voice, spellings, data)):
            for pattern, sound in FIXES.get(names[s][0], []):
                raw = pattern.sub(sound, raw)
            spelled = not VOWEL_LETTERS & set(s.lower()) and "\u0329" not in raw
            if not switched and not spelled and sounds.has_vowel(sounds.kokoro(raw)):
                out[s] = (names[s][0], voice, raw)
    return out


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; no name takes a plural."""
    out = {s: (sounds.kokoro(raw), False) for s, (_, _, raw) in readings(LANGUAGES).items()}
    kept = {fold(s) for s in out}
    out.update({s: (None, False) for s in attributed() if s not in out and fold(s) in kept})
    return out


def zipfs(words):
    """{(word, language): wordfreq's zipf frequency} for each (word, language) in `words`."""
    script = ("import json, sys\nfrom wordfreq import zipf_frequency\n"
              "print(json.dumps([zipf_frequency(w, lang) for w, lang in json.load(sys.stdin)]))")
    return dict(zip(words, wordfreq(script, json.dumps(list(words)))))


def common_words(languages, zipf):
    """{language: words used at least `zipf` often in it}, from wordfreq."""
    script = ("import json, sys\nfrom wordfreq import iter_wordlist, zipf_frequency\nout = {}\n"
              "for lang in sys.argv[2:]:\n    out[lang] = []\n    for w in iter_wordlist(lang):\n"
              "        if zipf_frequency(w, lang) < float(sys.argv[1]):\n            break\n        out[lang].append(w)\n"
              "print(json.dumps(out))")
    return wordfreq(script, "", str(zipf), *languages)


def wordfreq(script, stdin, *args):
    res = subprocess.run(["uv", "run", "-q", "--no-project", "--with", WORDFREQ, "python", "-I", "-c", script, *args],
                         input=stdin, capture_output=True, text=True, cwd="/")
    if res.returncode:
        raise SystemExit(f"wordfreq: {res.stderr}")
    return json.loads(res.stdout)


def skip(kept):
    """{pipeline language: words there the G2P would read as one of the names `kept`}, for names_skip.json:
    words of the language the pipeline reads as words ("las" in Spanish isn't a Polish Las). A spelling counts as
    one where it's used SKIP_ZIPF often there, and ten times as often as in English (with or without its accents):
    a name famous enough to be common in French ("Wojciech", "Räikkönen") is about as common in English, where a
    Spanish word ("jalo") isn't."""
    by_fold = collections.defaultdict(list)
    for name in kept:
        by_fold[fold(name)].append(name)
    found = []
    for language, words in common_words(PIPELINES, SKIP_ZIPF).items():
        for w in words:
            for n in by_fold.get(fold(w), ()):
                if accents_within(w, n) and attributed().get(n, ("", ""))[0] in LANGUAGES:
                    found.append((w, language))
    plain = {w: unicodedata.normalize("NFC", "".join(c for c in unicodedata.normalize("NFD", w)
                                                       if not unicodedata.combining(c))) for w, _ in found}
    freq = zipfs(set(found) | {(w, "en") for w, _ in found} | {(p, "en") for p in plain.values()})
    out = collections.defaultdict(set)
    for w, language in found:
        if freq[w, language] > max(freq[w, "en"], freq[plain[w], "en"]) + 1:
            out[language].add(w)
    return dict(out)
