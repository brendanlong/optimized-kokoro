# Adversarial review tools (PR #13)

These scripts and inputs come from the adversarial review of PR #13. They run against a local `kokoro-server` with a fake or hostile worker. Each gap the review found now has a regression test in `crates/server/tests/exhaustion.rs`; these tools are for running the attacks again by hand.

They are quick scripts: arguments are positional, and most expect the server on `127.0.0.1` and a worker socket you choose. Run Python with `-I`.

**Workers**
- `fakeworker.py SOCKET [normal|stall|slow] [delay_ms]` is a protocol-v2 worker that renders fake audio.
- `evilworker.py SOCKET MODE [seconds]` is a hostile worker with about 40 modes, listed in its source:
  - malformed or lying frames (`hello_huge_len`, `wrong_n`, `dur_over`, `samples_long`, …);
  - stalls and trickles (`stall`, `partial_stall`, `trickle_result`);
  - protocol abuse (`double_result`, `free_slot_result`, `skip_first`, `ignore_cancel`, `rapid_reconnect`).
- `evil_driver.sh MODE` runs one hostile mode against a server and reports its health. It needs `RUN=<dir>` holding `w.sock`.

**HTTP clients**
- `http_attack.py HOST PORT KEY TEST...` covers slowloris, trickled heads, idle keep-alive, chunked trickle and similar attacks.
- `limits.py PORT KEY1 KEY2 TEST` checks the stream limits and that permits are released.
- `cpu_bypass.py PORT KEY N WAIT [ROUNDS]` opens N connections at once with `worst.json` and disconnects them early. It reproduces finding 1.
- `abort_clients.py` and `memclients.py` are clients that disconnect mid-stream, or never read.
- `hdr.py` sends request heads of growing size (finding 7).
- `timing.py` checks the key comparison for timing differences.

**Inputs and generators**
- `worst.json` and `worst_b.json` are 20 KB request bodies of long negative numbers: about 481k phoneme ids, ~7 h of audio (finding 4).
- `shapes.py`, `shapes2.py` and `randmix.py` generate adversarial texts for `crates/server/examples/adv.rs`.
- `stgen.py TAGGER.safetensors OUTDIR` writes corrupted tagger weight files (finding 8).

**Rust harnesses** are in `crates/server/examples/`:
- `adv.rs` times the text path for each input line (JSON on stdin).
- `textfuzz.rs` sends random texts through the text path, looking for panics.
- `statefuzz.rs` checks that one input can't change the output for a later one.
