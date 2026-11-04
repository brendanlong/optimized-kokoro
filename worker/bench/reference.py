"""The model as it was before kokoro_worker/kernels.py, for comparisons: cuDNN convolutions and
torch.stft/istft (cuFFT). Benchmarks only; the worker always runs the kernels.

    synth = reference.cudnn(Synth(models))
"""

import torch

from kokoro_worker import kernels


def cudnn(synth):
    """Undo kernels.apply on synth.model, in place (the weights stay), and return synth. Call before
    the first run, so that no compiled graph holds the kernels."""
    from kokoro.istftnet import TorchSTFT  # after Synth has imported kokoro without its G2P

    for m in synth.model.modules():
        if type(m) is kernels.TritonConv1d:
            m.__class__ = torch.nn.Conv1d
            m.weight = torch.nn.Parameter(m.weight.contiguous(), requires_grad=False)
        elif type(m) is kernels.MatmulConvTranspose1d:
            m.__class__ = torch.nn.ConvTranspose1d
    g = synth.model.decoder.generator
    g.stft = TorchSTFT(filter_length=g.post_n_fft, hop_length=g.stft.hop, win_length=g.post_n_fft)
    return synth
