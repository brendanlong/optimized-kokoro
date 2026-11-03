import json, struct, random, os, copy
import sys; src=open(sys.argv[1],"rb").read()  # usage: stgen.py TAGGER.safetensors OUTDIR
n=struct.unpack("<Q",src[:8])[0]; H=json.loads(src[8:8+n]); D=src[8+n:]
out=sys.argv[2]
def w(name, header=None, data=D, raw=None, hlen=None):
    if raw is None:
        hb=json.dumps(header if header is not None else H).encode() if not isinstance(header,bytes) else header
        raw=struct.pack("<Q", len(hb) if hlen is None else hlen)+hb+data
    open(f"{out}/{name}","wb").write(raw)
def edit(name, f, data=D):
    h=copy.deepcopy(H); f(h); w(name,h,data)
w("00_original")
for cut in (0,4,8,100,2000,8+n,8+n+100,len(src)//2,len(src)-1):
    w(f"trunc_{cut}", raw=src[:cut])
w("hlen_huge", raw=struct.pack("<Q",2**64-1)+src[8:])
w("hlen_mid", raw=struct.pack("<Q",n//2)+src[8:])
w("hlen_zero", raw=struct.pack("<Q",0)+src[8:])
w("hdr_array", b"[1,2,3]")
w("hdr_deep", b"["*10000+b"]"*10000)
w("hdr_garbage", b"\xff\xfe\x00garbage")
w("hdr_empty_obj", b"{}")
edit("mixG_shape_empty", lambda h: h["mix.G"].update(shape=[]))
edit("mixG_width0", lambda h: h["mix.G"].update(shape=[0]))
edit("mixG_width_huge", lambda h: h["mix.G"].update(shape=[2**62]))
edit("embed0_rows0", lambda h: h["embed.0.E"].update(shape=[0,96], data_offsets=[0,0]))
edit("embed0_shape1d", lambda h: h["embed.0.E"].update(shape=[5000]))
edit("embed0_rows_huge", lambda h: h["embed.0.E"].update(shape=[2**40,96]))
edit("embed0_overflow", lambda h: h["embed.0.E"].update(shape=[2**62,2**62]))
edit("embed0_off_rev", lambda h: h["embed.0.E"].update(data_offsets=[1920000,0]))
edit("embed0_off_past", lambda h: h["embed.0.E"].update(data_offsets=[0,10**12]))
edit("embed0_off_neg", lambda h: h["embed.0.E"].update(data_offsets=[-1,5]))
edit("embed0_f16", lambda h: h["embed.0.E"].update(dtype="F16"))
edit("missing_tagger_b", lambda h: h.pop("tagger.b"))
edit("tagger_W_0", lambda h: h["tagger.W"].update(shape=[0,96], data_offsets=[0,0]))
edit("extra_encode4", lambda h: h.update({"encode.4.W": h["encode.3.W"]}))
edit("drop_encode3", lambda h: [h.pop(k) for k in list(h) if k.startswith("encode.3.")])
edit("meta_extra", lambda h: h.update({"__metadata__": {"a": "b"}}))
edit("shape_float", lambda h: h["mix.G"].update(shape=[96.5]))
edit("unknown_field", lambda h: h["mix.G"].update(extra=1))
edit("aliased_offsets", lambda h: [h[k].update(data_offsets=[0, h[k]["data_offsets"][1]-h[k]["data_offsets"][0]]) for k in h])
# Data corruption
nan=struct.pack("<f", float("nan"))
w("data_all_nan", H, nan*(len(D)//4))
w("data_all_inf", H, struct.pack("<f", float("inf"))*(len(D)//4))
w("data_zero", H, b"\0"*len(D))
random.seed(5)
for i in range(5):
    d=bytearray(D)
    for _ in range(2000):
        d[random.randrange(len(d))]=random.randrange(256)
    w(f"data_flip_{i}", H, bytes(d))
# random header byte flips
hb=json.dumps(H).encode()
for i in range(20):
    b=bytearray(hb)
    for _ in range(3): b[random.randrange(len(b))]=random.randrange(32,127)
    w(f"hdr_flip_{i}", bytes(b))
