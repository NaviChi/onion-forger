import sys, subprocess, datetime
proc = subprocess.Popen(
    ["cargo", "run", "--bin", "adapter-benchmark"],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    bufsize=1,
)
with open("adapter_10min.log", "w") as f:
    for line in iter(proc.stdout.readline, ''):
        ts = datetime.datetime.now().isoformat()
        f.write(f"[{ts}] {line}")
        f.flush()
proc.wait()
