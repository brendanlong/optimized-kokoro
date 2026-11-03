#!/usr/bin/env python3
"""Stream-limit and release checks. Usage: limits.py PORT KEY1 KEY2 TEST"""
import json, socket, sys, time

PORT, K1, K2, TEST = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
LONG = " ".join(f"Sentence number {i} is here and it is fairly long so it makes a chunk." for i in range(250))


def req(key, text, close=True):
    body = json.dumps({"input": text, "response_format": "pcm", "speed": 0.25}).encode()
    return (b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\n" + (b"Connection: close\r\n" if close else b"") +
            b"Authorization: Bearer " + key.encode() + b"\r\nContent-Length: %d\r\n\r\n" % len(body) + body)


def open_stream(key, text=LONG, rcvbuf=4096):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, rcvbuf)
    s.connect(("127.0.0.1", PORT))
    s.sendall(req(key, text))
    s.settimeout(10)
    first = s.recv(200)
    return s, first.split(b"\r\n", 1)[0]


def status(key, text="hello"):
    s = socket.create_connection(("127.0.0.1", PORT), timeout=20)
    s.sendall(req(key, text))
    d = b""
    try:
        while True:
            x = s.recv(65536)
            if not x: break
            d += x
    except socket.timeout:
        pass
    s.close()
    return d.split(b"\r\n", 1)[0], d.rsplit(b"\r\n\r\n", 1)[-1][:120] if b"json" in d[:300] else b""


if TEST == "per_key":
    held = []
    for i in range(6):
        s, st = open_stream(K1)
        print("stream", i, st)
        held.append(s)
    print("other key:", status(K2))
    time.sleep(30)
    print("after 30 s not reading, key1:", status(K1))
    for s in held: s.close()
    time.sleep(1)
    print("after closing, key1:", status(K1))

if TEST == "total":
    held = []
    for k in (K1, K2):
        for i in range(5):
            s, st = open_stream(k)
            print(k[:6], i, st)
            held.append(s)
    for s in held: s.close()
    time.sleep(1)
    print("after close:", status(K1), status(K2))

if TEST == "disconnect_paths":
    # 1) disconnect right after sending the body (during planning/first chunk), many times
    for i in range(50):
        s = socket.create_connection(("127.0.0.1", PORT)); s.sendall(req(K1, LONG)); s.close()
    # 2) disconnect after headers but mid-body
    for i in range(50):
        s = socket.create_connection(("127.0.0.1", PORT)); r = req(K1, LONG); s.sendall(r[: len(r) - 100]); s.close()
    # 3) RST mid-stream
    import struct
    for i in range(50):
        s = socket.create_connection(("127.0.0.1", PORT)); s.sendall(req(K1, LONG)); s.recv(1000)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0)); s.close()
    time.sleep(2)
    print("after 150 aborted requests, 4 parallel streams on key1:")
    held = []
    for i in range(4):
        s, st = open_stream(K1); print("  ", st); held.append(s)
    for s in held: s.close()
