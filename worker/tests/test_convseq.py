import torch
import torch.nn.functional as F

from kokoro_worker import convseq


def test_depthwise_matches_conv1d():
    torch.manual_seed(0)
    for dilation in (1, 3):
        conv = convseq.DepthwiseConv1d(8, 7, dilation)
        torch.nn.init.normal_(conv.weight)
        torch.nn.init.normal_(conv.bias)
        x = torch.randn(2, 8, 23)
        ref = F.conv1d(x, conv.weight, conv.bias, padding=dilation * 3, dilation=dilation, groups=8)
        torch.testing.assert_close(conv(x), ref, rtol=1e-5, atol=1e-5)


def test_padded_rows_match_rows_alone():
    torch.manual_seed(0)
    seq = convseq.ConvSeq(12, 16)
    for p in seq.parameters():
        torch.nn.init.normal_(p, std=0.3)
    lengths = [5, 31, 18]
    rows = [torch.randn(1, n, 12) for n in lengths]
    width = max(lengths)
    # Junk past each row's end: padding must not leak into real positions.
    x = torch.randn(len(rows), width, 12)
    keep = torch.zeros(len(rows), width, 1)
    for i, (r, n) in enumerate(zip(rows, lengths)):
        x[i, :n] = r[0]
        keep[i, :n] = 1
    out = seq(x, keep)
    for i, (r, n) in enumerate(zip(rows, lengths)):
        alone = seq(r, torch.ones(1, n, 1))
        torch.testing.assert_close(out[i, :n], alone[0], rtol=1e-5, atol=1e-5)
