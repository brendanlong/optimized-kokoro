"""Convolution blocks that stand in for Kokoro's six bidirectional LSTMs.

Distilled from them (brendanlong/kokoro#3, experiments/lstm_swap): each ConvSeq maps the LSTM's
input [B, T, C_in] to an output shaped like its [B, T, 512]. A ConvSeq is a linear projection and
two ConvNeXt blocks (depthwise convolution with kernel 7, dilations 1 and 3, then layer norm and a
position-wise MLP).

Under padding each row's output is what it would be alone: everything but the depthwise
convolution is per position, and the convolution's input is zeroed at padded positions, as the
convolution's own zero padding would be past the end of a row alone. Outputs at padded positions
are not zero; callers mask them.
"""

import torch
import torch.nn.functional as F

# Where the LSTMs sit in KModel.
PATHS = (
    "text_encoder.lstm",
    "predictor.text_encoder.lstms.0",
    "predictor.text_encoder.lstms.2",
    "predictor.text_encoder.lstms.4",
    "predictor.lstm",
    "predictor.shared",
)
BLOCKS, KERNEL, MULT = 2, 7, 1.5


class DepthwiseConv1d(torch.nn.Module):
    """A depthwise Conv1d ("same" zero padding) as a sum of shifted copies: no per-shape cuDNN
    plan, and kernels.apply leaves it alone. Parameters are named and shaped as nn.Conv1d's."""

    def __init__(self, dim: int, kernel: int, dilation: int):
        super().__init__()
        self.weight = torch.nn.Parameter(torch.empty(dim, 1, kernel))
        self.bias = torch.nn.Parameter(torch.empty(dim))
        self.kernel, self.dilation = kernel, dilation

    def forward(self, x):  # [B, C, T]
        t = x.shape[-1]
        pad = self.dilation * (self.kernel - 1) // 2
        xp = F.pad(x, (pad, pad))
        w = self.weight.to(x.dtype)
        y = self.bias.to(x.dtype)[:, None]
        for j in range(self.kernel):
            y = y + xp[..., j * self.dilation : j * self.dilation + t] * w[:, :, j]
        return y


class ConvNeXtBlock(torch.nn.Module):
    def __init__(self, dim: int, kernel: int, dilation: int, mult: float):
        super().__init__()
        self.dw = DepthwiseConv1d(dim, kernel, dilation)
        self.norm = torch.nn.LayerNorm(dim)
        self.pw1 = torch.nn.Linear(dim, int(mult * dim))
        self.pw2 = torch.nn.Linear(int(mult * dim), dim)

    def forward(self, x, keep):  # x [B, T, C], keep [B, T, 1]
        h = self.dw((x * keep).transpose(1, 2)).transpose(1, 2)
        return x + self.pw2(F.gelu(self.pw1(self.norm(h))))


class ConvSeq(torch.nn.Module):
    def __init__(self, d_in: int, d_out: int):
        super().__init__()
        self.proj = torch.nn.Linear(d_in, d_out)
        self.blocks = torch.nn.ModuleList(ConvNeXtBlock(d_out, KERNEL, 3**i, MULT) for i in range(BLOCKS))

    def forward(self, x, keep):  # x [B, T, C_in], keep [B, T, 1] (1 at real positions, 0 at padding)
        x = self.proj(x)
        for block in self.blocks:
            x = block(x, keep)
        return x


def install(model: torch.nn.Module) -> None:
    """Replace each LSTM in a KModel with an (untrained) ConvSeq of the same input and output size."""
    for path in PATHS:
        lstm = model.get_submodule(path)
        assert isinstance(lstm, torch.nn.LSTM) and lstm.bidirectional and lstm.batch_first
        parent, name = path.rsplit(".", 1)
        parent = model.get_submodule(parent)
        seq = ConvSeq(lstm.input_size, 2 * lstm.hidden_size)
        if isinstance(parent, torch.nn.ModuleList):
            parent[int(name)] = seq
        else:
            setattr(parent, name, seq)
