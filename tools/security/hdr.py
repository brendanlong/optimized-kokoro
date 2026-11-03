import socket,sys,time
for size in [65000, 66000, 70000, 100000, 131072, 200000, 1000000, 10000000]:
    s=socket.create_connection(("127.0.0.1",18880)); s.settimeout(10)
    req=b"GET /health HTTP/1.1\r\nConnection: close\r\nHost: x\r\nX-big: "+b"a"*size+b"\r\n\r\n"
    t=time.time()
    try:
        s.sendall(req); r="sent all"
    except Exception as e: r=f"send err {e}"
    try: d=s.recv(200)
    except Exception as e: d=str(e).encode()
    print(size, r, d[:50], round(time.time()-t,2))
    s.close()
# many headers fit 64K? 100 headers each 600B = 60KB
