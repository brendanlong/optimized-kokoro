"""Where a worker's startup goes: load, then each warmup shape split into torch.compile's stages (per
compiled graph) and everything outside them (running the model, and compiling and autotuning
kernels.py's Triton conv1d, which happen at run time).

    cd WORKER_DIR && python /path/to/compile_profile.py MODELS MAX_BATCH OUT_JSON

Imports kokoro_worker from the current directory, so it profiles any checkout of the worker.
Compile caches are wherever TORCHINDUCTOR_CACHE_DIR and TRITON_CACHE_DIR point.
"""

import json
import os
import sys
import time

sys.path.insert(0, os.getcwd())

import torch  # noqa: E402
import torch._dynamo.utils as du  # noqa: E402

from kokoro_worker.model import Request, Synth  # noqa: E402

models, max_batch, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
US = 1e-6


def graph(m) -> dict:
    total = m.entire_frame_compile_time_s or 0
    backend = m.backend_compile_time_s or 0
    return {
        "name": m.co_name,
        "nodes": m.graph_node_count,
        "ops": m.graph_op_count,
        "guards": m.guard_count,
        "shape_guards": m.shape_env_guard_count,
        "graph_breaks": m.num_graph_breaks,
        "fx_cache_hit": (m.inductor_fx_remote_cache_hit_count or 0) > 0 or None,
        "total_s": round(total, 2),
        # Tracing the Python code into a graph (Dynamo), before handing it to the backend.
        "trace_s": round(total - backend, 2),
        # AOTAutograd and Inductor: decompositions, graph passes, lowering, generating the code.
        "backend_s": round(backend, 2),
        "inductor_s": round(m.inductor_compile_time_s or 0, 2),
        "code_gen_s": round(m.code_gen_time_s or 0, 2),
        "triton_compile_s": round((m.triton_compile_time_us or 0) * US, 2),
        "autotune_s": round((m.compile_time_autotune_time_us or 0) * US, 2),
        "post_grad_s": round((m.post_grad_pass_time_us or 0) * US, 2),
    }


t0 = time.monotonic()
synth = Synth(models)
report = {"load_s": round(time.monotonic() - t0, 2), "shapes": []}
ids = sorted(synth.vocab_ids)
# server.warmup's shapes, in its order.
shapes = ([[40, 44], [100, 104, 110][:max_batch]] if max_batch >= 2 else []) + [[12], [60], [180], [400]]
cap, synth.max_chunk_samples = synth.max_chunk_samples, 2**32 - 1
for lengths in shapes:
    reqs = [Request([ids[(i * 7 + n) % len(ids)] for i in range(n)], synth.style(0, n), 100) for n in lengths]
    before = len(du.get_compilation_metrics())
    t = time.monotonic()
    for _ in synth.run(reqs):
        pass
    torch.cuda.synchronize()
    wall = time.monotonic() - t
    graphs = [graph(m) for m in du.get_compilation_metrics()[before:]]
    compiled = sum(g["total_s"] for g in graphs)
    report["shapes"].append({"lengths": lengths, "wall_s": round(wall, 2), "compile_s": round(compiled, 2),
                             "outside_compile_s": round(wall - compiled, 2), "graphs": graphs})
    print(f"{lengths}: {wall:.1f} s, of which torch.compile {compiled:.1f} s over {len(graphs)} graphs", flush=True)
    for g in graphs:
        print("   ", json.dumps(g), flush=True)
synth.max_chunk_samples = cap
report["total_s"] = round(time.monotonic() - t0, 2)
report["compile_times"] = du.compile_times(repr="str", aggregate=True)
print(report["compile_times"], flush=True)
print(f"total {report['total_s']} s", flush=True)
with open(out, "w") as f:
    json.dump(report, f, indent=1)
