"""A 120 s Bluetooth pairing window, for the first connection of a phone over Bluetooth (README, "Phone app").

Why: the laptop's Bluetooth service reads a connecting phone's battery and other services; an iPhone protects those, so it asks
to pair, and the pairing needs a confirmation on the laptop. With nobody to give it, the pairing fails and the phone asks
again, over and over. Run this, then tap Pair on the phone when it shows its code: the laptop confirms, both remember each
other (a bond), and the prompt stops for good.

It makes this process the default Bluetooth agent for 120 s, then quits (and the agent with it). A pairing code or a service
authorisation that arrives in that window is shown here and accepted only when you type y: compare the code with the one on
the phone first, and answer n for anything you do not recognise. Needs bluetoothctl and a powered adapter (`rfkill unblock bluetooth`). Writes ~/.local/share/openhonda/pair-window.log."""
import os, re, select, subprocess, sys, time

LOG = os.path.expanduser("~/.local/share/openhonda/pair-window.log")
os.makedirs(os.path.dirname(LOG), exist_ok=True)
p = subprocess.Popen(["bluetoothctl"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)

def send(s):
    p.stdin.write(s + "\n")
    p.stdin.flush()

for cmd in ("agent KeyboardDisplay", "default-agent", "pairable on"):
    send(cmd)
    time.sleep(0.4)
ansi = re.compile(r"\x1b\[[0-9;]*[A-Za-z]|\x01|\x02")
asks = re.compile(r"\[agent\] (Confirm passkey|Authorize service|Request authorization)")
end = time.time() + 120
done = False
with open(LOG, "a") as log:
    log.write(f"--- window opened {time.strftime('%H:%M:%S')}\n")
    print("Pairing window open for 120 s: tap Pair on the phone now.", flush=True)
    while time.time() < end and not done:
        if not select.select([p.stdout], [], [], 1.0)[0]:
            continue
        line = ansi.sub("", p.stdout.readline()).strip()
        if not line:
            continue
        log.write(line + "\n")
        log.flush()
        if asks.search(line) and line.rstrip().endswith("):"):  # the question ends with "(yes/no):"
            print(f"The laptop is asked: {line}", flush=True)
            print("Does the phone show the same code (or is this the phone you are pairing)? [y/N] ", end="", flush=True)
            ready = select.select([sys.stdin], [], [], max(0.0, end - time.time()))[0]
            answer = sys.stdin.readline().strip().lower() if ready else ""
            send("yes" if answer in ("y", "yes") else "no")
            log.write(f"answered {'yes' if answer in ('y', 'yes') else 'no'}\n")
        if "Pairing successful" in line:
            done = True
    log.write(f"--- window closed {time.strftime('%H:%M:%S')} paired={done}\n")
send("quit")
try:
    p.wait(timeout=5)
except subprocess.TimeoutExpired:
    p.kill()
print("paired" if done else "no pairing completed in the window")
