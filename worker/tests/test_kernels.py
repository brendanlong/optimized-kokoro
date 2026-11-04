"""kernels.py's transposed convolution and STFT against torch's, on CPU (the Triton conv1d needs a GPU;
bench/kernels_bench.py checks it)."""

import pytest
import torch
import torch.nn.functional as F

from kokoro_worker import kernels


@pytest.mark.parametrize(
    "in_c, out_c, k, stride, padding, output_padding, groups, t",
    [
        (512, 256, 20, 10, 5, 0, 1, 37),  # the generator's upsamplers
        (256, 128, 12, 6, 3, 0, 1, 1),
        (64, 64, 3, 2, 1, 1, 64, 50),  # AdainResBlk1d's depthwise pool
        (8, 4, 5, 3, 0, 2, 1, 7),  # kernel not a multiple of the stride, output_padding past the taps
    ],
)
def test_conv_transpose1d(in_c, out_c, k, stride, padding, output_padding, groups, t):
    torch.manual_seed(0)
    m = torch.nn.ConvTranspose1d(in_c, out_c, k, stride=stride, padding=padding, output_padding=output_padding,
                                 groups=groups).double()
    x = torch.randn(2, in_c, t, dtype=torch.float64)
    ref = m(x)
    m._wt = kernels.transpose_weight(m.weight.detach(), stride, groups)
    m.__class__ = kernels.MatmulConvTranspose1d
    got = m(x)
    assert got.shape == ref.shape
    assert torch.allclose(got, ref, atol=1e-10)


@pytest.mark.parametrize("length", [11, 12, 101, 6005])  # reflect padding needs more than n_fft / 2
def test_stft_round_trip_matches_torch(length):
    torch.manual_seed(0)
    st = kernels.MatmulSTFT(20, 5)
    x = torch.randn(3, length)
    window = torch.hann_window(20, periodic=True)
    spec = torch.stft(x, 20, 5, 20, window=window, return_complex=True)
    mag, phase = st.transform(x)
    assert torch.allclose(mag, spec.abs(), atol=1e-4)
    # Phases agree where the bin isn't ~0 (atan2 of rounding noise is arbitrary).
    big = spec.abs() > 1e-3
    diff = torch.remainder(phase - spec.angle() + torch.pi, 2 * torch.pi) - torch.pi
    assert diff[big].abs().max() < 1e-3
    ref = torch.istft(spec, 20, 5, 20, window=window)
    got = st.inverse(spec.abs(), spec.angle())[:, 0]
    assert got.shape == ref.shape
    assert torch.allclose(got, ref, atol=1e-4)


def test_stft_exact_at_dc_and_nyquist():
    # DC and Nyquist bins are real. With negative values there the phase must be exactly torch's (pi),
    # not -pi from atan2 of a rounding-sized imaginary part.
    st = kernels.MatmulSTFT(20, 5)
    x = -torch.ones(1, 200)
    x[:, ::2] = -2
    _, phase = st.transform(x)
    spec = torch.stft(x, 20, 5, 20, window=torch.hann_window(20, periodic=True), return_complex=True)
    for b in (0, -1):
        assert torch.equal(phase[:, b], spec.angle()[:, b])
