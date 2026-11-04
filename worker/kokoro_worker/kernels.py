"""Convolutions and STFT that run any length without per-shape plans.

cuDNN builds an execution plan the first time it sees each convolution shape, and cuFFT a plan per
STFT size: about 35 ms extra the first time a request's frame count occurs (most requests, since
the frame count depends on predicted durations), and more on a busy CPU. The replacements:

- Conv1d: one Triton implicit-GEMM kernel per layer configuration (channels, kernel, dilation,
  stride and padding are compile-time constants; batch and length are arguments). Each program
  computes a [BLOCK_O, BLOCK_T] output tile: for each tap and block of input channels, a masked
  load of the shifted input (zeros past either end stand in for the zero padding) and a
  tensor-core dot with that tap's weights, accumulated in fp32. The tile is autotuned once per
  layer configuration, not per length.
- ConvTranspose1d: one cuBLAS matmul to [out channels x taps, T], then overlap-add of the taps by
  reshaping (the kernel zero-padded to a multiple of the stride); depthwise ones elementwise.
- STFT/iSTFT (n_fft 20, hop 5): frames by unfold and a DFT matrix, the inverse by a matrix and the
  same overlap-add, with torch.stft/istft's centering, reflect padding and window normalization.

`apply(model)` swaps these into a loaded model in place. Conv1d falls back to F.conv1d off CUDA
or outside fp16/autocast, so the CPU tests and fp32 runs still work.
"""

import math

import torch
import torch.nn.functional as F
import triton
import triton.language as tl

_CONFIGS = [
    triton.Config({"BLOCK_O": bo, "BLOCK_T": bt, "BLOCK_C": bc}, num_warps=w, num_stages=s)
    for bo, bt, bc, w, s in ((64, 128, 32, 4, 3), (128, 64, 32, 4, 3), (64, 64, 64, 4, 3), (128, 128, 32, 8, 3))
]


# Keyed on the layer, not the length: tuned on the first call of each layer configuration (the
# worker's warmup), then reused for every length. The timings go to the Triton cache, which the
# compile-cache seed copies, so a seeded start doesn't tune again.
@triton.autotune(configs=_CONFIGS, key=["C", "O", "K", "DIL", "STRIDE"], cache_results=True)
# Triton compiles a variant per integer argument divisible by 16 (or equal to 1), and per pointer
# aligned to 16 bytes or not. For the lengths and the strides that scale with them, and for an input
# that may be a view at an odd offset, that is a 2 s compile the first time a request lands on a new
# combination. Pass them unspecialized. (The weights, bias and output are always our own aligned
# allocations.)
@triton.jit(do_not_specialize=["T_in", "T_out", "sxb", "sxc", "syb", "syo"],
            do_not_specialize_on_alignment=["x_ptr"])
def _conv1d_kernel(x_ptr, w_ptr, b_ptr, y_ptr, T_in, T_out,
                   sxb, sxc, sxt, syb, syo, syt,
                   C: tl.constexpr, O: tl.constexpr, K: tl.constexpr, DIL: tl.constexpr,
                   STRIDE: tl.constexpr, PAD: tl.constexpr, HAS_BIAS: tl.constexpr,
                   BLOCK_O: tl.constexpr, BLOCK_T: tl.constexpr, BLOCK_C: tl.constexpr):
    pid_t = tl.program_id(0)
    pid_o = tl.program_id(1)
    b = tl.program_id(2)
    offs_o = pid_o * BLOCK_O + tl.arange(0, BLOCK_O)
    offs_t = pid_t * BLOCK_T + tl.arange(0, BLOCK_T)
    o_ok = offs_o < O
    acc = tl.zeros((BLOCK_O, BLOCK_T), dtype=tl.float32)
    x_base = x_ptr + b * sxb
    for j in tl.static_range(K):
        t_in = offs_t * STRIDE + j * DIL - PAD
        t_ok = (t_in >= 0) & (t_in < T_in)
        for c0 in range(0, C, BLOCK_C):
            offs_c = c0 + tl.arange(0, BLOCK_C)
            c_ok = offs_c < C
            # weights arranged [K, O, C]
            w = tl.load(w_ptr + j * O * C + offs_o[:, None] * C + offs_c[None, :],
                        mask=o_ok[:, None] & c_ok[None, :], other=0.0)
            x = tl.load(x_base + offs_c[:, None] * sxc + t_in[None, :] * sxt,
                        mask=c_ok[:, None] & t_ok[None, :], other=0.0)
            acc += tl.dot(w, x)
    if HAS_BIAS:
        acc += tl.load(b_ptr + offs_o, mask=o_ok, other=0.0).to(tl.float32)[:, None]
    tl.store(y_ptr + b * syb + offs_o[:, None] * syo + offs_t[None, :] * syt, acc.to(y_ptr.dtype.element_ty),
             mask=o_ok[:, None] & (offs_t < T_out)[None, :])


def _t_out(t_in, k, stride, padding, dilation):
    return (t_in + 2 * padding - dilation * (k - 1) - 1) // stride + 1


# An opaque custom op rather than a bare kernel call: Inductor 2.8 traces bare Triton kernels into
# its graph, and its scheduler failed on the full decoder (KeyError in compute_ancestors).
@torch.library.custom_op("kokoro::conv1d", mutates_args=())
def conv1d(x: torch.Tensor, w_koc: torch.Tensor, bias: torch.Tensor | None, k: int, stride: int, padding: int,
           dilation: int) -> torch.Tensor:
    """x [B, C, T] fp16, w_koc [K, O, C] fp16 contiguous, bias [O] or None -> [B, O, T_out] fp16."""
    bsz, c, t_in = x.shape
    o = w_koc.shape[1]
    t_out = _t_out(t_in, k, stride, padding, dilation)
    y = torch.empty(bsz, o, t_out, device=x.device, dtype=x.dtype)
    # The kernel's offsets are int32. The chunk cap keeps the largest activation near 2^28 elements.
    assert x.numel() < 2**31 and y.numel() < 2**31, "conv1d: tensor too large for int32 offsets"

    def grid(meta):
        return triton.cdiv(t_out, meta["BLOCK_T"]), triton.cdiv(o, meta["BLOCK_O"]), bsz

    _conv1d_kernel[grid](x, w_koc, bias if bias is not None else w_koc, y, t_in, t_out,
                         x.stride(0), x.stride(1), x.stride(2), y.stride(0), y.stride(1), y.stride(2),
                         C=c, O=o, K=k, DIL=dilation, STRIDE=stride, PAD=padding, HAS_BIAS=bias is not None)
    return y


@conv1d.register_fake
def _(x, w_koc, bias, k, stride, padding, dilation):
    return x.new_empty(x.shape[0], w_koc.shape[1], _t_out(x.shape[2], k, stride, padding, dilation))


class TritonConv1d(torch.nn.Conv1d):
    """Conv1d through _conv1d_kernel on CUDA in fp16; `_w_koc`/`_b16` are set by apply()."""

    def forward(self, x):
        half = x.dtype == torch.float16 or (x.is_cuda and torch.is_autocast_enabled("cuda"))
        if not (x.is_cuda and half):
            return super().forward(x)
        return conv1d(x.to(torch.float16), self._w_koc, self._b16, self.kernel_size[0], self.stride[0],
                      self.padding[0], self.dilation[0])


def _overlap_add(z, stride: int):
    """z [B, C, R, stride, T]: tap block r of frame t lands at output frame t + r. Returns
    [B, C, (T + R - 1) * stride]."""
    bsz, c, r, s, t = z.shape
    out = None
    for i in range(r):
        part = F.pad(z[:, :, i], (i, r - 1 - i))  # [B, C, s, T + R - 1]
        out = part if out is None else out + part
    return out.transpose(2, 3).reshape(bsz, c, (t + r - 1) * s)


def transpose_weight(w, stride: int, groups: int):
    """ConvTranspose1d weight [C_in, O / groups, K] -> what conv_transpose1d multiplies by: the
    kernel zero-padded to a multiple of the stride, as [O * Kp, C_in] (groups == 1) or [C, Kp]
    (depthwise)."""
    in_c, out_per_group, k = w.shape
    r = math.ceil(k / stride)
    w = F.pad(w, (0, r * stride - k))
    if groups == 1:
        return w.permute(1, 2, 0).reshape(out_per_group * r * stride, in_c).contiguous()
    assert groups == in_c and out_per_group == 1, "only depthwise grouped transposed convs"
    return w[:, 0, :].contiguous()


def conv_transpose1d(x, wt, b, k: int, stride: int, padding: int, output_padding: int, groups: int):
    """F.conv_transpose1d (dilation 1) for groups == 1 or depthwise; wt from transpose_weight."""
    r = math.ceil(k / stride)
    wt = wt.to(x.dtype)
    t = x.shape[-1]
    if groups == 1:
        out_c = wt.shape[0] // (r * stride)
        z = torch.matmul(wt, x)  # [O * Kp, C] @ [B, C, T] -> [B, O * Kp, T]
    else:
        out_c = x.shape[1]
        z = x[:, :, None, :] * wt[:, :, None]  # [B, C, Kp, T]
    z = z.reshape(x.shape[0], out_c, r, stride, t)
    y = _overlap_add(z, stride)
    length = (t - 1) * stride - 2 * padding + k + output_padding
    y = y[..., padding : padding + length]
    if y.shape[-1] < length:  # output_padding past the last tap
        y = F.pad(y, (0, length - y.shape[-1]))
    if b is not None:
        y = y + b.to(y.dtype)[:, None]
    return y


class MatmulConvTranspose1d(torch.nn.ConvTranspose1d):
    """ConvTranspose1d as a matmul plus overlap-add; `_wt` is set by apply()."""

    def forward(self, x, output_size=None):
        assert output_size is None and self.dilation[0] == 1
        return conv_transpose1d(x, self._wt, self.bias, self.kernel_size[0], self.stride[0], self.padding[0],
                                self.output_padding[0], self.groups)


class MatmulSTFT(torch.nn.Module):
    """torch.stft / torch.istft (hann window, center, reflect padding, onesided) as matmuls, in fp32."""

    def __init__(self, n_fft: int, hop: int):
        super().__init__()
        assert n_fft % hop == 0
        self.n_fft, self.hop = n_fft, hop
        bins = n_fft // 2 + 1
        window = torch.hann_window(n_fft, periodic=True, dtype=torch.float64)
        n = torch.arange(n_fft, dtype=torch.float64)
        k = torch.arange(bins, dtype=torch.float64)
        angle = 2 * math.pi * n[:, None] * k[None, :] / n_fft  # [n_fft, bins]
        sin = torch.sin(angle)
        # Exactly 0 at DC and Nyquist, as an FFT gives: else atan2 sees +-1e-16 and the phase of a
        # negative real bin flips between pi and -pi.
        sin[:, 0] = 0
        if n_fft % 2 == 0:
            sin[:, -1] = 0
        # Forward: windowed frame @ [cos | -sin] -> real, imag.
        fwd = torch.cat([torch.cos(angle), -sin], dim=1) * window[:, None]
        # Inverse (irfft): bins other than DC and Nyquist count twice; then the window again.
        weight = torch.full((bins,), 2.0, dtype=torch.float64)
        weight[0] = 1
        if n_fft % 2 == 0:
            weight[-1] = 1
        inv = torch.cat([torch.cos(angle.T), -sin.T], dim=0) * (weight.repeat(2)[:, None] / n_fft)
        inv = inv * window[None, :]  # [2 * bins, n_fft]
        self.register_buffer("fwd", fwd.float(), persistent=False)
        self.register_buffer("inv", inv.float(), persistent=False)
        self.register_buffer("window_sq", (window**2).float(), persistent=False)

    @torch.autocast("cuda", enabled=False)
    def transform(self, x):
        """x [B, L] -> magnitude, phase [B, bins, frames]."""
        pad = self.n_fft // 2
        x = F.pad(x[:, None, :].float(), (pad, pad), mode="reflect")[:, 0]
        frames = x.unfold(-1, self.n_fft, self.hop)  # [B, frames, n_fft]
        spec = torch.matmul(frames, self.fwd).transpose(1, 2)  # [B, 2 * bins, frames]
        real, imag = spec.chunk(2, dim=1)
        return torch.sqrt(real * real + imag * imag), torch.atan2(imag, real)

    @torch.autocast("cuda", enabled=False)
    def inverse(self, magnitude, phase, frame_weight=None):
        """magnitude, phase [B, bins, frames] -> [B, 1, hop * (frames - 1)].

        frame_weight [frames], 1 or 0: frames that count in the window normalization. Each run of
        1s is then normalized as if it were the whole input (0 where no frame counts)."""
        magnitude, phase = magnitude.float(), phase.float()
        spec = torch.cat([magnitude * torch.cos(phase), magnitude * torch.sin(phase)], dim=1)
        frames = torch.matmul(spec.transpose(1, 2), self.inv)  # [B, T, n_fft]
        bsz, t, _ = frames.shape
        r = self.n_fft // self.hop
        z = frames.transpose(1, 2).reshape(bsz, 1, r, self.hop, t)
        y = _overlap_add(z, self.hop)  # [B, 1, (t + r - 1) * hop]
        w = self.window_sq.reshape(1, 1, r, self.hop, 1)
        w = w.expand(1, 1, r, self.hop, t) if frame_weight is None else w * frame_weight.float().reshape(1, 1, 1, 1, t)
        env = _overlap_add(w, self.hop)
        pad = self.n_fft // 2
        length = self.hop * (t - 1)
        y, env = y[..., pad : pad + length], env[..., pad : pad + length]
        if frame_weight is None:
            return y / env
        return torch.where(env > 0, y / env.clamp_min(1e-12), 0.0)


def apply(model: torch.nn.Module) -> None:
    """Swap the kernels above into a loaded KModel, in place: every Conv1d, every ConvTranspose1d,
    and the generator's STFT. Call once the model is on its device and in its dtype."""
    for m in model.modules():
        if type(m) is torch.nn.Conv1d:
            assert m.groups == 1 and m.padding_mode == "zeros"
            with torch.no_grad():
                m._w_koc = m.weight.detach().to(torch.float16).permute(2, 0, 1).contiguous()
                m._b16 = None if m.bias is None else m.bias.detach().to(torch.float16).contiguous()
                if m.weight.dtype == torch.float16:
                    # One copy: the parameter becomes a view of the rearranged weights.
                    m.weight = torch.nn.Parameter(m._w_koc.permute(1, 2, 0), requires_grad=False)
            m.__class__ = TritonConv1d
        elif type(m) is torch.nn.ConvTranspose1d:
            with torch.no_grad():
                m._wt = transpose_weight(m.weight.detach(), m.stride[0], m.groups)
            m.__class__ = MatmulConvTranspose1d
    g = model.decoder.generator
    g.stft = MatmulSTFT(g.post_n_fft, g.stft.hop_length).to(next(model.parameters()).device)
