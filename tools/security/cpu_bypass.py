import socket, sys, time, threading
PORT=int(sys.argv[1]); KEY=sys.argv[2]; N=int(sys.argv[3]); WAIT=float(sys.argv[4])
import os; body=open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "worst.json"),"rb").read()
req=(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer "+KEY.encode()+b"\r\nContent-Length: %d\r\n\r\n"%len(body))+body
res=[]
def one():
    s=socket.create_connection(("127.0.0.1",PORT)); s.sendall(req); s.settimeout(WAIT)
    try: d=s.recv(100); res.append(d.split(b"\r\n")[0])
    except socket.timeout: res.append(b"(no response yet)")
    s.close()
for rnd in range(int(sys.argv[5]) if len(sys.argv)>5 else 1):
    ts=[threading.Thread(target=one) for _ in range(N)]
    for t in ts: t.start()
    for t in ts: t.join()
from collections import Counter; print(Counter(res))
