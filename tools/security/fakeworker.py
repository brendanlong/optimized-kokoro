#!/usr/bin/env python3
"""Fake Kokoro worker, protocol v4. Usage: fakeworker.py SOCKET [MODE] [delay_ms]
Modes: normal (render each Synthesize after delay), stall (never reply), slow (delay).
Logs to stderr."""
import asyncio, os, struct, sys, time

SOCK = sys.argv[1]
MODE = sys.argv[2] if len(sys.argv) > 2 else "normal"
DELAY = float(sys.argv[3]) / 1000 if len(sys.argv) > 3 else 0.0
VOICES = ["af_heart", "af_bella", "am_adam", "bf_emma", "bm_george", "jf_alpha", "zf_xiaobei", "a", "b_"]
SLOTS = int(os.environ.get("FW_SLOTS", "1024"))
CAP = int(os.environ.get("FW_CAP", "1440000"))
DUR = int(os.environ.get("FW_DUR", "1"))

def hello():
    body = b"\x01KOKW" + struct.pack("<HHHHHIIH", 4, 178, 510, 8, SLOTS, 24000, CAP, len(VOICES))
    for v in VOICES:
        body += bytes([len(v)]) + v.encode()
    return struct.pack("<I", len(body)) + body

stats = {"synth": 0, "cancel": 0, "max_queue": 0, "ids_total": 0}

async def handle(reader, writer):
    writer.write(hello()); await writer.drain()
    queue = asyncio.Queue()
    async def render_loop():
        while True:
            slot, ids, speed = await queue.get()
            if MODE == "stall":
                continue
            if DELAY: await asyncio.sleep(DELAY)
            n = len(ids) + 2
            d = DUR
            total = 600 * n * d
            amp = 1000 + sum(ids) % 20000
            body = b"\x02" + struct.pack("<HH", slot, n) + struct.pack("<H", d) * n + struct.pack("<h", amp) * total
            writer.write(struct.pack("<I", len(body)) + body)
            await writer.drain()
    task = asyncio.create_task(render_loop())
    try:
        while True:
            hdr = await reader.readexactly(4)
            (ln,) = struct.unpack("<I", hdr)
            body = await reader.readexactly(ln)
            if body[0] == 0x10:
                slot, prio, waiting, speed, voice, n_ids, style = struct.unpack("<HBBHHHH", body[1:13])
                ids = body[13:13 + n_ids]
                stats["synth"] += 1; stats["ids_total"] += n_ids
                queue.put_nowait((slot, ids, speed))
                stats["max_queue"] = max(stats["max_queue"], queue.qsize())
            elif body[0] == 0x11:
                stats["cancel"] += 1
    except (asyncio.IncompleteReadError, ConnectionError):
        pass
    finally:
        task.cancel()
        print(f"[fakeworker] connection closed {stats}", file=sys.stderr, flush=True)

async def main():
    if os.path.exists(SOCK): os.unlink(SOCK)
    server = await asyncio.start_unix_server(handle, SOCK)
    print("[fakeworker] listening", file=sys.stderr, flush=True)
    async with server:
        await server.serve_forever()

asyncio.run(main())
