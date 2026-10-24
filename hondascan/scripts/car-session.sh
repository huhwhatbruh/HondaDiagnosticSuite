#!/usr/bin/env bash
# One guided car session: the car steps, in order, logged into one folder and bundled at the end.
#
#   hondascan/scripts/car-session.sh [--adapter SPEC] [--drive-seconds N] [--no-drive] [--no-actuator] [--auto]
#
# Defaults: adapter gd101-j2534, drive watch 600 s. --auto answers every prompt "yes" (only for the simulator: -a sim:drive).
# Everything here is read-only except the actuator test (ignition ON, engine OFF), which asks first and is skipped with --no-actuator.
# Clearing codes is NOT part of the session (run `hondascan clear SYSTEM` yourself if you want it tried).
# Every step's exit code goes into SUMMARY.txt; one failing step never stops the next. Ctrl-C ends the current step only.
set -u
cd "$(dirname "$0")/.."
ADAPTER=gd101-j2534; DRIVE=600; DO_DRIVE=1; DO_ACT=1; AUTO=0
while [ $# -gt 0 ]; do
  case $1 in
    --adapter) ADAPTER=$2; shift;;
    --drive-seconds) DRIVE=$2; shift;;
    --no-drive) DO_DRIVE=0;;
    --no-actuator) DO_ACT=0;;
    --auto) AUTO=1;;
    *) echo "unknown option $1"; exit 2;;
  esac; shift
done
BIN=target/release
H="$BIN/hondascan -a $ADAPTER"
OUT=${OUT:-$HOME/car-session-$(date +%Y-%m-%d_%H%M%S)}
mkdir -p "$OUT"; SUM=$OUT/SUMMARY.txt
say() { printf '\n=== %s\n' "$*" | tee -a "$SUM"; }
ask() { [ $AUTO = 1 ] && return 0; read -r -p "$1 [y/N] " a; [ "$a" = y ] || [ "$a" = Y ]; }
# step NAME CMD...: log to NAME.out (stdout+stderr), frames are logged by the command's own --log where it has one
step() { local n=$1; shift; say "$n"; "$@" >"$OUT/$n.out" 2>&1; local rc=$?; echo "$n: exit $rc" >>"$SUM"; tail -n 4 "$OUT/$n.out"; return $rc; }

say "Preflight"
for b in hondascan abs-probe; do [ -x $BIN/$b ] || { echo "missing $BIN/$b: run 'cargo build --release' in hondascan/"; exit 1; }; done
[ -f data/hondascan.db ] || { echo "missing data/hondascan.db: cargo run --release --bin build-db"; exit 1; }
case $ADAPTER in gd101*) $BIN/hondascan ports | tee "$OUT/ports.out" | grep -qi "e327:2534\|gd101" || { echo "GD101 not found by 'hondascan ports'. Replug it (needed after any Wine/GODIAG session)."; exit 1; };; esac
echo "output folder: $OUT"; echo "git $(git rev-parse --short HEAD), $(date -Is)" >>"$SUM"
echo "Checklist: laptop on charger or car engine RUNNING, parked, handbrake on, in P, A/C off, nothing else holding the adapter (GODIAG, Wine, the app)."
ask "Ready (engine running)?" || exit 1

# ---- Phase A: engine running, parked --------------------------------------------------------------------
step 01-scan $H --log "$OUT/01-scan.frames" scan
PCM=$(awk '/CANFI/{print $1; exit}' "$OUT/01-scan.out"); PCM=${PCM:-0x0E}
echo "PCM address: $PCM (from the scan)" | tee -a "$SUM"
step 02-dtc $H --log "$OUT/02-dtc.frames" dtc
for s in PGM-FI CVT METER SRS; do step "03-live-$s" $H --log "$OUT/03-live-$s.frames" live "$s"; done
step 04-abs-probe $BIN/abs-probe $ADAPTER --log "$OUT/04-abs-probe.frames"
step 05-memory-probe $H --log "$OUT/05-memory-probe.frames" memory-probe "$PCM"

# Choose the memory format/session the probe found (first line ending in ": ok").
OKLINE=$(grep -m1 ': ok' "$OUT/05-memory-probe.out" || true)
RWARGS=()
if [ -n "$OKLINE" ]; then
  FMT=$(sed -n 's/.*format \(0x[0-9A-Fa-f]*\):.*/\1/p' <<<"$OKLINE"); SES=$(sed -n 's/^session \(0x[0-9A-Fa-f]*\).*/\1/p' <<<"$OKLINE")
  RWARGS+=(--format "$FMT"); [ "$SES" = 0x03 ] && RWARGS+=(--extended)
  echo "memory read works: session $SES format $FMT -> ramwatch uses ${RWARGS[*]}" | tee -a "$SUM"
  step 06-memory-rpm $H --log "$OUT/06-memory-rpm.frames" memory "$PCM" FFF9098E 2 "${RWARGS[@]}"
  step 07-ramwatch-once $H --log "$OUT/07-ramwatch-once.frames" ramwatch "$PCM" --csv "$OUT/07-ramwatch-once.csv" "${RWARGS[@]}"
else
  echo "no memory format answered ok (see 05-memory-probe.out; e.g. NRC 0x33 = SecurityAccess, whose seed/key attempt is in the frames): bring the logs; ramwatch skipped." | tee -a "$SUM"
fi

# ---- Phase B: the drive ---------------------------------------------------------------------------------
if [ $DO_DRIVE = 1 ] && [ -n "$OKLINE" ]; then
  echo "Drive: idle ~1 min, steady cruise at a few speeds, part- and full-throttle pulls, a lift-off, a stop. ${DRIVE} s total. Passenger operates the laptop; stop at once if the engine runs rough or a light comes on."
  if ask "Start the drive recording now?"; then
    say "08-drive (ramwatch + live PGM-FI, ${DRIVE} s)"
    $H --log "$OUT/08-drive-ramwatch.frames" ramwatch "$PCM" --watch 0.5 --max-seconds "$DRIVE" --csv "$OUT/08-drive-ramwatch.csv" "${RWARGS[@]}" >"$OUT/08-drive-ramwatch.out" 2>&1 &
    RW=$!
    sleep 3
    timeout "$((DRIVE + 5))" $H --log "$OUT/08-drive-live.frames" live PGM-FI --watch 0.5 >"$OUT/08-drive-live.out" 2>&1 &
    LV=$!
    wait $RW; echo "08-drive-ramwatch: exit $?" >>"$SUM"; wait $LV; echo "08-drive-live: exit $? (124 = ran to the time limit; a failure at once = the adapter cannot be shared, then rerun live alone)" >>"$SUM"
  fi
fi

# ---- Phase C: actuator (parked, ignition ON, engine OFF) --------------------------------------------------
if [ $DO_ACT = 1 ]; then
  echo "Actuator test: park, ENGINE OFF, ignition ON, A/C off, battery fine, nobody near the fan. The radiator fan should run ~5 s and stop by itself."
  if ask "Run radiator_fan test now?"; then
    step 09-actuator-list $H --log "$OUT/09-actuator-list.frames" actuator list
    step 10-actuator-fan $H --log "$OUT/10-actuator-fan.frames" actuator run radiator_fan 1 --seconds 5 --yes
    echo "Did the fan run? NRC 0x31 in 10-actuator-fan.out means 0x013C is not the wire DID." | tee -a "$SUM"
  fi
fi

# ---- Which ROM maps the captured lookups came from (offline tool; skipped when its Python or the A550 image is missing) --------
PY=$(cd ../hds-xref && pwd)/.venv/bin/python; IMG=${IGNIMG:-$HOME/work/pcm/a550.bin}
for CSV in "$OUT"/07-ramwatch-once.csv "$OUT"/08-drive-ramwatch.csv; do
  [ -s "$CSV" ] || continue
  if [ -x "$PY" ] && [ -f "$IMG" ]; then
    step "11-ignfit-$(basename "$CSV" .csv)" $PY ../hds-xref/tools/ignfit.py "$CSV" "$IMG" --json "${CSV%.csv}.ignfit.json"
  else
    echo "ignfit skipped for $CSV (needs $PY and $IMG; run hds-xref/tools/ignfit.py on it at home)" | tee -a "$SUM"
  fi
done

# ---- Bundle ---------------------------------------------------------------------------------------------
say "Bundle"
SESSIONS=$HOME/.local/share/hondascan/vehicles
tar -czf "$OUT.tar.gz" -C "$(dirname "$OUT")" "$(basename "$OUT")" $( [ -d "$SESSIONS" ] && echo -C "$HOME/.local/share/hondascan" vehicles ) 2>/dev/null
echo "Bring back: $OUT.tar.gz (and read $SUM)"; cat "$SUM"
