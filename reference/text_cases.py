"""Writes fixtures/text_cases.jsonl, the hand-written cases dump_text.py records.

Usage (from reference/): python3 text_cases.py fixtures/text_cases.jsonl
"""

import json
import sys

normalize = [
    # URLs and emails
    "Visit https://www.example.com/path/to/page?query=1&b=2#frag now.",
    "Go to http://localhost:8080/api/v1 or HTTP://EXAMPLE.ORG.",
    "See example.co.uk, github.io and foo-bar_baz.net/x_y.",
    "Server at 192.168.1.254:3000/status is down.",
    "The file is at www.site.info/a%20b=c.",
    "Email me: john.smith+news@mail.example.com, or JANE@EXAMPLE.ORG.",
    "Contact a@b.co or x.y@z.museum today.",
    "An email-like thing: user@localhost isn't one.",
    "Version 1.2.3 of numpy.io came out.",
    "Trailing dot www.example.com. Then more.",
    "Ports: example.com:443/ and example.com:80",
    "Weird: https://example.com:8080?x=1:2",
    "A long domain " + "a" * 300 + ".com here.",
    # phones
    "Call (555) 123-4567 or 555.123.4567 or +44 20 7946 0958.",
    "Dial 1-800-555-0199 or +1 (800) 555 0199.",
    "Numbers: 123 456 7890 and 12 345 678 9012.",
    "Arabic digits: ٥٥٥-١٢٣-٤٥٦٧.",
    # units (only with unit_normalization)
    "It weighs 5kg and is 10 cm long, runs at 3.5 GHz with 16GB RAM.",
    "Speed 60 mph, 100 km/h, 9.8 m/s, -40 °C, 98.6°F, 300 K.",
    "Use 1 m, 0 m, 1.0 m, 1,5 s, 1,000 m, +3 V and 2 MΩ, 4 kΩ, 5 Ω.",
    "Files of 1 B, 1 b, 2 KB, 3 kb, 4 Mbps, 5 MBps, 6 TB and 7 pb.",
    "A 5 µF and a 6 μF capacitor, 7 nF, 8 pF, 9 mi/h, 10 ft/s.",
    "Times: 5 min, 2 h, 30 s, 100 ms; 5mins; 3hrs.",
    "Glued: x5km, 5kmx, 5 km., 5.5.5 km, 1,234.5 kg.",
    # money
    "It cost $5, $5.50, $0.99, $1, $1.01, $1,000,000 and $2.5 million.",
    "Prices: £3.20, €1.05 million, $5k, $3b, $7t, $1.5 trillion, -$5.",
    "Big: $99999999999999999999999 and $" + "9" * 400 + ".",
    "Tiny: $0.00001 and $0.1 and $.5 and $5.",
    "Glued money: $5abc, $5.5.5, $5,5, ab$5.",
    # numbers
    "The years 1984, 2000, 2005, 2010, 1066, 1500, 1501, 9999 and 10000.",
    "Ranges: 1-2, 10-20-30, pages 100-150.",
    "Decimals: 3.14, 0.001, .5, 1.50, 100.0, 2.0000001.",
    "Negative: -5, -0.5, -1,000, x-5.",
    "Commas: 1,000, 1,23, 12,345,678, 1,2,3, 1,000,00.",
    "Multipliers: 5k, 5 thousand, 3m, 2b, 7 million, 1.5 billion, 4 quadrillion, 5km.",
    "Huge: 123456789012345678901234567890123456 and 1234567890123456789012345678901234567.",
    "Floats: 12345678901234567.5, 0.1234567890123456789, 1e5.",
    "Ordinals stay: 1st 2nd 3rd 4th 21st 101st.",
    "Plural years: the 1990s and 80s and 1990S.",
    "Model 747, B12, X-15, F-16s, COVID-19, 4x4, 3D, H2O.",
    "Unicode: ٣ apples, ١٢٣٤ items, １２３ things, ۱۹۸۴.",
    "Versions: 2.0.1, v3.10.2, 1.2.3.4.5, 01.02.03.",
    "Mixed: 5.5.5.5, 1.2.3-beta, 3.2.",
    # times
    "Meet at 9:30am, 10:00, 12:05 PM, 3:00 pm and 23:59:59.",
    "Times 1:01:01, 0:00, 7:7, 11 : 30, 5 :45 am.",
    "Ratio 3:2 and 10:15:20 and 123:45.",
    # caps
    "THE QUICK BROWN FOX JUMPS. NASA AND THE FBI. HELLO.",
    "World War II and Louis XIV and MCMLXXXIV ARE ROMAN.",
    "BREAKING NEWS: SCIENTISTS FIND WATER ON MARS-2 TODAY",
    "An ACRONYM-HEAVY TEXT, LIKE NVIDIA'S GPUs, IS HARD.",
    "Mixed CAPS here AND THERE, ok? ABCDEF ghi.",
    "UNESCO alone, and IBM alone.",
    # contractions and words
    "How're you? What’re they? Where're we? Theyre here. Youre right. Whore? Were.",
    "Yeah, yea, YEAH, Yeah! yeahs.",
    "Dr. Who and DR. Smith; Mr. T and MR. Jones; Ms. Lee, MS. Lee; Mrs. Doe, MRS. Doe; etc. and etc. The end.",
    "dr. no, Dr.No, DR.X",
    # symbols and punctuation
    "A--B, A -- B, A---B, 50% off, A&B, 2+2=4, a/b, a\\b, a|b, x^2, ~5, *bold*, snake_case, #1, @home.",
    "Quotes: ‘single’, “double”, «guillemets», 'plain', \"plain\".",
    "CJK punct：你好，世界！好吗？是的。一、二；三",
    "Ellipsis…then, wait…… and … here…!",
    "Tabs\tand nbsp line sep\x1cfile sep　ideographic.",
    "Optional (s) plural: item(s) and file(s).",
    "Acronyms: U.S.A. and e.g. this, i.e. that, a.b.c.d.e.f.g.h.i.j.k.l.m.n done. U.K.",
    "P.M. A.M. p.m. at 5.",
    # unicode edge cases for Python's classes
    "İstanbul has 5İ and ıx; ſtreet 5K kelvin.",
    "Combining é 5 and ²5 and ½ and 5² and x²5.",
    "Superscript ³4 and 4³, fractions ¾.",
    "Greek ΣΑΣ and final sigma ΟΔΟΣ, German STRASSE ẞ.",
    # annotations
    "[Kokoro](/kˈOkəɹO/) costs $5 and [NASA](/nˈæsə/) is BIG.",
    "Broken [annotation](/x and [x]](/y/) and [](/z/).",
    "Nested [a [b](/c/)](/d/) here.",
    # paragraphs and lines
    "First line\nsecond line\n\nNew paragraph\n\n\n  Third.\r\n\r\nFourth!\n\n\"Quoted\"\n\n)",
    "Ends with colon:\n\nNext",
    "Only\n \n \nspaces",
    # sentences
    "Mr. Smith went to Washington. He said \"Hi!\" Then left... Really? Yes!!",
    "e.g. this. U.S. citizens. 3.5 apples. Version 2. Done",
    "",
    "   ",
    "!!!",
    # emoji
    "I love it 😀👍🏽 so much 🇺🇸 and 1️⃣ #️⃣ *️⃣ 👨‍👩‍👧 ok.",
    # pause tags (normalizer sees them; smart_split handles them before)
    "Wait [pause:1.5s] then go.",
]

languages = [
    ("e", "El 5 de mayo cuesta $5. ¿Dónde?"),
    ("f", "Le 5 mai coûte 5 €. Où?"),
    ("z", "我们今天去北京，明天去上海。这本书花了25元。"),
    ("j", "東京は日本の首都です。今日はいい天気ですね。"),
    ("h", "तेज़ भूरी लोमड़ी। पुस्तकालय कहाँ है?"),
    ("b", "The colour costs £5 at 10:30."),
]

smart_split = [
    ("One. Two, three; four: five. [pause:1s] Six. [PAUSE:0.25s][pause:0s] Seven 😀.", 20),
    ("A very long sentence, with many clauses, separated by commas; and semicolons: and colons, until the end.", 30),
    ("[Hi!](/hˈI/) said the man. Then, after a while, he left.", 15),
    ("Clause one，clause two、clause three；clause four：end.", 12),
    ("No punctuation in this very long sentence that goes on and on and on forever", 10),
    ("[pause:2s]", 450),
    ("[pause:100s] Long pause.", 450),
    (",,,;;; ...", 450),
    ("Hello\n\nWorld\nagain", 450),
    ("Price $5.50, time 10:30, call 555-123-4567.", 25),
]


out = []
for i, t in enumerate(normalize):
    for o in ["default", "all", "none", "off"]:
        out.append({"kind": "normalize", "id": f"normalize-{i}-{o}", "text": t, "options": o})
for i, (lang, t) in enumerate(languages):
    out.append({"kind": "normalize", "id": f"lang-{lang}-{i}", "text": t, "lang": lang})
for i, (t, n) in enumerate(smart_split):
    for o in ["default", "all"]:
        out.append({"kind": "smart_split", "id": f"split-{i}-{o}", "text": t, "options": o, "max_tokens": n})
for t in ["[voice:af_sky]Hi[voice:af_sky] there", "[rate:2]fast[rate:2]still", "x [baserate:2][rate:3] y",
          "[voice:af_sky(2)+af_bella]mixed", "[voice:   ]", "[pause:301s]", "[pause:60s]" * 5 + "[pause:0.1s]"]:
    out.append({"kind": "voices", "id": f"voices-{t}", "text": t, "voice": "af_heart"})
    out.append({"kind": "checks", "id": f"checks-{t}", "text": t, "allow_voice_tags": True, "remove_emoji": True})
with open(sys.argv[1], "w") as f:
    for case in out:
        f.write(json.dumps(case, ensure_ascii=False) + "\n")
