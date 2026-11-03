"""Hostile front for the worker: malformed and extreme frames, limits, cancellation, slow and stalled
frames, and 300 mutations. Usage: python tools/hostile.py SOCKET WORKER_PID (from the PR #11 review).
It speaks the protocol as of that review; update it with the spec."""
import math, os, random, socket, struct, sys, time

SOCK, PID = sys.argv[1], int(sys.argv[2])
VOICE_COUNT = None


def rss():
    with open(f"/proc/{PID}/status") as f:
        for line in f:
            if line.startswith("VmRSS"):
                return int(line.split()[1]) // 1024


def alive():
    try:
        os.kill(PID, 0)
        return True
    except OSError:
        return False


def connect():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(20)
    s.connect(SOCK)
    hello = read_frame(s)
    assert hello[0] == 1, hello[:4]
    global VOICE_COUNT
    VOICE_COUNT = struct.unpack_from("<H", hello, 21)[0]
    return s


def recv_exact(s, n):
    b = b""
    while len(b) < n:
        k = s.recv(n - len(b))
        if not k:
            raise EOFError
        b += k
    return b


def read_frame(s):
    (n,) = struct.unpack("<I", recv_exact(s, 4))
    return recv_exact(s, n)


def fr(body):
    return struct.pack("<I", len(body)) + body


def synth(rid, ids, voices=((0, 1.0),), speed=1.0, prio=0, style_len=None, n_ids=None, raw_speed=None, raw_voices=None):
    body = struct.pack("<BQB", 0x10, rid, prio)
    body += raw_speed if raw_speed is not None else struct.pack("<f", speed)
    body += struct.pack("<B", len(raw_voices) if raw_voices is not None else len(voices))
    for v in (raw_voices or [struct.pack("<Hf", i, w) for i, w in voices]):
        body += v
    body += struct.pack("<HH", len(ids) if n_ids is None else n_ids, len(ids) if style_len is None else style_len)
    body += bytes(ids)
    return fr(body)


def cancel(rid):
    return fr(struct.pack("<BQ", 0x11, rid))


def expect_closed(s, timeout=20):
    s.settimeout(timeout)
    frames = []
    try:
        while True:
            frames.append(read_frame(s))
    except EOFError:
        return True, frames
    except ConnectionResetError:
        return True, frames
    except socket.timeout:
        return False, frames


def run(name, payload, closes=True):
    s = connect()
    try:
        s.sendall(payload)
    except (BrokenPipeError, ConnectionResetError):
        pass
    if closes:
        ok, frames = expect_closed(s)
        res = "closed" if ok else "NOT CLOSED"
    else:
        s.settimeout(30)
        frames = []
        res = "open"
    s.close()
    print(f"{name:55s} {res:10s} frames={[f[0] for f in frames]} alive={alive()} rss={rss()}MB", flush=True)
    return frames


def f32(x):
    return struct.pack("<f", x)


NAN = struct.pack("<I", 0x7FC00001)
SNAN = struct.pack("<I", 0x7F800001)
INF = struct.pack("<I", 0x7F800000)
NEGZ = struct.pack("<I", 0x80000000)
SUBN = struct.pack("<I", 0x00000001)
ok_ids = [16, 43, 102, 56]

print("baseline rss", rss())
# length prefix
run("len 0", struct.pack("<I", 0))
run("len 65537", struct.pack("<I", 65537) + b"\x10" * 100)
run("len 0xffffffff", struct.pack("<I", 0xFFFFFFFF))
run("len 65536 of junk", struct.pack("<I", 65536) + bytes(65536))
run("truncated body then EOF", struct.pack("<I", 100) + b"\x10" * 50)
run("unknown type 0x00", fr(b"\x00"))
run("unknown type 0x01 (hello)", fr(b"\x01" + bytes(30)))
run("cancel short", fr(b"\x11" + bytes(7)))
run("cancel long", fr(b"\x11" + bytes(9)))
run("synth header short", fr(b"\x10" + bytes(13)))
run("synth trailing byte", synth(1, ok_ids)[:-0] + b"" if False else fr(synth(1, ok_ids)[4:] + b"\x01"))
run("synth n_ids says more than present", synth(1, ok_ids, n_ids=10))
run("synth n_ids 0", synth(1, [], n_ids=0))
run("synth n_ids 511", synth(1, [16] * 511))
run("synth style_len 0", synth(1, ok_ids, style_len=0))
run("synth style_len 511", synth(1, ok_ids, style_len=511))
run("synth id 0", synth(1, [16, 0, 16]))
run("synth id 178", synth(1, [16, 178]))
run("synth id 255", synth(1, [255]))
run("priority 2", synth(1, ok_ids, prio=2))
run("priority 255", synth(1, ok_ids, prio=255))
for nm, raw in [("NaN", NAN), ("sNaN", SNAN), ("inf", INF), ("-0", NEGZ), ("subnormal", SUBN),
                ("0.2499999", f32(0.24999999)), ("4.0000005", f32(4.0000005)), ("-1", f32(-1.0))]:
    run(f"speed {nm}", synth(1, ok_ids, raw_speed=raw))
run("n_voices 0", fr(struct.pack("<BQBfB", 0x10, 1, 0, 1.0, 0) + struct.pack("<HH", 4, 4) + bytes(ok_ids)))
run("n_voices 9", synth(1, ok_ids, voices=[(0, 1.0)] * 9))
run("voice index = n_voices", synth(1, ok_ids, voices=[(VOICE_COUNT or 54, 1.0)]))
run("voice index 65535", synth(1, ok_ids, voices=[(65535, 1.0)]))
for nm, raw in [("NaN", NAN), ("inf", INF), ("-0", NEGZ), ("0", f32(0.0)), ("-1", f32(-1.0))]:
    run(f"weight {nm}", synth(1, ok_ids, raw_voices=[struct.pack("<H", 0) + raw]))


def valid(name, payload, n_expected):
    s = connect()
    s.sendall(payload)
    frames = []
    s.settimeout(120)
    try:
        while len(frames) < n_expected:
            frames.append(read_frame(s))
    except Exception as e:
        print("   error", e)
    s.close()
    summary = []
    for f in frames:
        if f[0] == 2:
            rid, n = struct.unpack_from("<QI", f, 1)
            pcm = f[13:]
            peak = max(abs(x) for x in struct.unpack(f"<{n}h", pcm)) if n else 0
            summary.append(("audio", rid, n, peak))
        elif f[0] == 3:
            rid, n = struct.unpack_from("<QH", f, 1)
            d = struct.unpack_from(f"<{n}H", f, 11)
            summary.append(("dur", rid, n, sum(d), max(d)))
        else:
            summary.append(("failed", *struct.unpack_from("<QB", f, 1)))
    print(f"{name:55s} {summary} alive={alive()} rss={rss()}MB", flush=True)
    return frames


# Valid but extreme values: must be accepted and yield finite audio
valid("weight subnormal alone", synth(1, ok_ids, raw_voices=[struct.pack("<H", 0) + SUBN]), 2)
valid("weights max f32 x8", synth(2, ok_ids, raw_voices=[struct.pack("<H", i) + f32(3.4028235e38) for i in range(8)]), 2)
valid("weights max + subnormal", synth(3, ok_ids, raw_voices=[struct.pack("<H", 0) + f32(3.4028235e38), struct.pack("<H", 1) + SUBN]), 2)
valid("duplicate voice index in mix", synth(4, ok_ids, voices=[(0, 1.0), (0, 2.0)]), 2)
valid("speed 0.25, unused vocab ids", synth(5, [1, 2, 3, 177, 176, 175], speed=0.25), 2)
valid("speed 4.0, n_ids 1, style_len 510", synth(6, [16], speed=4.0, style_len=510), 2)
valid("cancel unknown id then synth", cancel(99) + synth(7, ok_ids), 2)
first = valid("reference request A", synth(10, ok_ids, voices=[(3, 1.0)]), 2)

# duplicate outstanding id: send many identical ids at once so the first is still queued/running
run("duplicate outstanding id", synth(20, [16] * 400) + synth(20, ok_ids))
# 1025 outstanding
big = b"".join(synth(1000 + i, [16] * 300, prio=1) for i in range(1025))
t = time.time()
frames = run("1025 outstanding", big)
print("   took", round(time.time() - t, 1), "s")
# exactly 1024 then cancel all: should get only Failed(cancelled) for queued ones, no close
s = connect()
s.sendall(b"".join(synth(5000 + i, [16] * 300, prio=1) for i in range(1024)))
s.sendall(b"".join(cancel(5000 + i) for i in range(1024)))
got = {}
s.settimeout(60)
try:
    while sum(got.values()) < 1024 + 0:
        f = read_frame(s)
        got[f[0]] = got.get(f[0], 0) + 1
        if f[0] == 4 and len(got) and sum(got.values()) >= 1024:
            break
except Exception as e:
    print("   1024+cancel error", e)
print("1024 outstanding then cancel all                       ", got, "alive", alive(), "rss", rss())
s.close()

# id reuse after Failed(cancelled) is legal
s = connect()
s.sendall(synth(1, [16] * 400) + synth(2, ok_ids, prio=1) + cancel(2))
seen = []
s.settimeout(60)
while True:
    f = read_frame(s)
    seen.append((f[0], struct.unpack_from("<Q", f, 1)[0]))
    if (4, 2) in seen:
        break
s.sendall(synth(2, ok_ids))
while (2, 2) not in seen:
    f = read_frame(s)
    seen.append((f[0], struct.unpack_from("<Q", f, 1)[0]))
print("reuse id after cancel                                  ", seen, "alive", alive())
s.close()

# Slow frame: dribble a valid frame one byte at a time
s = connect()
payload = synth(77, ok_ids)
t = time.time()
for b in payload:
    s.sendall(bytes([b]))
    time.sleep(0.05)
f1 = read_frame(s)
print("slow dribble valid frame                               ", f1[0], round(time.time() - t, 1), "s")
s.close()

# Half frame and hold: does a second connection get served? (one-at-a-time)
s = connect()
s.sendall(struct.pack("<I", 100) + b"\x10")
s2 = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s2.settimeout(5)
s2.connect(SOCK)
try:
    read_frame(s2)
    print("second connection got hello while first stalls       yes")
except socket.timeout:
    print("second connection got hello while first stalls       NO (blocked behind stalled front; expected by design)")
s.close()
s2.settimeout(20)
read_frame(s2)
s2.close()

# Random fuzz: mutate valid frames
rng = random.Random(1)
base = [synth(1, ok_ids), synth(2, [16] * 50, voices=[(0, 1.0), (5, 0.5)], speed=1.3, style_len=7), cancel(3)]
n_closed = 0
for i in range(300):
    p = bytearray(rng.choice(base))
    for _ in range(rng.randint(1, 4)):
        op = rng.random()
        if op < 0.5 and len(p) > 4:
            p[rng.randrange(4, len(p))] = rng.randrange(256)
        elif op < 0.75:
            p[rng.randrange(len(p))] = rng.randrange(256)
        elif op < 0.9:
            del p[rng.randrange(len(p)):]
        else:
            p += bytes(rng.randrange(256) for _ in range(rng.randint(1, 8)))
    s = connect()
    try:
        s.sendall(bytes(p))
        s.shutdown(socket.SHUT_WR)
    except OSError:
        pass
    ok, frames = expect_closed(s, timeout=60)
    s.close()
    if not ok:
        print("fuzz case not closed:", bytes(p).hex())
    for f in frames:
        if f[0] == 2:
            n = struct.unpack_from("<I", f, 9)[0]
            assert len(f) == 13 + 2 * n
    n_closed += ok
    if not alive():
        print("WORKER DIED on", bytes(p).hex())
        break
print("random fuzz cases closed:", n_closed, "of 300; alive", alive(), "rss", rss())

again = valid("reference request A again (state carry?)", synth(10, ok_ids, voices=[(3, 1.0)]), 2)
print("durations identical to first:", again[0] == first[0], "audio identical:", again[1] == first[1])
