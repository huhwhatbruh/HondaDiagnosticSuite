#!/bin/sh
# Starts the desktop app with the phone gateway on, for Wi-Fi and Bluetooth (README, "Phone app"). The pairing code is four
# letters, made once and kept in ~/.config/openhonda/phone-token; the addresses printed below are the ones to type into the
# phone app (or pick the laptop from its list: it announces itself over Bonjour). Needs the release builds of hondascan-rpc and
# hondascan-gui (`cargo build --release` in hondascan/ and hondascan-gui/).
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
dir="$HOME/.config/openhonda"
mkdir -p "$dir"
if [ ! -s "$dir/phone-token" ]; then
  LC_ALL=C tr -dc 'ABCDEFGHJKLMNPQRSTUVWXYZ' < /dev/urandom | head -c 4 > "$dir/phone-token"
fi
chmod 600 "$dir/phone-token"
port=8765
# The gateway listens on every interface by default, so the phone still finds the laptop after the laptop changes network (a
# hotspot). To listen on one address only (the laptop's address on your home network), run with PHONE_BIND=192.168.1.20.
bind="${PHONE_BIND:-0.0.0.0}"
echo "Pairing code: $(cat "$dir/phone-token")"
echo "Laptop addresses (port $port):"
ip -4 -o addr show scope global | awk -v port="$port" '{split($4, a, "/"); print "  " a[1] ":" port "   (" $2 ")"}'
if command -v rfkill >/dev/null 2>&1 && rfkill list bluetooth 2>/dev/null | grep -q "Soft blocked: yes"; then
  echo "Bluetooth is switched off in software: run  rfkill unblock bluetooth  for the Bluetooth link."
fi
export HONDASCAN_PHONE="$bind:$port" HONDASCAN_PHONE_TOKEN="$(cat "$dir/phone-token")"
export HONDASCAN_RPC="$repo/hondascan/target/release/hondascan-rpc"
exec "$repo/hondascan-gui/target/release/hondascan-gui" "$@"
