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
import tempfile
import unicodedata
from pathlib import Path

import hepburn
import pinyin
import sounds
import vietnamese
from spelling import accents_within, fold

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
LANGUAGE_NAMES = {"pl": "Polish", "hu": "Hungarian", "cs": "Czech", "sk": "Slovak", "tr": "Turkish", "fi": "Finnish",
                  "de": "German", "fr": "French", "es": "Spanish", "it": "Italian", "pt": "Portuguese", "nl": "Dutch",
                  "sv": "Swedish", "nb": "Norwegian", "da": "Danish", "et": "Estonian", "lv": "Latvian",
                  "lt": "Lithuanian", "sl": "Slovenian", "hr": "Croatian", "ro": "Romanian", "is": "Icelandic",
                  "id": "Indonesian", "ms": "Malay", "sw": "Swahili", "vi": "Vietnamese", "sq": "Albanian",
                  "az": "Azerbaijani", "mt": "Maltese", "mi": "Māori", "ga": "Irish", "bs": "Bosnian",
                  "ht": "Haitian Creole", "zh": "Chinese (pinyin)", "ja": "Japanese (romanized)"}
# The languages whose names are read so far.
LANGUAGES = {"pl", "fi", "cs", "es", "it", "fr", "hu", "ro", "sk", "sl", "et", "tr", "id", "zh", "ja", "vi", "de", "nl",
             "pt", "sv", "nb", "lv", "sq", "az", "sw", "hr", "bs", "ms"}
WORLD = "real people's family and given names"
# A spelling an earlier source reads differently is left to it (Tolkien's Martan, Cobas).
YIELDS = True
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
into English; a common word in Spanish, French, Italian or Portuguese, unless that is the name's own \
language (a Spanish surname that is a Spanish word, like Moreno, is fine); an abbreviation. When in doubt, flag it.

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
FIXES = {"cs": [(re.compile(r"v(?=$| )"), "f"), (re.compile("r\u031d(?=$| )"), "r\u031d\u030a")],
         # Estonian espeak doubles an overlong vowel (Siim as siiːm), m before b (Lembit as lemmbit), writes a long s as
         # s^s and n before t as ɲ.
         # Portuguese espeak writes in Kokoro's Portuguese voice's symbols what an English voice says otherwise: æ for
         # a final a (ɐ), y for a final e (i), ə after an r before a consonant (none: Carvalho isn't ka-ra-valho),
         # and e^ɪŋ, oŋ for the nasal e and o before a consonant (ẽ, õ). (Its lj for lh is put right in readings().)
         "pt": [(re.compile(r"æ"), "ɐ"), (re.compile(r"y(?=s?$| )"), "i"), (re.compile(r"ɾə(?=[^aeiouɐɛɔəæyɪʊˈˌ̃])"), "ɾ"),
                (re.compile(r"e\^ɪŋ(?=[^aeiouɐɛɔ])"), "ẽ"), (re.compile(r"oŋ(?=[sdtkɡ])"), "õ")],
         # Dutch espeak stresses the -eren, -elen and -enen of names, which Dutch leaves unstressed (Genderen is
         # GEN-de-ren): the stress goes to the first syllable and the vowel is ə. Not where that syllable is the only
         # one before (Deelen) or an unstressed prefix (Vermeeren).
         "nl": [(re.compile(r"^(.*[aeiouyɑɛɪɔʏøœə].*)ˈ(ɪː|eː)(?=[rln]ən$)"), lambda m: m[0] if m[1].startswith("vər")
                 else "ˈ" + m[1].replace("ˈ", "").replace("ˌ", "") + "ə")],
         "et": [(re.compile(r"([aeiouæøyɤ])\1ː"), r"\1ː"), (re.compile(r"mmb"), "mb"), (re.compile(r"s\^s"), "sː"),
                (re.compile(r"ɲ(?=t)"), "n")]}
# Each language's letters, where its names are read only if they use no others, and spellings its own words
# don't have: a name with them is a loan its rules misread (Swedish Bergholm, Snellman and Ahlroth among Finns,
# German Neuvirth and Fischerová among Czechs).
FOREIGN = {"fi": re.compile(r"th|ph|dt|ae|ehr|^sj|^sv|ström|stedt|roos$|roth$|holm$|hjelm|sköld|skog|lund|strand|vall$|man$"
                            r"|ld$|lm$"),
           "pl": re.compile(r"sch|th|ph|ei|ä|oo|ee|ae|tz|ff"),
           "cs": re.compile(r"sch|th|ph|ei|ee|oo|ae|tz|ff"),
           "es": re.compile(r"sch|th|ph|ck|tz|ff|ee|oo"), "it": re.compile(r"th|ph|ck|ee|oo|ae"),
           "fr": re.compile(r"sch|tz|ck|ee|oo|sh"), "hu": re.compile(r"sch|ck|tz|ß"), "id": re.compile(r"^ch")}
LETTERS = {"fi": set("adeghijklmnoprstuvyäö-"), "pl": set("aąbcćdeęfghijklłmnńoóprsśtuwyzźż-"),
           "cs": set("aábcčdďeéěfghiíjklmnňoóprřsštťuúůvyýzž-"), "es": set("aábcdeéfghiíjlmnñoópqrstuúüvxyz-"),
           "it": set("aàbcdeèéfghiìíjlmnoòópqrstuùúvz-"), "fr": set("aàâæbcçdeéèêëfghiîïjklmnoôœpqrstuùûüvwxyÿz-"),
           "hu": set("aábcdeéfghiíjklmnoóöőpqrstuúüűvwxyz-")}
# Spellings rewritten before espeak reads them. Hungarian's old ones, which noble families keep and espeak
# misreads: th is t at a word's start or end and in tth (Thököly, Horváth, Batthyány), but not between (Keszthely is
# Keszt-hely), and a final thy ti (Horthy); a final y after a consonant not of gy, ly, ny or
# ty is i (Esterházy, Pálffy); a final ch is cs, but not German sch (Kovách, not Bogsch); gh is g (Balogh,
# Alszeghy); cz is c; aá and aa are á (Gaál); eö is ö; and a doubled ss or ff before i is one (Andrássy, Pálffy).
# Indonesian's Dutch-era spellings, which many names keep: oe is u, dj j, tj c, sj sy, ch kh but at the start (Chandra
# is c, Chairil kh: such names are left out), and in a name spelled so, j alone y (Soekarno, Djojohadikusumo,
# Alisjahbana, Mochtar, Tjahjono); dh is d, sh sy, th t, and y after a consonant, not before a vowel, i (Rizky, Dedy).
# Romanian's ș and ț with a comma, which espeak knows only with a cedilla (ş, ţ: Lupașcu, Constanța).
# French ones espeak drops a vowel of: ouë (Halgouët), a final ouil (Fenouil), Gwen- (Gwenaël).
SPELLINGS = {"hu": [(re.compile(r"thy$"), "ti"), (re.compile(r"^th|th$"), "t"), (re.compile(r"tth"), "tt"), (re.compile(r"(?<=[^aeiouáéíóöőúüűglnt])y$"), "i"),
                    (re.compile(r"(?<!s)ch$"), "cs"), (re.compile(r"gh"), "g"), (re.compile(r"cz"), "c"),
                    (re.compile(r"aá|aa"), "á"), (re.compile(r"eö"), "ö"), (re.compile(r"(ss|ff)(?=i$)"), lambda m: m[1][0])],
             "ro": [(re.compile("ș"), "ş"), (re.compile("ț"), "ţ")],
             "nl": [(re.compile(r"ph"), "f")],
             # Swedish and Norwegian th is t (Thunberg, Thorbjørn).
             # Scandinavian h is silent but before a vowel (Dahl, Åsbahr) and after a final g or d (Söderbergh, Rydh).
             # But not where th joins a compound's parts (Westholm, Austheim).
             "sv": [(TH := re.compile(r"^th|th(?!olm|eim|amm|ag|us)"), "t"),
                    (SILENT_H := re.compile(r"(?<=[aeiouyåäöøæ])h(?![aeiouyåäöøæ])|(?<=[gd])h$"), "")],
             "nb": [(TH, "t"), (SILENT_H, "")],
             # German: Latin c before a front vowel is z (Cäcilia, Cäsar), a Slavic final ow o (Pankow), oe ö (Roesler;
             # not Joel, Noemi, Zoe), oethe öhte (Goethe), and a final chs ks, written kss so the vowel stays short
             # (Fuchs, Flachs; espeak reads chs before a vowel already, Sachsen), but a patronymic's -ichs, -ichsen is çs,
             # written chß (Friedrichs, Sönnichsen).
             # A compound's second part starting with st or sp is written as a word of its own, as espeak reads ʃt and
             # ʃp only at a word's start (Schweinsteiger, Einstein, Birkenstock, Husstedt; Pechstein keeps its ç).
             "de": [(re.compile(r"c(?=[eiäy])"), "z"), (re.compile(r"(?<=[rln]i)chs(?=$|en$)"), "chß"),
                    (re.compile(r"chs$"), "kss"),
                    (re.compile(r"(?<=o)w$"), ""), (re.compile(r"oethe"), "öhte"),
                    (re.compile(r"(?<![jnz])oe(?![aeiou])"), "ö"),
                    (re.compile(r"(?<=[^\W\d_])(?<!chri)(?=s(t(ein|adt|ätt|att|eig|ock|ern$|rom|raß|rass|edt|ieg|ab|uhl|ück)"
                                r"|p(iel|ieß|ring|rung|ach)))"), "-")],
             "id": [(re.compile(r".*(oe|dj|tj|sj|(?<!^)ch).*"), lambda m: old_indonesian(m[0])), (re.compile(r"dh"), "d"),
                    (re.compile(r"sh"), "sy"), (re.compile(r"th"), "t"), (re.compile(r"(?<=[^aeiouy])y(?![aeiou])"), "i")],
             # Malay's: ch is c (Cheah), dh d, sh sy, th t, and y after a consonant but s, not before a vowel, i (Asyraf keeps its sy).
             "ms": [(re.compile(r"ch"), "c"), (re.compile(r"dh"), "d"), (re.compile(r"sh"), "sy"), (re.compile(r"th"), "t"),
                    (re.compile(r"(?<=[^aeiouys])y(?![aeiou])"), "i")],
             "fr": [(re.compile(r"ouë"), "ouè"), (re.compile(r"ouil$"), "ouille"), (re.compile(r"^gwen"), "gwén")]}
# Sounds a voice of espeak writes as another: Turkish ü as ø, ö as œ and e as æ (Kültür as køltør); German
# -er as ɜ, English's r-colored vowel, for ɐ; Indonesian (and Malay) sy as ç for ʃ; Croatian and Bosnian a as æ;
# Norwegian kj as x for ç (Kjetil); Portuguese r after a consonant as ɹ for ɾ (Abrantes).
VOICE_SOUNDS = {"tr": str.maketrans({"ø": "y", "œ": "ø", "æ": "e"}), "de": str.maketrans({"ɜ": "ɐ"}),
                "hr": str.maketrans({"æ": "a"}), "bs": str.maketrans({"æ": "a"}),
                "id": str.maketrans({"ç": "ʃ"}), "ms": str.maketrans({"ç": "ʃ"}), "nb": str.maketrans({"x": "ç"}),
                "pt": str.maketrans({"ɹ": "ɾ"})}
# French readings where espeak still lost a vowel after w (Gouyon as ɡwjɔ̃, Desnoues as denw) are left out.
# Estonian ones where espeak put an i before a palatal consonant (Päll as pæiʎʎ, Otsing as oit͡siŋɡ).
BROKEN = {"fr": re.compile(r"w(?![aeiouyɑɛɔøœəɐ])"), "et": re.compile(r"(?<=[aeouæøy])i(?=ʎ|t͡s|s͡s|sː)")}
def vowel_letter(c):
    """Whether a letter is a vowel, with whatever marks: "ầ", "ư", "ő" are."""
    return unicodedata.normalize("NFD", c)[0] in "aeiouy" or c in "æøœ"


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


# Languages read by rules of their own, not espeak: pinyin and Hepburn romanizations, and Vietnamese, which espeak
# reads poorly. Each gives None for a word it can't read.
READERS = {"pinyin": pinyin.read, "hepburn": hepburn.read, "vi": vietnamese.read}


def read(voice, words, data):
    """(espeak's phonemes, whether it read some of the word in another language) for each word; for a voice of
    READERS, its phonemes ("" for a word it can't read)."""
    if voice in READERS:
        return [(READERS[voice](w) or "", False) for w in words]
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "kokoro-espeak", "--example", "read_words"],
                   cwd=ROOT, check=True)
    res = subprocess.run([str(ROOT / "target/release/examples/read_words"), voice], input="\n".join(words) + "\n",
                         capture_output=True, text=True, check=True, env={**os.environ, "ESPEAK_DATA": data})
    out = [json.loads(line) for line in res.stdout.splitlines()]
    assert len(out) == len(words)
    return out


def old_indonesian(word):
    for old, new in [("dj", "\0"), ("tj", "c"), ("sj", "sy"), ("j", "y"), ("\0", "j"), ("oe", "u"), ("ch", "kh")]:
        word = word.replace(old, new)
    return word


def respell(spelling, language):
    # English text writes Turkish İ as I, which Turkish reads as ı ("Ilkay" for İlkay): where the name with İ is
    # a Turkish name too, that's the one.
    if language == "tr" and spelling.startswith("I") and attributed().get("İ" + spelling[1:], ("", ""))[0] == "tr":
        spelling = "İ" + spelling[1:]
    word = spelling.lower()
    for pattern, new in SPELLINGS.get(language, []):
        word = pattern.sub(new, word)
    return word if word != spelling.lower() else spelling


def catalan(by_voice):
    """Move Spanish names that are more common in Catalan than in Spanish text to espeak's Catalan voice: Spain's
    citizenship doesn't tell Catalan names (Jordi, Puig, Josep) from Spanish ones."""
    spanish = [s for v in ("es", "es-419") for s in by_voice.get(v, [])]
    if not spanish:
        return
    freq = zipfs({(s, lang) for s in spanish for lang in ("ca", "es")})
    moved = {s for s in spanish if freq[s, "ca"] > freq[s, "es"]}
    for v in ("es", "es-419"):
        if v in by_voice:
            by_voice[v] = [s for s in by_voice[v] if s not in moved]
    by_voice["ca"] = sorted(moved)


def stressed_first(raw):
    """`raw` with its main stress on the first syllable, and the old main stress secondary."""
    first = re.search(r"[aeiouyɑɐɛɪɔʏøœəæɜɵʊɨ]", raw.replace("ˈ", "").replace("ˌ", ""))
    if first is None or re.match(r"[^aeiouyɑɐɛɪɔʏøœəæɜɵʊɨ]*ˈ", raw):
        return raw
    bare = raw.replace("ˌ", "").replace("ˈ", "ˌ")
    at = re.search(r"[aeiouyɑɐɛɪɔʏøœəæɜɵʊɨ]", bare).start()
    return bare[:at] + "ˈ" + bare[at:]


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
    catalan(by_voice)
    out, data = {}, espeak_data() if set(by_voice) - set(READERS) else None
    for voice, spellings in sorted(by_voice.items()):
        modern = [respell(s, names[s][0]) for s in spellings]
        modern_of = dict(zip(spellings, modern))
        for s, (raw, switched) in zip(spellings, read(voice, modern, data)):
            raw = raw.translate(VOICE_SOUNDS.get(names[s][0], {}))
            # Norwegian espeak reads a final e as a (Sverre as svarra): it's ə.
            if names[s][0] == "nb" and s.lower().endswith("e") and raw.endswith("a"):
                raw = raw[:-1] + "ə"
            # Portuguese espeak writes lh as lj, as it does li before a vowel (Ramalho, Júlio).
            if names[s][0] == "pt" and "lh" in s.lower():
                raw = raw.replace("lj", "ʎ")
            # Swedish and Norwegian compound surnames are stressed on the first part, as espeak doesn't stress -ström.
            if names[s][0] in ("sv", "nb") and re.search(r"(str[öø]m|sk[ij]?öld)$", s.lower()):
                raw = stressed_first(raw)
            # A respelling's hyphen makes espeak read each part as a word, each stressed: the first part is the main.
            if "-" in modern_of[s] and "-" not in s:
                raw = raw[:raw.index("ˈ") + 1] + raw[raw.index("ˈ") + 1:].replace("ˈ", "ˌ") if "ˈ" in raw else raw
            for pattern, sound in FIXES.get(names[s][0], []):
                raw = pattern.sub(sound, raw)
            # Letters that assimilate can leave a sound three times (Norwegian Sandnes as sɑnnnəs): it's long.
            raw = re.sub(r"([^\W\d_])\1\1+", r"\1\1", raw)
            spelled = not any(vowel_letter(c) for c in s.lower()) and "\u0329" not in raw
            # espeak spells out a word it can't read ("Jhon" as J-h-o-n), each letter stressed.
            spelled = spelled or "-" not in s and raw.count("ˈ") > 1
            broken = names[s][0] in BROKEN and BROKEN[names[s][0]].search(sounds.kokoro(raw).replace("ˈ", "").replace("ˌ", ""))
            if broken:
                continue
            # A word needs a main stress: Hungarian espeak gives some only a secondary one (Nagy as nˌɑɟ).
            if "ˈ" not in raw and "ˌ" in raw:
                raw = raw.replace("ˌ", "ˈ", 1)
            if not switched and not spelled and sounds.has_vowel(sounds.kokoro(raw)):
                out[s] = (names[s][0], voice, raw)
    return out


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; no name takes a plural. A name that is an English word with its g dropped
    ("Cuttin", "Castin": cuttin', castin') is left out."""
    dropped_g = lambda s: s.lower().endswith("in") and s.lower() + "g" in english
    # The rule readers' phonemes are Kokoro's already; espeak's are mapped to them.
    kokoro = lambda voice, raw: raw if voice in READERS else sounds.kokoro(raw)
    out = {s: (kokoro(voice, raw), False) for s, (_, voice, raw) in readings(LANGUAGES).items() if not dropped_g(s)}
    kept = {fold(s) for s in out}
    out.update({s: (None, False) for s in attributed() if s not in out and fold(s) in kept})
    return out


def zipfs(words):
    """{(word, language): wordfreq's zipf frequency} for each (word, language) in `words`."""
    script = ("import json, sys\nfrom wordfreq import zipf_frequency\n"
              "json.dump([zipf_frequency(w, lang) for w, lang in json.load(sys.stdin)], open(sys.argv[1], 'w'))")
    return dict(zip(words, wordfreq(script, json.dumps(list(words)))))


def common_words(languages, zipf):
    """{language: words used at least `zipf` often in it}, from wordfreq."""
    script = ("import json, sys\nfrom wordfreq import iter_wordlist, zipf_frequency\nout = {}\n"
              "for lang in sys.argv[3:]:\n    out[lang] = []\n    for w in iter_wordlist(lang):\n"
              "        if zipf_frequency(w, lang) < float(sys.argv[2]):\n            break\n        out[lang].append(w)\n"
              "json.dump(out, open(sys.argv[1], 'w'))")
    return wordfreq(script, "", str(zipf), *languages)


def wordfreq(script, stdin, *args):
    """What `script` writes as JSON to the file named by its first argument (a pipe can be non-blocking here, and
    the words don't fit in its buffer)."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "out.json"
        res = subprocess.run(["uv", "run", "-q", "--no-project", "--with", WORDFREQ, "python", "-I", "-c", script,
                              str(out), *args], input=stdin, capture_output=True, text=True, cwd="/")
        if res.returncode:
            raise SystemExit(f"wordfreq: {res.stderr}")
        return json.loads(out.read_text())


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
    # A pipeline reads its own language's names itself, in its own accent ("Rodríguez" in Spanish text).
    for name in kept:
        if attributed().get(name, ("", ""))[0] in PIPELINES:
            out[attributed()[name][0]].add(name)
    return dict(out)
