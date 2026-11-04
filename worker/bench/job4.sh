#!/bin/bash
# gpuc job: each concurrency twice, to separate cold cuDNN shapes from steady state.
cd "$(dirname "$0")/.." || exit
LOAD_ARGS="--concurrency 1 4 16 1 4 16" STEPS=bench CONFIGS="
warm-host-b1 host --max-batch 1
warm-host-b8 host --max-batch 8
" bash bench/job.sh
