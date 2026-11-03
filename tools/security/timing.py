import socket, time, statistics, random
K="secret-key-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
cands={"wrong_len_short":"x","wrong_len_long":"x"*200,"same_len_wrong_first":"X"+K[1:-0] if False else "X"+K[1:],"same_len_wrong_last":K[:-1]+"B","correct_len_all_wrong":"Z"*len(K)}
s=socket.create_connection(("127.0.0.1",18880)); s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY,1)
def probe(tok):
    req=("GET /v1/audio/voices HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer "+tok+"\r\n\r\n").encode()
    t=time.perf_counter_ns(); s.sendall(req); d=b""
    while not d.endswith(b"}"):
        x=s.recv(4096)
        if not x: raise EOFError
        d+=x
    return time.perf_counter_ns()-t
res={k:[] for k in cands}
names=list(cands)
for i in range(3000):
    random.shuffle(names)
    for k in names:
        try: res[k].append(probe(cands[k]))
        except Exception:
            s.close(); s=socket.create_connection(("127.0.0.1",18880)); s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY,1)
for k,v in res.items():
    v.sort(); n=len(v)
    print(f"{k:24} n={n} p10={v[n//10]/1000:.2f}us median={v[n//2]/1000:.2f}us trimmed_mean={statistics.mean(v[n//10:-n//10])/1000:.2f}us")
