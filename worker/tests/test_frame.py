"""The batch-first frame side on CPU with random weights: a padded batch gives each sentence what
it gives the sentence alone, and neighbours don't change it. Then how items are split into batches,
and what happens when a batch fails."""

import pytest
import torch

from kokoro_worker import convseq, frame

FRAMES = [5, 9, 3, 7]


@pytest.fixture
def kokoro(monkeypatch):
    from kokoro_worker import model

    model._import_kokoro()
    # The harmonic source's noise, as a function of sample position only, so a sentence gets the same
    # noise batched or alone. (Zero noise would leave empty frequency bins, whose phases are rounding
    # noise that the random gains blow up.)
    def noise(t, **kwargs):
        pos = torch.arange(t.shape[1], dtype=torch.float64)[None, :, None]
        dim = torch.arange(t.shape[2], dtype=torch.float64)[None, None, :]
        return (torch.frac(torch.sin(pos * 12.9898 + dim * 78.233) * 43758.5453) - 0.5).to(t.dtype).expand_as(t)

    monkeypatch.setattr(torch, "randn_like", noise)
    torch.manual_seed(0)
    import kokoro.istftnet as k

    return k


@pytest.fixture
def decoder(kokoro):
    dec = frame.Decoder(kokoro.SourceModuleHnNSF(24000, frame.HOP, harmonic_num=8, voiced_threshod=10)).eval()
    with torch.no_grad():
        for m in dec.modules():
            if isinstance(m, convseq.DepthwiseConv1d):  # left uninitialized: the worker loads weights
                torch.nn.init.normal_(m.weight, std=0.2)
                torch.nn.init.zeros_(m.bias)
        for p in dec.parameters():
            p.mul_(0.5)  # keeps random-weight activations in range
    return dec


@pytest.fixture
def predictor(kokoro):
    from kokoro.modules import ProsodyPredictor

    return ProsodyPredictor(style_dim=128, d_hid=512, nlayers=3, max_dur=50, dropout=0.2).eval()


def _inputs(seed=1):
    torch.manual_seed(seed)
    n, w = len(FRAMES), max(FRAMES)
    asr = torch.randn(n, 512, w)
    f0 = torch.rand(n, 2 * w) * 200 + 50
    f0[:, ::5] = 0  # some unvoiced frames
    energy = torch.randn(n, 2 * w)
    for i, f in enumerate(FRAMES):
        asr[i, :, f:] = 0
        f0[i, 2 * f :] = 0
        energy[i, 2 * f :] = 0
    return asr, f0, energy, torch.randn(n, 128)


@torch.no_grad()
def test_decoder_batch_matches_each_sentence_alone(decoder):
    asr, f0, energy, s = _inputs()
    batch = decoder(asr, f0, energy, s, torch.tensor(FRAMES))
    for i, f in enumerate(FRAMES):
        alone = decoder(asr[i : i + 1, :, :f], f0[i : i + 1, : 2 * f], energy[i : i + 1, : 2 * f], s[i : i + 1],
                        torch.tensor([f]))
        got, want = batch[i, 0, : 600 * f], alone[0, 0]
        assert float((got - want).abs().max()) < 1e-4 * float(want.abs().max())  # float rounding


@torch.no_grad()
def test_neighbours_dont_change_a_sentence(decoder):
    asr, f0, energy, s = _inputs()
    before = decoder(asr, f0, energy, s, torch.tensor(FRAMES))
    a2, f2, e2, s2 = _inputs(seed=2)
    for i, f in enumerate(FRAMES):
        others = torch.arange(len(FRAMES)) != i
        a, f_, e, st = a2.clone(), f2.clone(), e2.clone(), s2.clone()
        a[i], f_[i], e[i], st[i] = asr[i], f0[i], energy[i], s[i]
        after = decoder(a, f_, e, st, torch.tensor(FRAMES))
        b, a = before[i, 0, : 600 * f], after[i, 0, : 600 * f]
        assert float((b - a).abs().max()) < 1e-4 * float(b.abs().max())
        j = int(others.nonzero()[0])
        assert not torch.allclose(before[j, 0, : 600 * FRAMES[j]], after[j, 0, : 600 * FRAMES[j]], atol=1e-3)


@torch.no_grad()
def test_f0n_batch_matches_each_sentence_alone(predictor):
    torch.manual_seed(3)
    n, w = len(FRAMES), max(FRAMES)
    x, s = torch.randn(n, 512, w), torch.randn(n, 128)
    m_f = (torch.arange(w)[None, :] < torch.tensor(FRAMES)[:, None]).float()[:, None, :]
    x = x * m_f
    f0, energy = frame.f0n(predictor, x, s, m_f, m_f.repeat_interleave(2, dim=2))
    for i, f in enumerate(FRAMES):
        one = torch.ones(1, 1, f)
        a0, a1 = frame.f0n(predictor, x[i : i + 1, :, :f], s[i : i + 1], one, torch.ones(1, 1, 2 * f))
        assert torch.allclose(f0[i, : 2 * f], a0[0], atol=1e-5) and torch.allclose(energy[i, : 2 * f], a1[0], atol=1e-5)
        assert not f0[i, 2 * f :].any()


def _rows(frames, waiting, budget=100):
    from kokoro_worker.model import Synth

    synth = Synth.__new__(Synth)
    synth.frame_budget = budget
    return synth._rows(frames, waiting)


def test_rows_fill_to_the_frame_budget_padded():
    assert _rows([30, 30, 30, 30], [True] * 4, budget=90) == [[0, 1, 2], [3]]
    assert _rows([30, 30, 30, 30], [False] * 4, budget=90) == [[0, 1, 2], [3]]
    # a long item counts for every item it's padded with
    assert _rows([10, 40, 10], [True] * 3, budget=90) == [[0, 1], [2]]
    assert _rows([200, 10], [True, True]) == [[0], [1]]  # a long one still gets a batch


def test_waiting_chunks_dont_share_a_row_with_prefetch_after_them():
    # most urgent first: waiting, waiting, then prefetch
    assert _rows([10, 10, 10, 10], [True, True, False, False]) == [[0, 1], [2, 3]]
    # a more urgent prefetch, then less urgent waiting chunks: rows stay in order, and only a
    # prefetch after a waiting chunk starts a row
    assert _rows([10, 10, 10, 10], [False, True, False, True]) == [[0, 1], [2, 3]]
    assert _rows([10, 10, 10], [False, False, True]) == [[0, 1, 2]]
    # the budget still splits waiting chunks
    assert _rows([40, 40, 40, 10], [True, True, True, False]) == [[0, 1], [2], [3]]


class _FailingRows:
    """Synth._decode for a stub: a batch (more than one item) fails as `failure` says; alone,
    every item decodes except `bad`, whose audio is non-finite."""

    def __init__(self, failure, bad=None):
        self.failure, self.bad, self.calls = failure, bad, []

    def __call__(self, idx, frames, *args):
        from kokoro_worker.model import Result

        self.calls.append(list(idx))
        if len(idx) > 1 and self.failure == "oom":
            raise torch.OutOfMemoryError("row too big")
        return [Result(i, None, None, "non_finite") if i == self.bad or (len(idx) > 1 and self.failure == "nan")
                else Result(i, bytes([i]) * 2, [f]) for i, f in zip(idx, frames)]


def _contained(decode, idx, frames):
    from kokoro_worker.model import Synth

    synth = Synth.__new__(Synth)
    synth._decode = decode
    return [r for results in synth._decode_contained(idx, frames) for r in results]


@pytest.mark.parametrize("failure", ["oom", "nan"])
def test_a_failed_row_is_decoded_one_at_a_time_and_only_the_culprit_fails(failure):
    decode = _FailingRows(failure, bad=5)
    results = _contained(decode, [3, 5, 7], [10, 20, 30])
    assert decode.calls == [[3, 5, 7], [3], [5], [7]]
    assert [(r.index, r.pcm, r.durations, r.error) for r in results] == [
        (3, b"\x03\x03", [10], None), (5, None, None, "non_finite"), (7, b"\x07\x07", [30], None)]


def test_a_row_that_decodes_is_not_split():
    decode = _FailingRows(None)
    assert [r.index for r in _contained(decode, [4, 6], [10, 20])] == [4, 6]
    assert decode.calls == [[4, 6]]


def test_one_item_out_of_memory_fails_alone():
    def decode(idx, frames, *args):
        raise torch.OutOfMemoryError("too big")

    [result] = _contained(decode, [2], [10])
    assert (result.index, result.pcm, result.error) == (2, None, "out_of_memory")
