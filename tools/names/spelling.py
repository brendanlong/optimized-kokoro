"""How the G2P matches text to a name's spelling (crates/g2p-en/src/names.rs): in any case, with some of
the name's accents left out and none added."""

import unicodedata


def clusters(s):
    """Each letter, lowercase, with its accents."""
    out = []
    for c in unicodedata.normalize("NFD", s):
        if unicodedata.category(c).startswith("M") and out:
            out[-1][1].append(c)
        else:
            out.append([c.lower(), []])
    return out


def fold(s):
    """A spelling as the G2P keys it: lowercase, without accents."""
    return "".join(letter for letter, _ in clusters(s))


def accents_within(written, name):
    """Whether `written` is `name` in any case, with some of its accents left out and no others added."""
    a, b = clusters(written), clusters(name)
    return len(a) == len(b) and all(x[0] == y[0] and (not x[1] or x[1] == y[1]) for x, y in zip(a, b))


def marks(s):
    return sum(len(m) for _, m in clusters(s))
