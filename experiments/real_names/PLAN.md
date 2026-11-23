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
1. **Pilot:** Polish, Hungarian and Mandarin pinyin, with a listening page.
2. **Fan out** by language: European Latin-script languages, Turkic languages, Indonesian/Malay, Tagalog, Swahili, Māori and Hawaiian, Vietnamese (no tones), pinyin and Japanese romanization. Each gets an accuracy report, audio samples, and Opus and Sol reviews. Russian, Arabic, Hindi and Korean romanizations are left out: they have competing spellings, and English already has conventions for them.
3. **Places** (Łódź, Szczecin), with English's names for places (Warsaw) left to misaki.
