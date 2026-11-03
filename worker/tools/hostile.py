"""Hostile front for the worker: malformed and extreme frames, slot misuse, cancellation, slow and
stalled frames, and 300 mutations. Usage: python tools/hostile.py SOCKET WORKER_PID (from the PR #11
review; updated for protocol v2's slots)."""
import os, random, socket, struct, sys, time

SOCK, PID = sys.argv[1], int(sys.argv[2])
VOICE_COUNT = SLOTS = None


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
    assert hello[0] == 1 and struct.unpack_from("<H", hello, 5)[0] == 2, hello[:7]
    global VOICE_COUNT, SLOTS
    SLOTS = struct.unpack_from("<H", hello, 13)[0]
    VOICE_COUNT = struct.unpack_from("<H", hello, 23)[0]
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


def synth(slot, ids, voice=0, speed=100, prio=0, style_len=None, n_ids=None):
    body = struct.pack("<BHBHHHH", 0x10, slot, prio, speed, voice, len(ids) if n_ids is None else n_ids,
                       len(ids) if style_len is None else style_len)
    return fr(body + bytes(ids))


def cancel(slot):
    return fr(struct.pack("<BH", 0x11, slot))


def slot_of(f):
    return struct.unpack_from("<H", f, 1)[0]


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


ok_ids = [16, 43, 102, 56]

print("baseline rss", rss())
connect().close()
print("hello: slots", SLOTS, "voices", VOICE_COUNT)
# length prefix
run("len 0", struct.pack("<I", 0))
run("len 523 (one over Synthesize's largest)", struct.pack("<I", 523) + b"\x10" * 523)
run("len 65536 of junk", struct.pack("<I", 65536) + bytes(65536))
run("len 0xffffffff", struct.pack("<I", 0xFFFFFFFF))
run("truncated body then EOF", struct.pack("<I", 100) + b"\x10" * 50)
run("unknown type 0x00", fr(b"\x00"))
run("unknown type 0x01 (hello)", fr(b"\x01" + bytes(30)))
run("unknown type 0x02 (result)", fr(b"\x02" + bytes(4)))
run("cancel short", fr(b"\x11" + bytes(1)))
run("cancel long", fr(b"\x11" + bytes(3)))
run("cancel slot = slots", cancel(SLOTS))
run("cancel slot 65535", cancel(65535))
run("synth header short", fr(b"\x10" + bytes(10)))
run("synth trailing byte", fr(synth(1, ok_ids)[4:] + b"\x01"))
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
for speed in [0, 24, 401, 65535]:
    run(f"speed {speed}", synth(1, ok_ids, speed=speed))
run("voice index = n_voices", synth(1, ok_ids, voice=VOICE_COUNT))
run("voice index 65535", synth(1, ok_ids, voice=65535))
run("slot = slots", synth(SLOTS, ok_ids))
run("slot 65535", synth(65535, ok_ids))


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
            slot, n = struct.unpack_from("<HH", f, 1)
            d = struct.unpack_from(f"<{n}H", f, 5)
            pcm = f[5 + 2 * n:]
            assert len(pcm) == 1200 * sum(d), "audio length doesn't match the durations"
            peak = max(abs(x) for x in struct.unpack(f"<{len(pcm) // 2}h", pcm)) if pcm else 0
            summary.append(("result", slot, n, sum(d), max(d), len(pcm) // 2, peak))
        else:
            summary.append(("failed", *struct.unpack_from("<HB", f, 1)))
    print(f"{name:55s} {summary} alive={alive()} rss={rss()}MB", flush=True)
    return frames


# Valid but extreme values: must be accepted and yield finite audio
valid("speed 25, unused vocab ids", synth(5, [1, 2, 3, 177, 176, 175], speed=25), 1)
valid("speed 400, n_ids 1, style_len 510", synth(6, [16], speed=400, style_len=510), 1)
valid("last slot, last voice", synth(SLOTS - 1, ok_ids, voice=VOICE_COUNT - 1), 1)
valid("cancel a free slot, then synth", cancel(7) + synth(7, ok_ids), 1)
first = valid("reference request A", synth(10, ok_ids, voice=3), 1)

# a Synthesize for a busy slot: the first is still queued or running when the second arrives
run("synthesize for a busy slot", synth(20, [16] * 400) + synth(20, ok_ids))
# every slot, then cancel all: queued ones answer Failed(cancelled), running ones their result
s = connect()
t = time.time()
s.sendall(b"".join(synth(i, [16] * 300, prio=1) for i in range(SLOTS)))
s.sendall(b"".join(cancel(i) for i in range(SLOTS)))
got, slots_seen = {}, set()
s.settimeout(60)
try:
    while len(slots_seen) < SLOTS:
        f = read_frame(s)
        got[f[0]] = got.get(f[0], 0) + 1
        slots_seen.add(slot_of(f))
except Exception as e:
    print("   all slots + cancel error", e)
print(f"{'every slot, then cancel all':55s} {got} one reply per slot: {len(slots_seen) == SLOTS} "
      f"{round(time.time() - t, 1)} s alive={alive()} rss={rss()}MB")
s.close()

# reusing a slot after its Failed(cancelled) is legal
s = connect()
s.sendall(synth(1, [16] * 400) + synth(2, ok_ids, prio=1) + cancel(2))
seen = []
s.settimeout(60)
while (3, 2) not in seen:
    f = read_frame(s)
    seen.append((f[0], slot_of(f)))
s.sendall(synth(2, ok_ids))
while (2, 2) not in seen:
    f = read_frame(s)
    seen.append((f[0], slot_of(f)))
print(f"{'reuse a slot after its cancel':55s} {seen} alive={alive()}")
s.close()

# Slow frame: dribble a valid frame one byte at a time
s = connect()
payload = synth(77, ok_ids)
t = time.time()
for b in payload:
    s.sendall(bytes([b]))
    time.sleep(0.05)
f1 = read_frame(s)
print(f"{'slow dribble valid frame':55s} {f1[0]} {round(time.time() - t, 1)} s")
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
base = [synth(1, ok_ids), synth(2, [16] * 50, voice=5, speed=130, style_len=7), cancel(3),
        synth(4, ok_ids) + cancel(4) + synth(4, [16])]
n_closed = 0
for i in range(300):
    p = bytearray(rng.choice(base))
    for _ in range(rng.randint(1, 4)):
        op = rng.random()
        if op < 0.5 and len(p) > 4:
            p[rng.randrange(4, len(p))] = rng.randrange(256)
        elif op < 0.75 and p:
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
            n = struct.unpack_from("<H", f, 3)[0]
            d = struct.unpack_from(f"<{n}H", f, 5)
            assert len(f) == 5 + 2 * n + 1200 * sum(d)
    n_closed += ok
    if not alive():
        print("WORKER DIED on", bytes(p).hex())
        break
print("random fuzz cases closed:", n_closed, "of 300; alive", alive(), "rss", rss())

again = valid("reference request A again (state carry?)", synth(10, ok_ids, voice=3), 1)
print("result identical to the first:", again == first)
