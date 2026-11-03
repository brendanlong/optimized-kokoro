"""Export en_core_web_sm's tokenizer data and tok2vec + tagger weights for the Rust crate.

Run with the reference venv's python (see reference/README.md):

    reference/.cache/kfa/.venv/bin/python -I crates/spacy-en/export.py crates/spacy-en/data

Writes `tokenizer.json` (patterns, special cases, norm tables, Python's Unicode predicates) and
`tagger.safetensors` (float32 weights). Regexes are re-emitted from Python's own parse tree, so
character classes, `\\w`/`\\d`/`\\s` and escapes reach Rust as explicit code point ranges.
"""

import json
import re
import sys
from pathlib import Path
from re import _constants as C
from re import _parser as sre

import numpy as np
import spacy
import spacy.lang.lex_attrs
import thinc
from safetensors.numpy import save_file
from spacy.attrs import NORM
from spacy.lang.norm_exceptions import BASE_NORMS
from spacy.symbols import IDS
from spacy.tokenizer import Tokenizer

MAX = 0x10FFFF
SURROGATES = (0xD800, 0xDFFF)


def ranges(pred):
    out = []
    for cp in range(MAX + 1):
        if SURROGATES[0] <= cp <= SURROGATES[1] or not pred(chr(cp)):
            continue
        if out and out[-1][1] == cp - 1:
            out[-1][1] = cp
        else:
            out.append([cp, cp])
    return out


def complement(rs):
    out, lo = [], 0
    for a, b in rs + [[MAX + 1, MAX + 1]]:
        if lo < a:
            out.append([lo, a - 1])
        lo = b + 1
    return out


def without_surrogates(lo, hi):
    if hi < SURROGATES[0] or lo > SURROGATES[1]:
        return [[lo, hi]]
    return [r for r in ([lo, SURROGATES[0] - 1], [SURROGATES[1] + 1, hi]) if r[0] <= r[1]]


CATEGORIES = {
    name: ranges(lambda c, p=re.compile(pattern): p.match(c) is not None)
    for name, pattern in [
        (C.CATEGORY_DIGIT, r"\d"),
        (C.CATEGORY_NOT_DIGIT, r"\D"),
        (C.CATEGORY_SPACE, r"\s"),
        (C.CATEGORY_NOT_SPACE, r"\S"),
        (C.CATEGORY_WORD, r"\w"),
        (C.CATEGORY_NOT_WORD, r"\W"),
    ]
}


def char(cp):
    c = chr(cp)
    return c if c.isascii() and c.isalnum() else f"\\x{{{cp:X}}}"


def emit_class(items):
    negate = False
    rs = []
    for op, av in items:
        if op is C.NEGATE:
            negate = True
        elif op is C.LITERAL:
            rs.extend(without_surrogates(av, av))
        elif op is C.RANGE:
            rs.extend(without_surrogates(*av))
        elif op is C.CATEGORY:
            rs.extend(CATEGORIES[av])
        else:
            raise ValueError(f"unsupported class item {op}")
    body = "".join(char(a) if a == b else f"{char(a)}-{char(b)}" for a, b in rs)
    return f"[{'^' if negate else ''}{body}]"


def emit(items, asserts=True):
    out = []
    for op, av in items:
        if op is C.LITERAL:
            out.append(char(av))
        elif op is C.IN:
            out.append(emit_class(av))
        elif op is C.BRANCH:
            out.append("(?:" + "|".join(emit(b, asserts) for b in av[1]) + ")")
        elif op is C.SUBPATTERN:
            _group, add_flags, del_flags, p = av
            if add_flags & ~re.UNICODE or del_flags:
                raise ValueError(f"unsupported inline flags {add_flags} {del_flags}")
            out.append("(?:" + emit(p, asserts) + ")")
        elif op in (C.MAX_REPEAT, C.MIN_REPEAT):
            lo, hi, p = av
            rep = f"{{{lo},}}" if hi is C.MAXREPEAT else f"{{{lo},{hi}}}"
            out.append("(?:" + emit(p, asserts) + ")" + rep + ("?" if op is C.MIN_REPEAT else ""))
        elif op in (C.ASSERT, C.ASSERT_NOT):
            if not asserts:
                continue
            direction, p = av
            kind = ("=" if op is C.ASSERT else "!") if direction == 1 else ("<=" if op is C.ASSERT else "<!")
            out.append(f"(?{kind}" + emit(p, asserts) + ")")
        elif op is C.AT and av is C.AT_BEGINNING:
            out.append("^")
        elif op is C.AT and av is C.AT_END:
            # Python's `$` also matches before a final "\n"; spans never end in one (checked below).
            out.append("$")
        else:
            raise ValueError(f"unsupported regex node {op} {av}")
    return "".join(out)


def translate(compiled):
    """The pattern, and a version without lookarounds: a superset the plain `regex` engine can
    run, to skip the backtracking one where nothing can match."""
    if compiled.flags & ~re.UNICODE:
        raise ValueError(f"unsupported flags {compiled.flags}")
    tree = sre.parse(compiled.pattern, compiled.flags)
    return {"pattern": emit(tree), "filter": emit(tree, asserts=False)}


def check_whitespace_never_matches(tok):
    # Spans are runs of either whitespace or non-whitespace, so a "\n" can only end a span of
    # whitespace. If no affix pattern can match whitespace, `$` before a final "\n" never fires.
    for c in (chr(cp) for cp in range(MAX + 1) if chr(cp).isspace()):
        for s in (c, c * 3, c + "\n", "\n" + c):
            assert tok.prefix_search(s) is None, repr(s)
            assert tok.suffix_search(s) is None, repr(s)
            assert not list(tok.infix_finditer(s)), repr(s)
            assert tok.url_match(s) is None, repr(s)


def special_cases(nlp):
    tok = nlp.tokenizer
    cases = {}
    for chunk, substrings in sorted(tok.rules.items()):
        tokens = []
        for spec in substrings:
            spec = spacy.attrs.intify_attrs(spec, _do_deprecated=True)
            assert set(spec) <= {spacy.attrs.ORTH, NORM}, spec
            tokens.append([spec[spacy.attrs.ORTH], spec.get(NORM)])
        assert "".join(t[0] for t in tokens) == chunk
        cases[chunk] = tokens
    # Rules the tokenizer re-applies after tokenizing, with a PhraseMatcher over token texts: the
    # rule's own text tokenized without special cases (Tokenizer.add_special_case).
    assert tok.faster_heuristics
    plain = Tokenizer(
        nlp.vocab,
        rules={},
        prefix_search=tok.prefix_search,
        suffix_search=tok.suffix_search,
        infix_finditer=tok.infix_finditer,
        token_match=tok.token_match,
        url_match=tok.url_match,
    )
    phrases = []
    for chunk in sorted(tok.rules):
        if tok.find_prefix(chunk) or tok.find_infix(chunk) or tok.find_suffix(chunk) or " " in chunk:
            phrases.append([t.text for t in plain(chunk)])
    return cases, phrases


def lexeme_norms(nlp):
    """The NORM getter's exceptions, in the order it checks them: the model's `lexeme_norm` table
    (keyed by string id, so exported as [id, norm] pairs), then spaCy's BASE_NORMS, then lower()."""
    table = nlp.vocab.lookups.get_table("lexeme_norm")
    getter = nlp.vocab.lex_attr_getters[NORM]
    inner = getter.args[0]
    assert getter.args[1] == (table,) and inner.args[1] == (BASE_NORMS,), getter
    assert inner.args[0] is spacy.lang.lex_attrs.lower, inner
    # Lexemes created while loading keep whatever NORM they got then; make sure that's the same.
    assert all(lex.norm_ == getter(lex.orth_) for lex in nlp.vocab)
    return [[k, v] for k, v in table.items()], BASE_NORMS


def lower_table():
    table = {}
    for cp in range(MAX + 1):
        c = chr(cp)
        if not SURROGATES[0] <= cp <= SURROGATES[1] and c.lower() != c:
            table[cp] = c.lower()
    return sorted(table.items())


def weights(nlp):
    tok2vec = nlp.get_pipe("tok2vec").model
    tagger = nlp.get_pipe("tagger").model
    tensors = {}
    meta = {"embed_seeds": [], "embed_columns": []}
    embeds = [n for n in tok2vec.walk() if n.name == "hashembed"]
    for i, node in enumerate(embeds):
        tensors[f"embed.{i}.E"] = node.get_param("E")
        meta["embed_seeds"].append(node.attrs["seed"])
        meta["embed_columns"].append(node.attrs["column"])
    columns = next(n for n in tok2vec.walk() if n.name == "extract_features").attrs["columns"]
    meta["features"] = list(columns)
    maxouts = [n for n in tok2vec.walk() if n.name == "maxout"]
    norms = [n for n in tok2vec.walk() if n.name == "layernorm"]
    assert len(maxouts) == len(norms) == 5
    for i, (mx, ln) in enumerate(zip(maxouts, norms)):
        name = "mix" if i == 0 else f"encode.{i - 1}"
        nO, nP, nI = mx.get_param("W").shape
        tensors[f"{name}.W"] = mx.get_param("W").reshape(nO * nP, nI)
        tensors[f"{name}.b"] = mx.get_param("b").reshape(nO * nP)
        tensors[f"{name}.G"] = ln.get_param("G")
        tensors[f"{name}.bias"] = ln.get_param("b")
    encoder = next(n for n in tok2vec.walk() if n.name.startswith("with_array(residual"))
    meta["pad"] = encoder.attrs["pad"]
    meta["window"] = next(n for n in tok2vec.walk() if n.name == "expand_window").attrs["window_size"]
    meta["pieces"] = maxouts[0].get_dim("nP")
    softmax = tagger.get_ref("softmax")
    assert not softmax.attrs["softmax_normalize"]
    tensors["tagger.W"] = softmax.get_param("W")
    tensors["tagger.b"] = softmax.get_param("b")
    tensors = {k: np.ascontiguousarray(v, dtype=np.float32) for k, v in tensors.items()}
    return tensors, meta


def main(out_dir):
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    nlp = spacy.load("en_core_web_sm", enable=["tok2vec", "tagger"])
    assert nlp.pipe_names == ["tok2vec", "tagger"], nlp.pipe_names
    assert type(thinc.api.get_current_ops()).__name__ == "NumpyOps"
    tok = nlp.tokenizer
    assert tok.token_match is None
    check_whitespace_never_matches(tok)
    cases, phrases = special_cases(nlp)
    lexeme_norm, base_norms = lexeme_norms(nlp)
    tensors, meta = weights(nlp)
    data = {
        "source": {
            "spacy": spacy.__version__,
            "thinc": thinc.__version__,
            "model": f"{nlp.meta['lang']}_{nlp.meta['name']} {nlp.meta['version']}",
            "python": sys.version.split()[0],
        },
        "prefix": translate(tok.prefix_search.__self__),
        "suffix": translate(tok.suffix_search.__self__),
        "infix": translate(tok.infix_finditer.__self__),
        "url_match": translate(tok.url_match.__self__),
        "special_cases": cases,
        "special_phrases": phrases,
        "lexeme_norm": lexeme_norm,
        "base_norms": base_norms,
        "symbols": {k: v for k, v in IDS.items() if k},
        "tags": list(nlp.get_pipe("tagger").labels),
        "model": meta,
        "unicode": {
            "space": ranges(str.isspace),
            "alpha": ranges(str.isalpha),
            "upper": ranges(str.isupper),
            "digit": ranges(str.isdigit),
            # For str.lower()'s final-sigma rule (CPython's handle_capital_sigma).
            "cased": ranges(lambda c: (c + "Σ").lower()[-1] == "ς"),
            "case_ignorable": ranges(
                lambda c: ("A" + c + "Σ").lower()[-1] == "ς" and (c + "Σ").lower()[-1] != "ς"
            ),
            "lower": lower_table(),
        },
    }
    with open(out / "tokenizer.json", "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, separators=(",", ":"))
    save_file(tensors, str(out / "tagger.safetensors"))


if __name__ == "__main__":
    main(sys.argv[1])
