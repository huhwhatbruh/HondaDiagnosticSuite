# Honda Diagnostic Suite

An open, native diagnostic tool for Honda vehicles, for Linux and Windows. Read-only in this release: scan, trouble codes, live data,
recorded drives and history, health report.

Project page, wiki and issue tracker: https://github.com/huhwhatbruh/HondaDiagnosticSuite

## What is in this folder

| File | What it is |
|---|---|
| `hondascan-gui` | The desktop app (`.exe` on Windows, as are the next two) |
| `hondascan-rpc` | Its backend. Keep it next to the app |
| `hondascan` | The command-line tool |
| `data/`, `drivers/` | What the program needs. Keep them next to the executables |
| `60-hondascan.rules` | Linux only: udev rule for USB adapters (below) |
| `SHA256SUMS`, `manifest.json`, `VERSION` | Checksums and version of every file here |
| `OFL-Noto.txt` | Licence of the app's font |

## Set-up

Nothing else needs installing for the program itself: no Python, no Rust, no Wine.

**Linux**

- **A desktop with OpenGL and X11 or Wayland** for the app window. The command-line tool needs none of it.
- **USB adapters (GODIAG GD101 and others):**
  `sudo cp 60-hondascan.rules /etc/udev/rules.d/ && sudo udevadm control --reload`, then replug the adapter. Without it
  ModemManager can grab the adapter.
- **GODIAG GD101 in J2534 mode:** a C compiler (`cc`, `gcc` or `clang`) installed once. The program builds its small
  native driver from `drivers/gd101_j2534.c` the first time.
- **Serial and ELM327 adapters:** your user needs the serial group (`dialout` on Debian and Ubuntu, `uucp` on Arch).
- **SocketCAN adapters:** bring the interface up at 500 kbit/s: `sudo ip link set can0 up type can bitrate 500000`.

**Windows (64-bit)**

- **Serial and ELM327 adapters** (USB, or Bluetooth once paired): install the adapter's USB-serial driver as its maker
  documents, find its COM port in Device Manager, and name it: `hondascan.exe -a elm:COM5 scan`. The app lists the ports it
  finds. Close any other program that has the port open (a terminal program, another copy of hondascan): Windows allows one at a time.
- **J2534 devices (GODIAG GD101 and others):** install the maker's driver package, which registers a J2534 DLL.
  `hondascan.exe -a gd101-j2534 scan` uses the registered GODIAG one; `-a j2534:NAME` picks any registered device by name, or
  `-a j2534:C:\path\to\device.dll`. This build is 64-bit and loads only 64-bit DLLs; many J2534 DLLs are 32-bit and
  will say so when loading.
- **Not on Windows:** SocketCAN, the phone gateway's Bluetooth channel and its Bonjour announcement (the gateway itself works
  over Wi-Fi at an address you type). Windows has had far less testing than Linux: it has not been run against a car.

`./hondascan selfcheck` reports what is present and what is missing.

## First run

```
./hondascan -a sim scan                 # the built-in simulated car, no hardware (hondascan.exe on Windows)
./hondascan -a gd101-j2534 scan         # a real car (name your adapter after -a)
./hondascan -a gd101-j2534 dtc          # trouble codes
./hondascan-gui                         # the desktop app
```

Adapter names and what has been tested: https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters

## Safety

This release only reads. Confirmed on one car so far (2015 Honda Accord 2.4, GODIAG GD101 in J2534 mode); everything
else is untested, and the program says "untested" where it is. Please report what you see.

This is an independent project and is not affiliated with or endorsed by Honda.
