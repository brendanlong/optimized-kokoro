import json, socket, sys, time
PORT=int(sys.argv[1]); N=int(sys.argv[2]); SPEED=float(sys.argv[3]); HOLD=float(sys.argv[4])
text=" ".join("The quick brown fox jumps over the lazy dog near the riverbank today." for _ in range(60))
body=json.dumps({"input":text,"response_format":"pcm","speed":SPEED}).encode()
socks=[]
for i in range(N):
    s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096); s.connect(("127.0.0.1",PORT))
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k\r\nContent-Length: %d\r\n\r\n"%len(body)+body)
    socks.append(s)
time.sleep(HOLD)
st=[]
for s in socks:
    s.settimeout(2)
    try: st.append(s.recv(100).split(b"\r\n")[0])
    except Exception as e: st.append(str(e))
print(st[:3], len(st))
