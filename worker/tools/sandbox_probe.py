"""What the worker can reach inside its sandbox: uid, capabilities, devices, mounts, writable paths,
network, host sockets, exec on /tmp, GPUs. Run with `worker/run.sh probe DIR MODELS` (from PR #11 review)."""
import os, socket, glob, subprocess, errno
print("uid/gid", os.getuid(), os.getgid(), "groups", os.getgroups())
for l in open("/proc/self/status"):
    if l.startswith(("Cap", "NoNewPrivs", "Seccomp")): print(l.strip())
print("--- /dev"); print(sorted(os.listdir("/dev")))
print("dri:", glob.glob("/dev/dri/*"))
for d in sorted(glob.glob("/dev/nvidia*") + glob.glob("/dev/dri/*")):
    try:
        fd = os.open(d, os.O_RDWR); os.close(fd); print("open ok ", d)
    except OSError as e: print("open err", d, errno.errorcode.get(e.errno), e.strerror)
print("--- mounts")
for l in open("/proc/mounts"): print(l.strip())
print("--- write tests")
for p in ["/", "/app", "/models", "/run/kokoro", "/tmp", "/etc", "/home/worker"]:
    try:
        fn = os.path.join(p, ".probe"); open(fn, "w").write("x"); os.unlink(fn); print("writable", p)
    except OSError as e: print("not writable", p, e.strerror)
print("--- network")
print("ifaces", os.listdir("/sys/class/net") if os.path.exists("/sys/class/net") else "no sysfs net")
for host in [("1.1.1.1", 53), ("127.0.0.1", 22)]:
    s = socket.socket(); s.settimeout(3)
    try: s.connect(host); print("connect OK", host)
    except OSError as e: print("connect err", host, e)
print("--- host UDS")
for p in ["/run/nvidia-persistenced/socket"]:
    s = socket.socket(socket.AF_UNIX)
    try: s.connect(p); print("UDS connect OK", p)
    except OSError as e: print("UDS err", p, e)
print("--- exec on /tmp")
open("/tmp/x.sh","w").write("#!/bin/sh\necho exec-from-tmp-ok\n"); os.chmod("/tmp/x.sh", 0o755)
print(subprocess.run(["/tmp/x.sh"], capture_output=True, text=True).stdout.strip())
print("--- gpus")
print(subprocess.run(["nvidia-smi", "-L"], capture_output=True, text=True))
import torch; print("torch cuda", torch.cuda.is_available(), torch.cuda.device_count())
print("env NVIDIA", {k: v for k, v in os.environ.items() if "NVIDIA" in k or "CUDA" in k})
