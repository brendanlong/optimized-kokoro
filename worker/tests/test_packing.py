"""Packing on CPU, with a random-weight decoder for the parts that need one. Packed against unpacked
audio isn't compared here: with random weights the audio is too sensitive (a 1e-6 change in F0
moves samples by 0.5); bench/check_packing.py compares them on the real model."""

import pytest
import torch
import torch.nn.functional as F

from kokoro_worker import masked


def _rows(frames, c, scale, extra):
    return [torch.randn(1, c, f * scale + extra, dtype=torch.float64) for f in frames]


def _packed(pk, rows, key):
    r, extra = masked.RATES[key]
    out = torch.zeros(1, rows[0].shape[1], r * pk.total + extra, dtype=torch.float64)
    for row, s in zip(rows, pk.starts):
        out[..., s * r : s * r + row.shape[-1]] = row
    return out


def test_layout():
    pk = masked.Packing([3, 5, 1], "cpu", torch.float32)
    assert pk.starts == [0, 3 + masked.GAP, 8 + 2 * masked.GAP]
    assert pk.total == 9 + 3 * masked.GAP
    for key, (r, extra) in masked.RATES.items():
        m = pk.tensors["m_" + key][0, 0]
        assert m.shape[0] == r * pk.total + extra
        assert int(m.sum()) == sum(f * r + extra for f in [3, 5, 1])


def test_pack_puts_rows_in_place_and_zeros_the_gaps():
    frames = [3, 5, 1]
    pk = masked.Packing(frames, "cpu", torch.float32)
    x = torch.randn(3, 4, 5)
    packed = pk.pack(x, 5)[0]
    for i, (s, f) in enumerate(zip(pk.starts, frames)):
        assert torch.equal(packed[:, s : s + f], x[i, :, :f])
    assert int((packed.abs().sum(0) > 0).sum()) == sum(frames)


def test_segment_statistics_and_expand_match_each_row_alone():
    frames = [3, 5, 1]
    pk = masked.Packing(frames, "cpu", torch.float64)
    t = pk.tensors
    for key, (r, extra) in masked.RATES.items():
        rows = _rows(frames, 4, r, extra)
        x = _packed(pk, rows, key)
        sums = masked._segment_sum(t, key, x)
        for i, row in enumerate(rows):
            assert torch.allclose(sums[0, :, i], row[0].sum(-1))
        v = torch.randn(1, 4, len(frames), dtype=torch.float64)
        expanded = masked._expand(t, key, v) * t["m_" + key]
        for i, (row, s) in enumerate(zip(rows, pk.starts)):
            got = expanded[..., s * r : s * r + row.shape[-1]]
            assert torch.equal(got, v[..., i : i + 1].expand_as(got))


@torch.no_grad()
def test_adain_matches_each_row_alone():
    torch.manual_seed(0)
    frames = [4, 7, 2]
    pk = masked.Packing(frames, "cpu", torch.float64)
    mod = torch.nn.Module()
    mod.fc = torch.nn.Linear(8, 2 * 6).double()
    mod.norm = torch.nn.InstanceNorm1d(6, affine=True).double()
    with torch.no_grad():
        mod.norm.weight.uniform_(0.5, 1.5)
        mod.norm.bias.uniform_(-0.5, 0.5)
    s = torch.randn(len(frames), 8, dtype=torch.float64)
    for key, (r, extra) in masked.RATES.items():
        rows = _rows(frames, 6, r, extra)
        y = masked._adain(mod, _packed(pk, rows, key), s, pk.tensors, key)
        for i, (row, st) in enumerate(zip(rows, pk.starts)):
            gamma, beta = mod.fc(s[i : i + 1])[:, :, None].chunk(2, dim=1)
            want = (1 + gamma) * mod.norm(row) + beta
            assert torch.allclose(y[..., st * r : st * r + row.shape[-1]].double(), want, atol=1e-5)
        gap = y * (1 - pk.tensors["m_" + key])
        assert float(gap.abs().max()) == 0


@pytest.fixture
def decoder(monkeypatch):
    """A random-weight Kokoro decoder (Kokoro's shapes), with the excitation noise off and the
    harmonic phase summed in float64 as the packed path does."""
    from kokoro_worker import model
    from kokoro_worker.kernels import MatmulSTFT

    model._import_kokoro()
    from kokoro.istftnet import Decoder, SineGen

    def f02sine(self, f0_values):
        rad = (f0_values / self.sampling_rate) % 1
        phase = (torch.cumsum(rad.double(), dim=1).float() * 2 * torch.pi).transpose(1, 2)
        phase = F.interpolate(phase * self.upsample_scale, scale_factor=self.upsample_scale, mode="linear")
        return torch.sin(phase.transpose(1, 2))

    monkeypatch.setattr(torch, "randn_like", torch.zeros_like)
    monkeypatch.setattr(SineGen, "_f02sine", f02sine)
    torch.manual_seed(0)
    dec = Decoder(512, 128, 80, [3, 7, 11], [10, 6], 512, [[1, 3, 5]] * 3, [20, 12], 20, 5).eval()
    dec.generator.stft = MatmulSTFT(20, 5)
    return dec


FRAMES = [5, 9, 3, 7]


def _inputs():
    torch.manual_seed(1)
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


def _row(pk, x):
    out = torch.zeros(1, 2 * pk.total)
    for i, (s, f) in enumerate(zip(pk.starts, FRAMES)):
        out[0, 2 * s : 2 * (s + f)] = x[i, : 2 * f]
    return out


@torch.no_grad()
def test_harmonic_source_matches_each_sentence_alone(decoder):
    _, f0, _, _ = _inputs()
    pk = masked.Packing(FRAMES, "cpu", torch.float32)
    packed = masked._source(decoder.generator.m_source, _row(pk, f0), pk.tensors)
    for i, (s, f) in enumerate(zip(pk.starts, FRAMES)):
        alone = decoder.generator.m_source(f0[i : i + 1, : 2 * f, None])[0, :, 0]
        assert torch.allclose(packed[0, 600 * s : 600 * (s + f)], alone, atol=1e-4)


@torch.no_grad()
def test_neighbours_dont_change_a_sentence(decoder):
    # Each sentence keeps its place in the row while everything else in the row changes: its audio
    # must not change at all.
    asr, f0, energy, s = _inputs()
    pk = masked.Packing(FRAMES, "cpu", torch.float32)

    def run(asr, f0, energy, s):
        audio = masked.decoder(decoder, pk.pack(asr, max(FRAMES)), _row(pk, f0), _row(pk, energy), s, pk.tensors)
        return pk.unpack_audio(audio)

    before = run(asr, f0, energy, s)
    for i in range(len(FRAMES)):
        others = torch.arange(len(FRAMES)) != i
        a2, f2, e2, s2 = asr.clone(), f0.clone(), energy.clone(), s.clone()
        a2[others] = a2[others].flip(-1).roll(1, -1) * 2
        f2[others] = f2[others] * 1.5
        e2[others] = -e2[others]
        s2[others] = s2[others].flip(0) + 1
        for j, f in enumerate(FRAMES):  # keep the padding of the padded inputs zero
            a2[j, :, f:] = 0
            f2[j, 2 * f :] = 0
            e2[j, 2 * f :] = 0
        after = run(a2, f2, e2, s2)
        diff = float((before[i] - after[i]).abs().max())
        assert diff < 1e-5, f"sentence {i}: max diff {diff:.3g} (peak {float(before[i].abs().max()):.3g})"
        assert not torch.allclose(before[(i + 1) % len(FRAMES)], after[(i + 1) % len(FRAMES)], atol=1e-3)


def _groups(frames, waiting, budget=100):
    from kokoro_worker.model import Synth

    synth = Synth.__new__(Synth)
    synth.frame_budget = budget
    return synth._groups(frames, waiting)


def test_rows_fill_to_the_frame_budget():
    span = 30 + masked.GAP
    assert _groups([30, 30, 30, 30], [True] * 4, budget=3 * span) == [[0, 1, 2], [3]]
    assert _groups([30, 30, 30, 30], [False] * 4, budget=3 * span) == [[0, 1, 2], [3]]
    assert _groups([200, 10], [True, True]) == [[0], [1]]  # a long one still gets a row


def test_waiting_chunks_dont_share_a_row_with_prefetch_after_them():
    # most urgent first: waiting, waiting, then prefetch
    assert _groups([10, 10, 10, 10], [True, True, False, False]) == [[0, 1], [2, 3]]
    # a more urgent prefetch, then less urgent waiting chunks: rows stay in order, and only a
    # prefetch after a waiting chunk starts a row
    assert _groups([10, 10, 10, 10], [False, True, False, True]) == [[0, 1], [2, 3]]
    assert _groups([10, 10, 10], [False, False, True]) == [[0, 1, 2]]
    # the budget still splits waiting chunks
    assert _groups([40, 40, 40, 10], [True, True, True, False]) == [[0, 1], [2], [3]]
