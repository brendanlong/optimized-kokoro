#!/bin/bash
# gpuc job: padding-policy sweep (host, faster to compile), then gVisor cache seeding.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
STEPS=bench CONFIGS="
host-b8-s025 host --max-batch 8 --pad-slack 0.25
host-b8-s1 host --max-batch 8 --pad-slack 1
host-b8-sinf host --max-batch 8 --pad-slack 1000
host-b8-sinf-plain host --max-batch 8 --pad-slack 1000 --no-exact
host-b16-s1 host --max-batch 16 --pad-slack 1
" bash bench/job.sh
bash bench/job_cache.sh
