"""Generate adversarial texts for differential testing: gen_adversarial.py SEED N VOICE... > pairs.jsonl"""
import json
import random
import sys

seed, n = int(sys.argv[1]), int(sys.argv[2])
voices = sys.argv[3:]
rng = random.Random(seed)

WORDS = (
    "the a of and to in is was he for it with as his on be at by I this had not are but from or have an they which "
    "one you were her all she there would their we him been has when who will more no if out so said what up its about "
    "into than them can only other new some could time these two may then do first any my now such like our over man "
    "me even most made after also did many before must through back years where much your way well down should because "
    "each just those people Mr Mrs Dr St how too little state good very make world still own see men work long get here "
    "between both life being under never day same another know while last might us great old year off come since against "
    "go came right used take three Tchaikovsky Nguyễn Łódź Zürich naïve café façade résumé coöperate Beyoncé São Paulo "
    "iPhone McDonald's O'Brien don't won't it's rock'n'roll e.g. i.e. U.S.A. NASA FBI XIV MCMXCIV Louis XVI Henry VIII "
    "co-operate well-known twenty-one self-esteem C++ C# .NET http://example.com/a?b=c user@example.com #hashtag @user "
    "ABC abc AbC aBc ABCs PhDs 1st 2nd 3rd 4th 21st 101st 1990s '90s"
).split()
NUMBERS = [
    "0", "7", "13", "42", "100", "101", "999", "1000", "1001", "1234", "12345", "123456", "1234567", "12345678",
    "1,234", "12,345", "1,234,567", "1.234", "1.234.567", "3.14", "0.5", ".5", "10.05", "1,5", "12 345", "1 000 000",
    "007", "0001", "00:30", "07:45", "12:00", "1/2", "3/4", "50%", "$5", "$1,000.50", "€20", "£3", "¥100", "5kg", "10km",
    "km²", "m³", "°C", "1e10", "2^10", "1-2", "1990-2000", "2006.", "1.", "2.,", "-5", "+3", "±2", "1234567890123456789012345",
    "99999999999", "2147483647", "2147483648", "4294967296", "1.000,5", "1,000.5", "12.34.56", "1..2", "1...", "3.",
]
PUNCT = list(".,;:!?¡¿-–—…\"'«»“”‘’()[]{}<>/\\|_*&^%$#@~`+=")
SCRIPTS = [
    "abcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "àáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿāăąćčďđēėęěğīįıķĺļľłńņňōőœŕřśşšţťūůűųźżžſ",
    "αβγδεζηθικλμνξοπρστυφχψωάέήίόύώϊϋΐΰ",
    "абвгдеёжзийклмнопрстуфхцчшщъыьэюяАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ",
    "אבגדהוזחטיכלמנסעפצקרשת",
    "ابتثجحخدذرزسشصضطظعغفقكلمنهوي",
    "कखगघचछजझटठडढणतथदधनपफबभमयरलवशषसहािीुूेैोौंः्",
    "অআইঈউঊএঐওঔকখগঘচছজঝটঠডঢণতথদধনপফবভমযরলশষসহ",
    "ㄱㄴㄷ가나다라마바사아자차카타파하한국어",
    "あいうえおかきくけこアイウエオ漢字中文",
    "ԱԲԳԴԵԶԷԸԹԺաբգդեզէըթժ",
    "ႠႡႢაბგდევზ",
    "😀🎉👍🏽❤️✓★☆♪→←↑↓∞≈≠≤≥",
    "̧́̀̈​‌‍­﻿  ",
    "\x01\x02\x03\x07\x08\x0b\x0c\x0e\x1b\x1f\x7f\x85\xa0",
    "0123456789٠١٢٣٤٥٦٧٨٩०१२३४५६७८९",
]
EMBEDDED = ["\x0145S", "\x01B1", "\x01B0", "\x01B.,", "\x01V", "\x01+10P", "\x013Y", "\x01Y", "\x01", "[\x02h@l'oU]]", "[\x02_^_FR b'O~ZuR]]", "[\x02"]
SPACES = [" ", " ", " ", "  ", "\n", "\n\n", "\t", " \n ", "\r\n"]


def token():
    r = rng.random()
    if r < 0.45:
        w = rng.choice(WORDS)
        if rng.random() < 0.1:
            w = w.upper()
        elif rng.random() < 0.1:
            w = w.capitalize()
        return w
    if r < 0.65:
        return rng.choice(NUMBERS)
    if r < 0.8:
        return "".join(rng.choice(PUNCT) for _ in range(rng.randint(1, 3)))
    if r < 0.97:
        s = rng.choice(SCRIPTS)
        return "".join(rng.choice(s) for _ in range(rng.randint(1, 8)))
    return rng.choice(EMBEDDED)


def text():
    r = rng.random()
    if r < 0.05:
        # one long run, no clause breaks
        return "".join(rng.choice(SCRIPTS[0] + " ") for _ in range(rng.randint(700, 1200)))
    if r < 0.08:
        return rng.choice(SCRIPTS[rng.randrange(len(SCRIPTS))]) * rng.randint(50, 200)
    n = rng.randint(1, 40)
    parts = []
    for _ in range(n):
        parts.append(token())
        parts.append(rng.choice(SPACES) if rng.random() < 0.8 else "")
    return "".join(parts)


for _ in range(n):
    t = text()
    t = t.replace("\x00", "")
    print(json.dumps({"voice": rng.choice(voices), "text": t}, ensure_ascii=False))
