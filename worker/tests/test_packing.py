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
    # Against the same sentence packed into a row of its own: the gaps stand in for a row's reflect
    # padding at the edges (an approximation the real model doesn't hear; bench/check_packing.py),
    # but what's next to a sentence must not matter at all.
    asr, f0, energy, s = _inputs()
    pk = masked.Packing(FRAMES, "cpu", torch.float32)
    packed = pk.unpack_audio(masked.decoder(decoder, pk.pack(asr, max(FRAMES)), _row(pk, f0), _row(pk, energy), s,
                                            pk.tensors))
    for i, f in enumerate(FRAMES):
        one = masked.Packing([f], "cpu", torch.float32)
        row = lambda x: F.pad(x[i : i + 1, : 2 * f], (0, 2 * masked.GAP))  # noqa: E731
        alone = one.unpack_audio(masked.decoder(decoder, one.pack(asr[i : i + 1], max(FRAMES)), row(f0), row(energy),
                                                s[i : i + 1], one.tensors))[0]
        assert torch.allclose(packed[i], alone, atol=1e-5)
