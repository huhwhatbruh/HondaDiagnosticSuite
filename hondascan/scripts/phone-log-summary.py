"""Wi-Fi against Bluetooth, from the app's polls (by their `link` field) and from the gateway's own log (by peer: an IP is Wi-Fi, `ble` is Bluetooth; 127.0.0.1 is left out, it is the laptop itself)."""
import json, os, statistics

d = os.path.expanduser("~/.local/share/openhonda")
app = [json.loads(l) for l in open(d + "/phone-client.jsonl")]
gw = [json.loads(l) for l in open(d + "/phone-gateway.jsonl")]

def pct(xs, p):
    xs = sorted(xs)
    return xs[int((len(xs) - 1) * p)] if xs else None

def line(name, rtt, extra=""):
    if not rtt:
        print(f"  {name}: no samples"); return
    print(f"  {name}: {len(rtt)} polls; median {statistics.median(rtt):.0f} ms, p90 {pct(rtt, .9):.0f}, p99 {pct(rtt, .99):.0f}, worst {max(rtt):.0f}; over 150 ms: {100 * sum(1 for x in rtt if x > 150) // len(rtt)}% {extra}")

print("PHONE'S VIEW (round trip as the app measures it, successful polls)")
polls = [r for r in app if r["ev"] == "poll"]
for link in ("wifi", "bluetooth"):
    ok = [r for r in polls if r.get("link") == link and r["ok"]]
    bad = [r for r in polls if r.get("link") == link and not r["ok"]]
    size = statistics.median(r["bytes"] for r in ok) if ok else 0
    line(link, [r["rtt_ms"] for r in ok], f"; failed {len(bad)}; reply {size:.0f} bytes")
ev = [(round(r["t"] % 1000), r["ev"], r.get("to") or r.get("status") or r.get("why")) for r in app if r["ev"] in ("mode", "route", "ble")]
print("  mode / route / ble events (last 12):", ev[-12:])
print("  state changes (last 6):", [(round(r["t"] % 1000), r["to"][:60]) for r in app if r["ev"] == "state"][-6:])

print("\nLAPTOP'S VIEW (the time the laptop spent on each snapshot request, which over Bluetooth includes sending the reply)")
for name, rows in (("wifi", [r for r in gw if r["ev"] == "req" and r.get("peer") not in (None, "ble", "127.0.0.1") and r["route"] == "/v1/snapshot"]),
                   ("bluetooth", [r for r in gw if r["ev"] == "req" and r.get("peer") == "ble" and r["route"] == "/v1/snapshot"])):
    if rows:
        ms = [r["ms"] for r in rows]
        raw = statistics.median(r.get("raw_bytes", r["bytes"]) for r in rows)
        sent = statistics.median(r["bytes"] for r in rows)
        print(f"  {name}: {len(rows)} requests; handling median {statistics.median(ms):.0f} ms, p90 {pct(ms, .9):.0f}, worst {max(ms):.0f}; reply {sent:.0f} bytes (from {raw:.0f})")
ble = [r for r in gw if r["ev"] == "ble"]
print("  bluetooth connections accepted:", sum(1 for r in ble if r["state"] == "connected"), "; last events:", [r["state"] for r in ble][-6:])
