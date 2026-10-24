"""Runs hondascan-rpc on a simulated car with the phone gateway on (Wi-Fi 0.0.0.0:8765 and Bluetooth), without the desktop app,
so the phone app can be tried and measured with no car. The simulated drive (`sim:drive`) moves through a 140 s cycle: idle,
accelerate to 90 km/h, cruise, brake. Usage: python3 -I hondascan/scripts/phone-sim-gateway.py [adapter]  (default sim:drive).
The pairing code is the one in ~/.config/openhonda/phone-token (made here if there is none). Stop it with Ctrl-C."""
import json, os, secrets, subprocess, sys, time

repo = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
adapter = sys.argv[1] if len(sys.argv) > 1 else "sim:drive"
cfg = os.path.expanduser("~/.config/openhonda")
os.makedirs(cfg, exist_ok=True)
path = cfg + "/phone-token"
def saved():
    try:
        with open(path) as f:
            return f.read().strip()
    except FileNotFoundError:
        return ""
if not saved():
    with open(path, "w") as f:
        f.write("".join(secrets.choice("ABCDEFGHJKLMNPQRSTUVWXYZ") for _ in range(4)))
os.chmod(path, 0o600)
tok = saved()
env = dict(os.environ, HONDASCAN_PHONE="0.0.0.0:8765", HONDASCAN_PHONE_TOKEN=tok)
p = subprocess.Popen([repo + "/hondascan/target/release/hondascan-rpc"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=sys.stderr, env=env, text=True, cwd=repo + "/hondascan")
for i, (m, a) in enumerate([("connect", {"adapter": adapter}), ("scan", {})], 1):
    p.stdin.write(json.dumps({"id": i, "method": m, "params": a}) + "\n")
p.stdin.flush()
print("gateway up on", adapter, "; pairing code", tok, flush=True)
try:
    while p.poll() is None:
        time.sleep(5)
except KeyboardInterrupt:
    p.stdin.close()
    p.wait(timeout=10)
