import json, socket, sys, time, threading
PORT=int(sys.argv[1]); N=int(sys.argv[2]); AFTER=float(sys.argv[3])
text=" ".join(f"Sentence {i} is right here." for i in range(40))
body=json.dumps({"input":text,"response_format":"pcm"}).encode()
def one():
    s=socket.create_connection(("127.0.0.1",PORT))
    s.sendall(b"POST /v1/audio/speech HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k\r\nContent-Length: %d\r\n\r\n"%len(body)+body)
    time.sleep(AFTER); s.close()
ts=[threading.Thread(target=one) for _ in range(N)]
for t in ts: t.start()
for t in ts: t.join()
