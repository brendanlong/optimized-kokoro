#!/usr/bin/env python3
"""Emit adversarial texts at ~5000 and ~20000 chars as JSON lines for adv.rs."""
import json, random, sys

random.seed(1)
T = 20000

def fit(unit, target):
    """Repeat a unit-producing function until `target` chars."""
    out = []
    i = 0
    n = 0
    while True:
        u = unit(i)
        if n + len(u) > target:
            break
        out.append(u); n += len(u); i += 1
    return "".join(out)

def whole(make, target):
    s = make(target)
    return s[:target]

S = {}
def shape(name):
    def deco(f):
        S[name] = f; return f
    return deco

# unit-based shapes
units = {
    "sentences": lambda i: "The cat sat on the mat. ",
    "no punct words": lambda i: "word ",
    "commas": lambda i: "word, ",
    "semicolon clauses": lambda i: "one two three four five six seven eight; ",
    "pause tags tiny": lambda i: "[pause:0.001s] a ",
    "pause tags only": lambda i: "[pause:0.0s]",
    "phoneme links": lambda i: "[word](/wˈɜɹd/) ",
    "phoneme links no space": lambda i: "[w](/w/)",
    "phoneme link long": lambda i: "[" + "a" * 50 + "](/" + "ə" * 50 + "/) ",
    "unclosed link": lambda i: "[a](/b ",
    "unclosed bracket": lambda i: "[",
    "bracket paren": lambda i: "[](/",
    "script switch": lambda i: "a Привет αβ ",
    "script switch nospace": lambda i: "aБα",
    "numbers": lambda i: "1,234 and 5.6 ",
    "big numbers": lambda i: "123456789012345678901234567890 ",
    "long digit run": lambda i: "1",
    "comma digits": lambda i: "1,",
    "dot digits": lambda i: "1.",
    "versions": lambda i: "1.2.3.",
    "money": lambda i: "$1.5 billion ",
    "money multipliers": lambda i: "$1" + " billion" * 3 + " ",
    "multiplier run": lambda i: " hundred",
    "k run": lambda i: "k",
    "times": lambda i: "12:30 pm ",
    "time-ish colons": lambda i: "1:2",
    "phone": lambda i: "+1 (555) 123-4567 ",
    "phone-ish digits": lambda i: "555 ",
    "urls": lambda i: "https://www.example.com/a/b?c=d ",
    "url long path": lambda i: "http://a.com/" if i == 0 else "x",
    "domain dots": lambda i: "a.",
    "domain run com": lambda i: "a" * 300 + ".com ",
    "dash dot run": lambda i: "a-.",
    "emails": lambda i: "a.b@example.com ",
    "at signs": lambda i: "a@",
    "email-ish": lambda i: "a" * 60 + "@" + "b" * 60 + ".",
    "caps words": lambda i: "NASA ",
    "caps run": lambda i: "A",
    "caps digit": lambda i: "AB1",
    "dotted letters": lambda i: "a.b.",
    "dotted acronym": lambda i: "U.S.A. ",
    "dotted caps": lambda i: "A.",
    "acronym nospace": lambda i: "A.B",
    "Mr run": lambda i: "Mr. ",
    "MR caps": lambda i: "MR. A",
    "etc": lambda i: "etc. ",
    "Dr": lambda i: "DR. X ",
    "contractions": lambda i: "they're ",
    "yeah": lambda i: "yeah ",
    "hyphens": lambda i: "qx-",
    "hyphen words": lambda i: "well-known-",
    "dashes": lambda i: "--",
    "apostrophes": lambda i: "'",
    "rock'n'roll": lambda i: "a'",
    "quotes": lambda i: '"',
    "smart quotes": lambda i: "“a” ",
    "parens": lambda i: "(",
    "parens words": lambda i: "(a) ",
    "ellipsis": lambda i: "…",
    "ellipsis word": lambda i: "…a",
    "dots": lambda i: ".",
    "bangs": lambda i: "!",
    "question": lambda i: "?",
    "spaces": lambda i: " a ",
    "multi spaces": lambda i: "a    ",
    "newlines": lambda i: "a\n",
    "blank lines": lambda i: "a\n\n",
    "crlf": lambda i: "\r\n",
    "tabs": lambda i: "\t",
    "nbsp": lambda i: "a ",
    "zero width": lambda i: "a​",
    "combining": lambda i: "á",
    "only combining": lambda i: "́",
    "emoji": lambda i: "😀",
    "emoji words": lambda i: "a😀 ",
    "keycaps": lambda i: "1️⃣",
    "flags": lambda i: "🇺🇸",
    "ZWJ emoji": lambda i: "👨‍👩‍👧",
    "Han": lambda i: "東京",
    "Han + latin": lambda i: "東a",
    "kana": lambda i: "カタカナ",
    "Hangul": lambda i: "안녕 ",
    "Arabic": lambda i: "مرحبا ",
    "Hebrew points": lambda i: "שָׁלוֹם ",
    "Devanagari": lambda i: "नमस्ते ",
    "Thai": lambda i: "สวัสดี",
    "Greek marks": lambda i: "ά",
    "Cyrillic hyphen": lambda i: "Б-",
    "units": lambda i: "5 km ",
    "units glued": lambda i: "5km",
    "percent": lambda i: "5% ",
    "fractions": lambda i: "1/2 ",
    "ordinals": lambda i: "21st ",
    "years": lambda i: "1999 ",
    "plurals (s)": lambda i: "cat(s) ",
    "symbols": lambda i: "& @ # % ^ * + = < > | ~ ",
    "symbol run": lambda i: "&",
    "unknown words": lambda i: "qz%xv " % i if False else f"qz{i:x}v ",
    "long word": lambda i: "a",
    "long unknown words": lambda i: "".join(random.choice("bcdfghjklmnpqrstvwxz") for _ in range(40)) + " ",
    "mixed case word": lambda i: "aB",
    "CJK punct": lambda i: "a。",
    "CJK commas": lambda i: "word，",
    "fullwidth": lambda i: "ａｂｃ ",
    "roman numerals": lambda i: "XIV ",
    "hashtags": lambda i: "#tag ",
    "markdown": lambda i: "**bold** _it_ ",
    "backslashes": lambda i: "\\",
    "control chars": lambda i: "a\x01\x02",
    "surrogate-ish": lambda i: "\U0001d400",
    "RTL marks": lambda i: "a‮",
    "private use": lambda i: "",
}
for k, u in units.items():
    S[k] = (lambda u: lambda t: fit(u, t))(u)

def mix(t):
    parts = ["Mr. Smith paid $1,234.56 for 3 km of U.S.A. cable at 12:30 pm, ", "[word](/wˈɜɹd/) ",
             "Привет мир ", "https://example.com/x?y=1 ", "NASA and the FBI, ", "[pause:0.01s] ",
             "they're well-known; ", "東京 ", "a.b@c.com ", "+1 555 123 4567 ", "1.2.3 ", "“quote” — ",
             "21st 1999 5% 1/2 ", "😀 "]
    return fit(lambda i: random.choice(parts), t)
S["random mix"] = mix

def long_clause_mix(t):
    # long clauses with no sentence end, forcing stitching, with numbers and links
    return fit(lambda i: f"word{i % 7} [x](/ks/) {i} and Привет " , t)
S["long clause mix"] = long_clause_mix

def one_huge_sentence(t):
    return fit(lambda i: random.choice(["alpha ", "beta ", "1,234 ", "NASA ", "U.S. ", "Mr. ", "[a](/ə/) "]), t)
S["one huge sentence"] = one_huge_sentence

sel = sys.argv[1:] or list(S)
for name in sel:
    for target in (T // 4, T):
        print(json.dumps({"name": name, "text": S[name](target)}))
