"""Score each arm against the key: primary stress on the right vowel, and consonant edit distance."""

import json
import sys

from phon import consonants, edit_distance, stress_index, vowel_count, words


def load(path):
    return {r["name"]: r for r in map(json.loads, open(path))}


def score_arm(key, arm):
    rows = []
    for name, k in key.items():
        a = arm[name]
        kw, aw = words(k["phonemes"]), words(a["phonemes"] or "")
        if len(kw) == 1 and len(aw) > 1:  # "Tar-Aldarion" written as two words
            aw = ["".join(aw)]
        same_words = len(kw) == len(aw)
        stress, cons, cons_len, syll = [], 0, 0, []
        for i, kword in enumerate(kw):
            aword = aw[i] if same_words else ""
            ks = stress_index(kword)
            if ks is not None:
                stress.append(same_words and stress_index(aword) == ks)
            kc = consonants(kword)
            cons += edit_distance(kc, consonants(aword), key_had_x="h" in kc)
            cons_len += len(kc)
            syll.append(same_words and vowel_count(aword) == vowel_count(kword))
        rows.append({"name": name, "key": k["phonemes"], "arm": a["phonemes"], "stress_ok": all(stress) if stress else None,
                     "cons_errors": cons, "cons_len": cons_len, "syllables_ok": all(syll),
                     "invalid": a.get("invalid", [])})
    return rows


def summary(rows):
    scored = [r for r in rows if r["stress_ok"] is not None]
    return {
        "names": len(rows),
        "stress_correct": sum(r["stress_ok"] for r in scored) / len(scored),
        "consonant_error_rate": sum(r["cons_errors"] for r in rows) / sum(r["cons_len"] for r in rows),
        "consonants_exact": sum(r["cons_errors"] == 0 for r in rows) / len(rows),
        "syllable_count_ok": sum(r["syllables_ok"] for r in rows) / len(rows),
        "invalid_symbols": sum(bool(r["invalid"]) for r in rows),
    }


def main(key_path, out, *arms):
    key = load(key_path)
    result = {}
    for spec in arms:
        label, path = spec.split("=", 1)
        rows = score_arm(key, load(path))
        result[label] = {"summary": summary(rows), "rows": rows}
    json.dump(result, open(out, "w"), ensure_ascii=False, indent=1)
    for label, r in result.items():
        print(f"{label:14}", "  ".join(f"{k}={v:.2f}" if isinstance(v, float) else f"{k}={v}" for k, v in r["summary"].items()))


if __name__ == "__main__":
    main(*sys.argv[1:])
