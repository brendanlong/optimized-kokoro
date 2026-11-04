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


def test_adain_matches_each_row_alone():
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


def test_inverse_stft_normalizes_each_run_of_frames_alone():
    from kokoro_worker.kernels import MatmulSTFT

    stft = MatmulSTFT(20, 5)
    mag, phase = torch.rand(1, 11, 9), torch.rand(1, 11, 9) * 6 - 3
    alone = stft.inverse(mag, phase)
    weight = torch.ones(9)
    padded = stft.inverse(F.pad(mag, (0, 4)), F.pad(phase, (0, 4)), F.pad(weight, (0, 4)))
    assert torch.allclose(padded[..., : alone.shape[-1]], alone, atol=1e-5)
