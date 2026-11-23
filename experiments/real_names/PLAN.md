# Real names in their own languages: plan

Goal: read as many unambiguous real names as possible in their own language's sounds, as `names.json` already does for Tolkien's names. Brendan asked for this on 2026-11-23.

## What's decided

- **Native readings, in the sounds Kokoro has.** Brendan agreed after the listening test (`page.py`, 2026-11-23): "most of these sound fine with the native pronunciation". names.json applies in every pipeline, so a native reading also suits French or Spanish text.
- **Who the bearers are decides whether a spelling is read natively.** Accentless spellings are the main case. Claude proposed this and Brendan said "let's try that". Wikidata keeps a family-name item per spelling ("Nguyễn", 1,242 bearers; "Nguyen", 360). A spelling whose bearers come mostly from countries that speak the name's language is read natively. One whose bearers are mostly elsewhere (Nguyen, Dvorak, Brzezinski) keeps English's reading. Measured 2026-11-23, as the share of bearers in English-speaking countries:

  | Spelling | English-speaking share | Spelling | English-speaking share |
  |---|---|---|---|
  | Nguyễn | 3% | Nguyen | 26% (and many French) |
  | Dvořák | 0% | Dvorak | 43% |
  | Brzeziński | 2% | Brzezinski | 34% |
  | Erdoğan | 0% | Erdogan | 5% (mostly Turkish) |
  | Kowalczyk | 3% | | |

  The same rule covers diaspora names that never had accents (Kowalczyk stays native). It needs no new mechanism: names.json can already list a spelling as "not the name" ("Dior" for "Díor").
- **espeak reads, the list decides.** For most languages, the reading comes from our own espeak port at build time, run in the name's language. The port can load upstream espeak-ng's data for every language. Its output is converted to Kokoro's sounds. The hard part is the list: which spellings are names of one language, and aren't words or anglicized names. Hand-written rules are only for languages where espeak fails: Mandarin pinyin, Japanese romanization, and probably Irish and Danish.

## Phase 0 results (2026-11-23)

The infrastructure is in `tools/names/`; it reads no real names yet, because `real.LANGUAGES` is empty:
- `wikidata.py` writes the committed `real_names.tsv` from QLever (about 20 seconds);
- `countries.tsv` maps countries to languages;
- `real.py` reads names with our espeak port on espeak 1.52's full data;
- `sounds.py` maps espeak's sounds to Kokoro's;
- `evaluate.py` compares the readings with Wiktionary.

- **How many names:** 111,988 Latin spellings with 3 or more bearers whose country is known, at least 75% of them from one language's countries. Leaving out English and the spellings misaki already has, about 107,000 are other languages' names. The biggest groups are French 20k, German 17k, Italian 12k, Spanish 9k, Polish 8.5k and Dutch 5.7k.
- **Cost:** a names.json with every language (112k entries, 3.1 MB) adds 0.15 s to startup and 43 MB of memory (0.10 s and 75 MB before, measured with the phonemize example). That's acceptable, so `names.rs` needs no redesign.
- **espeak against Wiktionary:** names whose sounds all match, compared coarsely (vowel quality, length and how each dictionary writes a sound are ignored):

  | Result | Languages |
  |---|---|
  | Good, 85% or more | Finnish 99%, Polish 97%, Slovak 90%, Spanish 89%, Czech 88%, French 88%, Hungarian 87%, Italian 86%, Romanian 85% (66 names) |
  | Middling | Estonian 81%, Indonesian 81%, Albanian 78%, Slovene 77% (no stress marks to compare), Turkish 74% (stress 79%), Azerbaijani 73% |
  | Poor, or too few names to tell | German 43%, Portuguese 39%, Dutch 62%, Swedish 24%, Norwegian 31%, Danish 6%, Icelandic 26%, Lithuanian 12% (stress 54%), Vietnamese 38% |

  Some of the poor results are differences in how each dictionary writes sounds (German ɐ and ʁ, Portuguese's two standards). Others are real espeak errors. Each language's pilot has to look at its worst cases.
- **espeak errors found so far:**
  - Czech names without a vowel letter (Vlk, Plch) are spelled out, so they're left out.
  - Czech readings miss di→ɟ in some names (Vladislav) and final devoicing (-slav).
  - Hungarian's old noble spellings (Batthyány, Kovách) come out wrong.
  - Lithuanian stress is a guess.
  - espeak sometimes switches to English partway through a word: 1,221 French names and Vietnamese "Baotixita". Those names are left out.

## Open questions (Claude's proposals)

- **Thresholds.** What share of a spelling's bearers must come from its language's countries, and the minimum number of bearers to trust that share.
- **Dialect by country.** For example, Latin American Spanish (no θ) for bearers from Mexico.

## Phases

0. **Infrastructure:**
   - cached, pinned Wikidata queries (QLever): family and given names, their spellings, and their bearers' citizenship;
   - a map from country to language;
   - a build-time espeak harness, converting its output to Kokoro's sounds;
   - a check of readings against Wiktionary IPA, for measuring only (Wiktionary is CC BY-SA, so it isn't shipped);
   - generalizing `tools/names/build.py` for real-language sources;
   - extending the common-words check to English and every pipeline;
   - measuring `names.rs` startup time and memory at 100k+ entries.
1. **Pilot:** Polish, Finnish and Czech, which espeak reads best, with a listening page. Each needs:
   - the flag pass (`flag.py real`, with the prompt for real names), and Claude's review of the flags;
   - automatic common-word exclusions from wordfreq. For Spanish, French, Italian and Portuguese, these are the names that are common words in that language's text. For English, they're any wordfreq word the misaki dictionaries miss.

   Mandarin pinyin needs hand-written rules, so it moves to Phase 2.
2. **Fan out** by language: European Latin-script languages, Turkic languages, Indonesian/Malay, Tagalog, Swahili, Māori and Hawaiian, Vietnamese (no tones), pinyin and Japanese romanization. Each gets an accuracy report, audio samples, and Opus and Sol reviews. Russian, Arabic, Hindi and Korean romanizations are left out: they have competing spellings, and English already has conventions for them.
3. **Places** (Łódź, Szczecin), with English's names for places (Warsaw) left to misaki.
