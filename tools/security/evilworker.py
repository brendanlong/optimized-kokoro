#!/usr/bin/env python3
"""Hostile worker. Usage: evilworker.py SOCKET MODE [seconds-to-run]. Logs events to stderr."""
import os, socket, struct, sys, threading, time, random

SOCK, MODE = sys.argv[1], sys.argv[2]
RUN = float(sys.argv[3]) if len(sys.argv) > 3 else 20
VOICES = ["af_heart", "af_bella", "bf_emma"]


def hello(slots=64, cap=2880000, voices=VOICES, version=2, vocab=178, maxph=510, batch=4, rate=24000, magic=b"KOKW", n_voices=None):
    body = b"\x01" + magic + struct.pack("<HHHHHIIH", version, vocab, maxph, batch, slots, rate, cap,
                                         len(voices) if n_voices is None else n_voices)
    parts = []
    for v in voices:
        vb = v.encode() if isinstance(v, str) else v
        parts.append(bytes([len(vb)]) + vb)
    body += b"".join(parts)
    return struct.pack("<I", len(body)) + body


def frame(body):
    return struct.pack("<I", len(body)) + body


def result(slot, durs, sample=3000, nsamples=None, n=None):
    total = 600 * sum(durs) if nsamples is None else nsamples
    body = b"\x02" + struct.pack("<HH", slot, len(durs) if n is None else n) + struct.pack(f"<{len(durs)}H", *durs)
    body += struct.pack("<h", sample) * total
    return frame(body)


def failed(slot, code):
    return frame(b"\x03" + struct.pack("<HB", slot, code))


def read_frames(conn, on_frame):
    buf = b""
    while True:
        try:
            d = conn.recv(65536)
        except OSError:
            return
        if not d:
            return
        buf += d
        while len(buf) >= 4:
            (ln,) = struct.unpack("<I", buf[:4])
            if len(buf) < 4 + ln:
                break
            body, buf = buf[4:4 + ln], buf[4 + ln:]
            if body[0] == 0x10:
                slot, prio, speed, voice, n_ids, style = struct.unpack("<HBHHHH", body[1:12])
                on_frame(("syn", slot, speed, n_ids, body[12:]))
            elif body[0] == 0x11:
                on_frame(("cancel", struct.unpack("<H", body[1:3])[0]))


def log(*a):
    print(f"[evil {MODE} {time.strftime('%X')}]", *a, file=sys.stderr, flush=True)


def maxdur(speed):
    return -(-5000 // speed)


def serve(conn):
    m = MODE
    # ---------- hello abuse ----------
    hellos = {
        "slots0": lambda: hello(slots=0), "slots65535": lambda: hello(slots=65535),
        "cap_over": lambda: hello(cap=2880001), "cap0": lambda: hello(cap=0), "cap_max": lambda: hello(cap=0xFFFFFFFF),
        "vocab1": lambda: hello(vocab=1), "vocab257": lambda: hello(vocab=257), "voices0": lambda: hello(voices=[]),
        "voices_dup": lambda: hello(voices=["af_a", "af_a"]), "voice_bad": lambda: hello(voices=["af_A"]),
        "voice_long": lambda: hello(voices=["a" * 65]), "voice_empty": lambda: hello(voices=[""]),
        "voice_nul": lambda: hello(voices=[b"af\x00x"]), "version1": lambda: hello(version=1),
        "magic": lambda: hello(magic=b"KOKX"), "maxph511": lambda: hello(maxph=511), "batch0": lambda: hello(batch=0),
        "rate": lambda: hello(rate=22050), "nvoices_more": lambda: hello(n_voices=5),
        "trailing": lambda: hello()[:-0] + b"",
        "voices65535": lambda: hello(voices=[("v%05d" % i) + "_" + "x" * 57 for i in range(65535)]),
    }
    if m in hellos:
        h = hellos[m]()
        conn.sendall(h)
        log("sent hello", len(h))
        time.sleep(RUN)
        return
    if m == "hello_huge_len":
        conn.sendall(struct.pack("<I", 0xFFFFFFFF) + b"\x01"); time.sleep(RUN); return
    if m == "hello_len_over":
        conn.sendall(struct.pack("<I", 25 + 65535 * 65 + 1) + b"\x01"); time.sleep(RUN); return
    if m == "len0":
        conn.sendall(struct.pack("<I", 0)); time.sleep(RUN); return
    if m == "garbage":
        conn.sendall(os.urandom(100000)); time.sleep(RUN); return
    if m == "hello_trickle":
        h = hello()
        for b in h:
            conn.sendall(bytes([b])); time.sleep(0.2)
        time.sleep(RUN); return
    if m == "hello_partial_stall":
        conn.sendall(hello()[:10]); time.sleep(RUN); return
    if m == "no_hello":
        time.sleep(RUN); return
    if m == "rapid_reconnect":
        conn.sendall(hello()); conn.close(); return
    if m == "first_not_hello":
        conn.sendall(failed(0, 2)); time.sleep(RUN); return
    if m == "hello_twice":
        conn.sendall(hello() + hello()); time.sleep(RUN); return

    CAP = int(os.environ.get("EVIL_CAP", "2880000"))
    conn.sendall(hello(slots=int(os.environ.get("EVIL_SLOTS", "64")), cap=CAP))
    lock = threading.Lock()
    busy = {}

    def send(b):
        with lock:
            try:
                conn.sendall(b)
            except OSError as e:
                log("send failed", e)

    def on(f):
        if f[0] == "syn":
            _, slot, speed, n, ids = f
            busy[slot] = (speed, n)
            log("syn", slot, speed, n)
            act(slot, speed, n)
        else:
            log("cancel", f[1])
            on_cancel(f[1])

    def act(slot, speed, n):
        d = [1] * (n + 2)
        if m == "free_slot_result":
            send(result(slot + 1, d))
        elif m == "slot_oob":
            send(result(60000, d))
        elif m == "wrong_n":
            send(result(slot, [1] * (n + 3)))
        elif m == "n_lies":
            send(result(slot, d, n=n + 1))
        elif m == "dur0":
            send(result(slot, [0] + d[1:]))
        elif m == "dur_over":
            send(result(slot, [maxdur(speed) + 1] + d[1:]))
        elif m == "dur_max":
            send(result(slot, [maxdur(speed)] * (n + 2)))
        elif m == "cap_exceed":
            send(result(slot, [maxdur(speed)] * (n + 2)))
        elif m == "fill_cap":
            cap = -(-CAP * 100 // speed)
            per = max(1, min(maxdur(speed), cap // (600 * (n + 2))))
            send(result(slot, [per] * (n + 2)))
        elif m == "cancel_then_result":
            def t():
                time.sleep(0.5); send(result(slot, d))
            threading.Thread(target=t, daemon=True).start()
        elif m == "skip_first":
            if slot != 0:
                send(result(slot, d))
        elif m == "samples_short":
            send(result(slot, d, nsamples=600 * (n + 2) - 1))
        elif m == "samples_long":
            send(result(slot, d, nsamples=600 * (n + 2) + 1))
        elif m == "fail_cancelled_uncancelled":
            send(failed(slot, 1))
        elif m == "fail_code9":
            send(failed(slot, 9))
        elif m == "fail_internal":
            send(failed(slot, 2))
        elif m == "fail_toolong":
            send(failed(slot, 3))
        elif m == "double_result":
            send(result(slot, d) + result(slot, d))
        elif m == "partial_stall":
            send(result(slot, d)[:-100])
        elif m == "header_stall":
            send(result(slot, d)[:6])
        elif m == "trickle_result":
            def t():
                r = result(slot, d)
                for i in range(0, len(r), 50):
                    send(r[i:i + 50]); time.sleep(0.5)
            threading.Thread(target=t, daemon=True).start()
        elif m == "stall":
            pass
        elif m == "slow_ok":
            def t():
                time.sleep(2); send(result(slot, d))
            threading.Thread(target=t, daemon=True).start()
        elif m in ("ok", "race_cancel", "ignore_cancel"):
            if m == "race_cancel":
                def t():
                    time.sleep(0.3); send(result(slot, d))
                threading.Thread(target=t, daemon=True).start()
            else:
                send(result(slot, d))
        elif m == "swap":
            # answer every request with the previous slot's audio when lengths match
            send(result(slot, d, sample=1234))
        elif m == "unknown_type":
            send(frame(b"\x07\x00\x00"))
        elif m == "failed_long":
            send(frame(b"\x03" + struct.pack("<HB", slot, 2) + b"x"))
        elif m == "result_len_lie_huge":
            send(struct.pack("<I", 0x7FFFFFFF) + b"\x02" + struct.pack("<HH", slot, n + 2))
        elif m == "result_short_len":
            send(struct.pack("<I", 3) + b"\x02" + struct.pack("<H", slot))

    def on_cancel(slot):
        if m == "race_cancel":
            send(failed(slot, 1))  # in addition to the delayed Result: one of them is for a free slot
        elif m == "ignore_cancel":
            pass

    threading.Thread(target=read_frames, args=(conn, on), daemon=True).start()
    time.sleep(RUN)


def main():
    if os.path.exists(SOCK):
        os.unlink(SOCK)
    srv = socket.socket(socket.AF_UNIX)
    srv.bind(SOCK)
    srv.listen(16)
    end = time.time() + RUN
    n = 0
    srv.settimeout(1)
    while time.time() < end:
        try:
            c, _ = srv.accept()
        except socket.timeout:
            continue
        n += 1
        log("connection", n)
        threading.Thread(target=lambda c=c: (serve(c), c.close()), daemon=True).start()
    log("connections total", n)


main()
