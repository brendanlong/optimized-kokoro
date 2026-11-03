#!/usr/bin/env python3
"""Raw-TCP attacks on a running kokoro-server. Usage: http_attack.py HOST PORT KEY TEST..."""
import json, socket, sys, time, threading

HOST, PORT, KEY = sys.argv[1], int(sys.argv[2]), sys.argv[3]
TESTS = sys.argv[4:]


def conn(timeout=30):
    s = socket.create_connection((HOST, PORT), timeout=timeout)
    return s


def recv_all(s, limit=1 << 24, timeout=15):
    s.settimeout(timeout)
    out = b""
    t0 = time.time()
    try:
        while len(out) < limit:
            d = s.recv(65536)
            if not d:
                return out, "EOF", time.time() - t0
            out += d
    except socket.timeout:
        return out, "TIMEOUT", time.time() - t0
    except ConnectionResetError:
        return out, "RST", time.time() - t0
    return out, "LIMIT", time.time() - t0


def status_lines(data):
    return [l for l in data.split(b"\r\n") if l.startswith(b"HTTP/1.")]


def speech(body, key=KEY, extra=b""):
    if isinstance(body, (dict, list)):
        body = json.dumps(body).encode()
    auth = b"Authorization: Bearer " + key.encode() + b"\r\n" if key is not None else b""
    return (b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\n" + auth + extra +
            b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body)


def one(raw, timeout=15, limit=1 << 26):
    s = conn()
    s.sendall(raw)
    data, how, dt = recv_all(s, limit=limit, timeout=timeout)
    s.close()
    return data, how, dt


def show(name, data, how, dt, n=300):
    head = data.split(b"\r\n\r\n", 1)
    print(f"--- {name}: {how} after {dt:.2f}s, {len(data)} bytes; statuses={status_lines(data)[:5]}")
    if len(head) > 1 and b"json" in head[0]:
        print("    body:", head[1][:n])


def t_unauth_trickle():
    """Unauthenticated POST with a huge Content-Length, then trickle the body: how long does the server keep the connection?"""
    s = conn(120)
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nContent-Length: 100000000\r\n\r\n")
    t0 = time.time()
    s.settimeout(0.2)
    got = b""
    while time.time() - t0 < 60:
        try:
            s.sendall(b"a")
        except (BrokenPipeError, ConnectionResetError) as e:
            print(f"--- unauth_trickle: send failed after {time.time()-t0:.1f}s: {e}; got {status_lines(got)}")
            return
        try:
            d = s.recv(4096)
            if d:
                got += d
            else:
                print(f"--- unauth_trickle: EOF after {time.time()-t0:.1f}s; got {status_lines(got)}")
                return
        except socket.timeout:
            pass
        time.sleep(0.8)
    print(f"--- unauth_trickle: STILL OPEN after 60s; got {status_lines(got)}")


def t_unauth_chunked_trickle():
    s = conn(120)
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n")
    t0 = time.time()
    s.settimeout(0.2)
    got = b""
    while time.time() - t0 < 60:
        try:
            s.sendall(b"1\r\na\r\n")
        except (BrokenPipeError, ConnectionResetError) as e:
            print(f"--- unauth_chunked_trickle: send failed after {time.time()-t0:.1f}s: {e}; got {status_lines(got)}")
            return
        try:
            d = s.recv(4096)
            if d:
                got += d
            else:
                print(f"--- unauth_chunked_trickle: EOF after {time.time()-t0:.1f}s; got {status_lines(got)}")
                return
        except socket.timeout:
            pass
        time.sleep(0.8)
    print(f"--- unauth_chunked_trickle: STILL OPEN after 60s; got {status_lines(got)}")


def t_404_trickle():
    s = conn(120)
    s.sendall(b"POST /nope HTTP/1.1\r\nHost: x\r\nContent-Length: 100000000\r\n\r\n")
    t0 = time.time(); s.settimeout(0.2); got = b""
    while time.time() - t0 < 40:
        try:
            s.sendall(b"a")
            d = s.recv(4096)
            if not d:
                print(f"--- 404_trickle: EOF after {time.time()-t0:.1f}s; {status_lines(got)}"); return
            got += d
        except socket.timeout:
            pass
        except (BrokenPipeError, ConnectionResetError) as e:
            print(f"--- 404_trickle: {e} after {time.time()-t0:.1f}s; {status_lines(got)}"); return
        time.sleep(0.8)
    print(f"--- 404_trickle: STILL OPEN after 40s; {status_lines(got)}")


def t_slowloris_headers():
    s = conn(120)
    t0 = time.time()
    req = b"GET /health HTTP/1.1\r\nHost: x\r\n" + b"X-a: b\r\n" * 1000
    try:
        for i, b in enumerate(req):
            s.sendall(bytes([b]))
            time.sleep(0.5)
            if time.time() - t0 > 40:
                break
    except (BrokenPipeError, ConnectionResetError) as e:
        print(f"--- slowloris_headers: closed after {time.time()-t0:.1f}s ({e})"); return
    data, how, dt = recv_all(s, timeout=2)
    print(f"--- slowloris_headers: after {time.time()-t0:.1f}s {how} {status_lines(data)}")


def t_idle_keepalive():
    s = conn(60)
    s.sendall(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n")
    data, how, dt = recv_all(s, timeout=40)
    print(f"--- idle_keepalive: {how} after {dt:.1f}s {status_lines(data)}")


def t_partial_request_line_idle():
    s = conn(60)
    s.sendall(b"GET /hea")
    data, how, dt = recv_all(s, timeout=40)
    print(f"--- partial_request_line: {how} after {dt:.1f}s {status_lines(data)}")


def t_auth_body_slowloris():
    s = conn(60)
    body = json.dumps({"input": "hi", "response_format": "pcm"}).encode()
    s.sendall(speech(body)[:-len(body)])
    t0 = time.time()
    s.settimeout(0.1)
    got = b""
    for b in body[:-1]:
        try:
            s.sendall(bytes([b]))
        except Exception as e:
            print(f"--- auth_body_slowloris: send {e} at {time.time()-t0:.1f}s"); break
        try:
            d = s.recv(4096)
            if d: got += d
            else: break
        except socket.timeout:
            pass
        time.sleep(1)
        if got: break
    print(f"--- auth_body_slowloris: after {time.time()-t0:.1f}s {status_lines(got)} {got[-200:]}")


def t_expect_continue_unauth():
    s = conn(30)
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: 100\r\n\r\n")
    data, how, dt = recv_all(s, timeout=15)
    show("expect_continue_unauth", data, how, dt)


def t_expect_continue_auth():
    body = json.dumps({"input": "hi there", "response_format": "pcm"}).encode()
    s = conn(30)
    s.sendall(speech(body, extra=b"Expect: 100-continue\r\n")[:-len(body)])
    s.settimeout(3)
    try:
        d = s.recv(4096)
    except socket.timeout:
        d = b"(nothing)"
    s.sendall(body)
    data, how, dt = recv_all(s, timeout=5)
    print(f"--- expect_continue_auth: first={d[:40]!r}; then {status_lines(data)}")


def t_pipeline_flood():
    s = conn(60)
    n = 20000
    req = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n" * n
    t0 = time.time()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 1 << 20)
    try:
        s.settimeout(20)
        s.sendall(req)
        sent = "all sent"
    except Exception as e:
        sent = f"send stopped: {e}"
    data, how, dt = recv_all(s, limit=1 << 30, timeout=10)
    print(f"--- pipeline_flood: {sent} in {time.time()-t0:.1f}s; {len(status_lines(data))} responses, {how}")


def t_pipeline_unauth_posts():
    s = conn(60)
    one_req = b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nContent-Length: 30000\r\n\r\n" + b"x" * 30000
    t0 = time.time()
    try:
        s.settimeout(20)
        s.sendall(one_req * 200)
        sent = "all sent"
    except Exception as e:
        sent = f"send stopped: {e}"
    data, how, dt = recv_all(s, limit=1 << 30, timeout=5)
    print(f"--- pipeline_unauth_posts: {sent} {time.time()-t0:.1f}s; {len(status_lines(data))} responses {set(status_lines(data))}, {how}")


def t_many_headers():
    for n in (90, 101, 200):
        raw = b"GET /health HTTP/1.1\r\nHost: x\r\n" + b"".join(b"X-%d: y\r\n" % i for i in range(n)) + b"\r\n"
        show(f"many_headers {n}", *one(raw, timeout=5))
    raw = b"GET /health HTTP/1.1\r\nHost: x\r\nX-big: " + b"a" * 70000 + b"\r\n\r\n"
    show("huge_header 70KB", *one(raw, timeout=5))
    raw = b"GET /" + b"a" * 70000 + b" HTTP/1.1\r\nHost: x\r\n\r\n"
    show("huge_uri 70KB", *one(raw, timeout=5))


def t_chunked_edges():
    good = json.dumps({"input": "hi there", "response_format": "pcm"}).encode()
    auth = b"Authorization: Bearer " + KEY.encode() + b"\r\n"
    base = b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nConnection: close\r\n" + auth
    cases = {
        "chunked ok": base + b"Transfer-Encoding: chunked\r\n\r\n" + b"%x\r\n" % len(good) + good + b"\r\n0\r\n\r\n",
        "chunk size overflow": base + b"Transfer-Encoding: chunked\r\n\r\nffffffffffffffffffff\r\n",
        "chunk ext huge": base + b"Transfer-Encoding: chunked\r\n\r\n1;" + b"e" * 100000 + b"\r\na\r\n0\r\n\r\n",
        "CL+TE": base + b"Content-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        "double CL": base + b"Content-Length: 5\r\nContent-Length: 6\r\n\r\nhello!",
        "TE gzip,chunked": base + b"Transfer-Encoding: gzip, chunked\r\n\r\n0\r\n\r\n",
        "TE identity": base + b"Transfer-Encoding: identity\r\nContent-Length: 2\r\n\r\n{}",
        "neg CL": base + b"Content-Length: -1\r\n\r\n",
        "CL with plus": base + b"Content-Length: +2\r\n\r\n{}",
        "bare LF": b"POST /v1/audio/speech HTTP/1.1\nHost: x\nAuthorization: Bearer " + KEY.encode() + b"\nContent-Length: 2\n\n{}",
        "obs-fold auth": b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer\r\n " + KEY.encode() + b"\r\nContent-Length: 2\r\n\r\n{}",
        "HTTP/1.0 no CL": b"POST /v1/audio/speech HTTP/1.0\r\nHost: x\r\n" + auth + b"\r\n" + good,
        "HTTP/0.9": b"GET /health\r\n\r\n",
        "HTTP/2 preface": b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n",
    }
    for name, raw in cases.items():
        show(name, *one(raw, timeout=5))


def t_huge_trailers():
    """Authenticated chunked body followed by a huge trailer section."""
    good = json.dumps({"input": "hi there", "response_format": "pcm"}).encode()
    auth = b"Authorization: Bearer " + KEY.encode() + b"\r\n"
    s = conn(60)
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\n" + auth + b"Transfer-Encoding: chunked\r\n\r\n" +
              b"%x\r\n" % len(good) + good + b"\r\n0\r\n")
    t0 = time.time(); sent = 0
    s.settimeout(0.5)
    try:
        while time.time() - t0 < 15 and sent < 200 << 20:
            s.sendall(b"X-t%d: " % sent + b"a" * 1000 + b"\r\n")
            sent += 1000
    except Exception as e:
        print(f"    trailers send stopped after {sent} bytes: {e}")
    data, how, dt = recv_all(s, timeout=3)
    print(f"--- huge_trailers: sent {sent} trailer bytes in {time.time()-t0:.1f}s; {how} {status_lines(data)}")


def t_methods_paths():
    cases = [
        (b"HEAD /health", None), (b"HEAD /v1/audio/voices", None), (b"HEAD /v1/audio/voices", KEY),
        (b"OPTIONS /v1/audio/speech", None), (b"DELETE /v1/audio/speech", KEY), (b"FOO /health", None),
        (b"GET /v1/audio/speech", KEY), (b"GET /v1/audio/%76oices", KEY), (b"GET //v1/audio/voices", KEY),
        (b"GET /v1/audio/voices/", KEY), (b"GET /v1/audio/voices?x=1", KEY), (b"GET http://evil/v1/audio/voices", KEY),
        (b"GET * ", None), (b"CONNECT x:443", None), (b"GET /health#frag", None), (b"GET /v1/../health", None),
        (b"GET /\xff\xfe", None), (b"get /health", None),
    ]
    for line, key in cases:
        auth = b"Authorization: Bearer " + key.encode() + b"\r\n" if key else b""
        raw = line + b" HTTP/1.1\r\nHost: x\r\n" + auth + b"\r\n"
        data, how, dt = one(raw, timeout=3)
        print(f"--- {line!r} key={bool(key)}: {status_lines(data)} {data.split(b'\r\n\r\n',1)[-1][:120]!r}")


def t_auth_variants():
    good = json.dumps({"input": "hi", "response_format": "pcm"}).encode()
    cases = {
        "no auth": b"",
        "bearer lower": b"Authorization: bearer " + KEY.encode() + b"\r\n",
        "BEARER": b"Authorization: BEARER " + KEY.encode() + b"\r\n",
        "two spaces": b"Authorization: Bearer  " + KEY.encode() + b"\r\n",
        "tab": b"Authorization: Bearer\t" + KEY.encode() + b"\r\n",
        "trailing space": b"Authorization: Bearer " + KEY.encode() + b" \r\n",
        "basic": b"Authorization: Basic " + KEY.encode() + b"\r\n",
        "x-api-key": b"X-API-Key: " + KEY.encode() + b"\r\n",
        "prefix": b"Authorization: Bearer " + KEY.encode()[:-1] + b"\r\n",
        "longer": b"Authorization: Bearer " + KEY.encode() + b"x\r\n",
        "empty": b"Authorization: Bearer \r\n",
        "two headers bad first": b"Authorization: Bearer nope\r\nAuthorization: Bearer " + KEY.encode() + b"\r\n",
        "two headers good first": b"Authorization: Bearer " + KEY.encode() + b"\r\nAuthorization: Bearer nope\r\n",
        "non-utf8": b"Authorization: Bearer \xff" + KEY.encode() + b"\r\n",
        "comma-joined": b"Authorization: Bearer " + KEY.encode() + b",second-key\r\n",
    }
    for name, hdr in cases.items():
        raw = (b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\n" + hdr + b"Content-Length: " + str(len(good)).encode() + b"\r\n\r\n" + good)
        data, how, dt = one(raw, timeout=5, limit=200)
        print(f"--- auth {name}: {status_lines(data)}")


def t_json_cases():
    big_nest = "[" * 100000 + "]" * 100000
    cases = {
        "dup input": b'{"input":"a","input":"b","response_format":"pcm"}',
        "dup model": b'{"model":"a","model":"b","input":"x","response_format":"pcm"}',
        "unknown field": b'{"input":"a","response_format":"pcm","foo":1}',
        "nested in model": ('{"model":' + big_nest + ',"input":"a","response_format":"pcm"}').encode(),
        "nested unknown": ('{"zzz":' + big_nest + '}').encode(),
        "top array": b'[1,2,3]',
        "invalid utf8 in input": b'{"input":"\xff\xfe","response_format":"pcm"}',
        "lone surrogate": b'{"input":"\\ud800","response_format":"pcm"}',
        "nul in input": b'{"input":"a\\u0000b","response_format":"pcm"}',
        "speed huge": b'{"input":"a","response_format":"pcm","speed":1e308}',
        "speed big int": b'{"input":"a","response_format":"pcm","speed":1' + b"0" * 400 + b'}',
        "speed NaN": b'{"input":"a","response_format":"pcm","speed":NaN}',
        "speed str": b'{"input":"a","response_format":"pcm","speed":"1"}',
        "speed 0.2449": b'{"input":"hello","response_format":"pcm","speed":0.2449}',
        "speed 0.25": b'{"input":"hello","response_format":"pcm","speed":0.25}',
        "speed 4.0049": b'{"input":"hello","response_format":"pcm","speed":4.0049}',
        "speed 4": b'{"input":"hello","response_format":"pcm","speed":4}',
        "speed -0": b'{"input":"hello","response_format":"pcm","speed":-0.0}',
        "stream false": b'{"input":"hello","response_format":"pcm","stream":false}',
        "stream null": b'{"input":"hello","response_format":"pcm","stream":null}',
        "fmt mp3": b'{"input":"hello","response_format":"mp3"}',
        "no fmt": b'{"input":"hello"}',
        "voice empty": b'{"input":"hello","response_format":"pcm","voice":""}',
        "voice ja": b'{"input":"hello","response_format":"pcm","voice":"jf_alpha"}',
        "voice zf": b'{"input":"hello","response_format":"pcm","voice":"zf_xiaobei"}',
        "voice a": b'{"input":"hello","response_format":"pcm","voice":"a"}',
        "voice blend": b'{"input":"hello","response_format":"pcm","voice":"af_heart+af_bella"}',
        "voice 65": ('{"input":"hello","response_format":"pcm","voice":"' + "a" * 65 + '"}').encode(),
        "voice unicode": '{"input":"hello","response_format":"pcm","voice":"áf_heart"}'.encode(),
        "voice 4-byte first": '{"input":"hello","response_format":"pcm","voice":"\U0001F600"}'.encode(),
        "empty input": b'{"input":"","response_format":"pcm"}',
        "ws input": b'{"input":"   \\n\\t ","response_format":"pcm"}',
        "punct input": b'{"input":"...!!!???","response_format":"pcm"}',
        "input null": b'{"input":null,"response_format":"pcm"}',
        "empty body": b'',
        "trailing garbage": b'{"input":"a","response_format":"pcm"} x',
        "trailing ws": b'{"input":"hello","response_format":"pcm"}   \n',
        "BOM": b'\xef\xbb\xbf{"input":"hello","response_format":"pcm"}',
        "pause tag only": b'{"input":"[pause:5s]","response_format":"pcm"}',
        "pause tags over": ('{"input":"' + "[pause:10s] a " * 7 + '","response_format":"pcm"}').encode(),
        "pause nan": b'{"input":"a [pause:nans] b","response_format":"pcm"}',
        "pause huge": b'{"input":"a [pause:1e300s] b","response_format":"pcm"}',
    }
    for name, body in cases.items():
        data, how, dt = one(speech(body), timeout=10, limit=1 << 26)
        hdr, _, rest = data.partition(b"\r\n\r\n")
        sl = status_lines(data)
        print(f"--- json {name}: {sl[:1]} {len(rest)}B {rest[:160]!r}" if b"json" in hdr else f"--- json {name}: {sl[:1]} {len(rest)}B audio {how} {dt:.2f}s")


def t_input_limit():
    for n, ch in ((20000, "a"), (20001, "a"), (20000, "\U0001F600"), (20001, "\U0001F600"), (20000, "é")):
        body = json.dumps({"input": ch * n, "response_format": "pcm"}).encode()  # ascii escapes
        data, how, dt = one(speech(body), timeout=60, limit=1 << 28)
        print(f"--- input {n}x{ch!r} ({len(body)}B body): {status_lines(data)[:1]} {how} {dt:.2f}s {len(data)}B")
    # Body over the limit, valid JSON
    body = b'{"input":"' + b"a" * 300000 + b'","response_format":"pcm"}'
    show("body 300KB", *one(speech(body), timeout=10))


def t_conn_cap(n=None):
    """Hold N idle connections; can a fresh client still get /health?"""
    n = n or 80
    socks = []
    for i in range(n):
        try:
            socks.append(conn(5))
        except Exception as e:
            print("connect failed", i, e); break
    time.sleep(0.5)
    t0 = time.time()
    try:
        data, how, dt = one(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", timeout=30)
        print(f"--- conn_cap with {len(socks)} idle held: fresh /health {status_lines(data)} after {time.time()-t0:.1f}s")
    except Exception as e:
        print(f"--- conn_cap: fresh client error after {time.time()-t0:.1f}s: {e}")
    for s in socks: s.close()


def t_conn_cap_reopen():
    """Attacker who keeps 64 connections open, reopening each when closed (no key). Measure legit latency over 30 s."""
    stop = time.time() + 30
    def hog():
        while time.time() < stop:
            try:
                s = conn(15)
                s.sendall(b"GET /hea")  # never finish
                s.settimeout(15)
                try: s.recv(1)
                except Exception: pass
                s.close()
            except Exception:
                time.sleep(0.05)
    ts = [threading.Thread(target=hog, daemon=True) for _ in range(int(__import__("os").environ.get("HOGS","100")))]
    for t in ts: t.start()
    time.sleep(1)
    lat = []
    while time.time() < stop - 1:
        t0 = time.time()
        try:
            data, how, dt = one(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", timeout=20)
            ok = bool(status_lines(data))
        except Exception:
            ok = False
        lat.append((round(time.time() - t0, 2), ok))
    print(f"--- conn_cap_reopen: legit /health latencies {lat}")


TABLE = {k[2:]: v for k, v in globals().items() if k.startswith("t_")}
for t in TESTS:
    TABLE[t]()
