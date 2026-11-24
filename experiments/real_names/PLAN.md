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

- **How many names:** 96,344 Latin spellings get a language. To get one, a spelling needs both:
  - at least 3 bearers in countries of one language, with that language the most common among them;
  - at least 75% of its bearers with a citizenship in countries that speak that language. Countries of several languages count here, so Czechoslovakia counts for a Czech name.

  Belgian-only and Swiss-only names get no language, and neither do names of Russians and other peoples written in scripts this doesn't read. The biggest groups are French 16.5k, German 13.7k, Italian 10.8k, Spanish 9k, Polish 7.9k, Dutch and Norwegian 4.2k each, and Finnish 4.1k.
- **Cost:** a names.json with every language (112k entries, 3.1 MB) adds 0.15 s to startup and 43 MB of memory (0.10 s and 75 MB before, measured with the phonemize example). That's acceptable, so `names.rs` needs no redesign.
- **espeak against Wiktionary:** names whose sounds all match, compared coarsely (vowel quality, length and how each dictionary writes a sound are ignored):

  | Result | Languages |
  |---|---|
  | Good, 85% or more | Finnish 99%, Polish 98%, Czech 93%, Slovak 90%, Spanish 89%, French 88%, Hungarian 87%, Italian 86%, Romanian 85% (60 names) |
  | Middling | Estonian 81%, Indonesian 81%, Albanian 78%, Slovene 76% (no stress marks to compare), Turkish 74% (stress 79%), Azerbaijani (too few names) |
  | Poor, or too few names to tell | German 43%, Portuguese 39%, Dutch 62%, Swedish 24%, Norwegian 31%, Danish 6%, Icelandic 26%, Lithuanian 12% (stress 54%), Vietnamese 38% |

  Some of the poor results are differences in how each dictionary writes sounds (German ɐ and ʁ, Portuguese's two standards). Others are real espeak errors. Each language's pilot has to look at its worst cases.
- **espeak errors found so far:**
  - Czech names without a vowel letter (Vlk, Plch) are spelled out, so they're left out.
  - Czech readings miss di→ɟ in some names (Vladislav) and final devoicing (-slav).
  - Hungarian's old noble spellings (Batthyány, Kovách) come out wrong.
  - Lithuanian stress is a guess.
  - espeak sometimes switches to English partway through a word: 1,221 French names and Vietnamese "Baotixita". Those names are left out.
  - French espeak reads some names with no vowel at all (Gees as ʒ, Bres as bʁ). Those are left out too.

## Phase 1 results: pilot of Polish, Finnish and Czech (2026-11-24)

- **names.json:** 20,605 entries, from 5,405. That's 7,881 Polish, 3,777 Finnish and 1,621 Czech readings, before the flags and English words are taken out.
- **Flag pass:** Opus checked 15,869 spellings with `flag.py real`. Every flagged spelling is left out, without review.
  - The first prompt flagged famous people whose names English anglicizes: Räikkönen, Dvořák, Brzeziński, Häkkinen.
  - Leaving a name out makes it worse, not better. espeak's English rules then guess (Räikkönen as ɹˈɛɪkˌɜɹnən), and misaki's dictionaries already hold the anglicizations English really uses.
  - So the prompt now says not to flag famous bearers. Rechecking the 551 flags left 225.
- **Loans:** a name spelled with letters its language doesn't use is left out (`real.LETTERS`). Finland counts as Finnish, so this keeps Swedish names in Finland (Bergholm, Björkenheim) from being read by Finnish rules.
- **Common words:** for Spanish, French, Italian and Portuguese, a name that is a common word in that language (wordfreq zipf 3 or more) is read as the word there.
- **Bug found and fixed:** the names lookup in `names.rs` didn't count IPA letters as letters, so "kluk" inside "[klukɔˈvʲit͡ʂɛ]" was taken for a name. Its link broke the brackets.
- **English Wikipedia check:** in 3,926 paragraphs of 1,899 random English Wikipedia articles, 47 paragraphs change, in 64 words.
  - Almost all of them are Polish, Finnish and Czech people and places.
  - Misread as names: the Bulgarian "Gorna" and the Nepali film title "Woda". Iraq's Rawa and Kufa are read in Polish and Finnish but sound nearly the same.
- **Coverage gap:** many famous people's surnames aren't linked to a Wikidata family-name item. Kieślowski has 1 bearer and Kvitová has no item, so they're missing. A next step could take surnames from people's own native-name labels (P1559), with their citizenship.

## Phase 2 results: people's own names; Spanish, Italian, French and Hungarian (2026-11-24)

- **New source of spellings.** Each person's name as their country writes it replaces Wikidata's name items: the label in the language of a single-language country, otherwise the label for every language (mul) or in English. Each word of the name counts as a bearer.
  - This finds names no name item links. Kieślowski and Błaszczykowski were missing before.
  - A spelling with a letter only one language uses (ś, ł, ř, ů, ő, ű, ğ, ı, þ, ð) settles on a single bearer.
  - The fetch is 9 batches of countries (QLever times out streaming all 6.5M people at once), plus one query per language.
- **Canada** is listed with English and French. Before, French Canadians counted as English speakers, which dropped Thibault and Geneviève. The English-speaking share now counts only English-only countries.
- **Spanish voice:** es-419 unless 75% of a name's Spanish-speaking bearers are Spaniards. Martínez and González read with s; Iñaki and Zapatero keep θ.
- **Hungarian:** old spellings are modernized before espeak reads them (thy$→ti, th→t, final ch→cs, final gh→g, cz→c, aa→á, eö→ö). Batthyány, Horthy, Kovách and Horváth match Wiktionary, and Hungarian goes from 87% to 90%.
- **Spelled-out readings:** espeak spells some words letter by letter (Spanish "Jhon"). A one-word reading with more than one main stress is left out.
- **Pipelines:** a pipeline reads its own language's names itself. Spanish names in Spanish text aren't links, so they keep that pipeline's accent ("Rodríguez").
- **names.rs:** a name after an elision isn't a word of its own ("d'Espagne"). Its link made misaki read the word twice.
- **Size and cost:** names.json has 78,665 names (2.5 MB). Startup is 0.22 s and memory 112 MB, from 0.10 s and 75 MB before any real names.
- **Flag pass:** 74,623 more spellings checked; 2,362 flagged in all.
- **English Wikipedia** (the same 3,926 paragraphs): 293 change, in 328 distinct words, almost all of them correct (Martínez, Jean-Luc Godard, Miklós Rózsa, Giuseppe). Places that are also surnames are read natively too (Cádiz, Troyes).
- **Filed:** issue #79. In the espeak pipelines, the word before a name's link is read out of context ("e" in Portuguese as the letter ɛ).

- **Brendan's decision (2026-11-24):** names that pass the cutoff but that English text usually uses for English speakers or brands (Giuliani, Rubio, Pujols, Sotomayor, Peugeot, Pirelli) keep their native readings. He found them "about as good or better than the default". The listening page has them.

## Phase 3 results: six more languages (2026-11-24)

- **Languages:** Romanian, Slovak, Slovenian, Estonian, Turkish and Indonesian. names.json has 105,181 names with readings, up from 79,812.
- **Romanian:** espeak knows ș and ț only with a cedilla, so they're rewritten before it reads them. That takes Romanian from 70% to 84% exact.
- **Turkish:** espeak writes ü, ö and e as ø, œ and æ, and those are mapped back. That takes Turkish from 75% to 86%.
- **Indonesian:** Dutch-era spellings are modernized (oe, dj, tj, sj, ch, and a lone j in such names), and espeak's ç for sy becomes ʃ. That takes Indonesian from 85% to 88%.
- **Slovenian** is at 75%: espeak turns l before a consonant into w more often than Slovenian does. It's included as a mild error.
- **Held back:** German (63% exact even after mapping -er's ɜ to ɐ, with compounds and Latin names wrong) and Dutch (52%, with stress and "ph" errors). misaki's English guesses for German names are often passable, so a wrong native reading would be a step back. Both wait for hand-written rules, along with Portuguese, the Scandinavian languages, pinyin, Japanese romanization and Vietnamese.
- **English Wikipedia check** (the same 3,926 paragraphs): 37 more paragraphs change, almost all correctly (Barış Alper Yılmaz, Mihkel, Bambang Yudhoyono, Çankırı). The Kurdish "Gormez" is read as Turkish Görmez.
- **Issue #79 is fixed in #82.** The text around a link is read whole with a stand-in name.

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
