# Honda Diagnostic Suite

**Honda's dealer diagnostic tool, rebuilt for Linux. Free and open source.**

HDS is the software Honda dealers use. It only runs on Windows and needs Honda's own interface, and the dealer tooling that does
this costs thousands of dollars. This does the reading side on a cheap USB adapter, on Linux and more operating systems to come.

> **Proven on a 2015 Accord (K24W1, CVT) so far.** I need people with other Hondas:
> **[open a vehicle report](../../issues/new?template=vehicle_report.md)**

[Download v0.1.1](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.1.1) · [Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) · [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) · [Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) · [FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ)


A generic OBD-II reader talks to the emissions computer and knows a few standard codes. A modern Honda carries a dozen or more
control units (engine, CVT, airbags, instrument cluster, ABS, steering, tyre pressure and more), and each answers in Honda's own
diagnostic language. Each unit by name.

## What is this?

- **Every system** It finds the car's control units by asking for them, the way Honda's own tool does, in
  under a second: engine (PGM-FI), CVT, SRS airbags, instrument cluster and the rest of the 182 systems Honda knows about.
- **36,000 of Honda's own trouble codes** with Honda's descriptions and repair-help text, not just the standard P-codes.
  Stored, pending and history codes, freeze frame, emissions readiness.
- **Live data a generic reader never shows.** Misfires counted per cylinder, knock retard in degrees, the valve-timing angle,
  fuel-learning trims, the CVT's shift state: 25,882 signals, each converted with Honda's own formula. Gauges, plus an
  engine-map view built from what the car reports as you drive.
- **Every drive recorded.** Codes that came and went, fuel economy, misfires, knock, and a health report against a saved baseline.
- **Native Linux** The GODIAG GD101 in J2534 mode runs through a small C driver written for this
  project. SocketCAN, slcan, J2534, ELM327 and clones are supported too.
- **Honda's own database.** 182 systems, 36,000 trouble codes, 35,397 help texts, 16,031 ECU identifiers, 25,882 CAN signal
  definitions and 3,607 unit converters. It is in the repo, so a clone builds and runs.
- **Control units found without a lookup table.** HDS decides which ECU is which by running a small rule program against what the
  car answers. That program is run here too, and the Rust version agrees with it.


## How it got here

I took HDS apart bit by bit completely without ever having to install Microslop WindowsSlopware.

- **Honda's diagnostic logic is bytecode.** I wrote a decompiler for it: 139 modules, 10,021 functions, no opcode left undecoded.
  Every in-scope function was run next to Honda's own interpreter on 127,313 inputs. **9,660 compared, 0 mismatches.**
- **All 3,607 unit converters were run against Honda's own conversion code**: 67,553 runs, 0 mismatches.
- **The bus was read from the interface's firmware** with a disassembler (addressing, ISO-TP framing, bit rate, CAN address
  table), not guessed from captures.
- **The GD101's Windows driver was traced** and rewritten as a Linux J2534 library, tested against a fake that has the real
  device's 10-frame transmit queue.
- **The simulators are stricter than the hardware**, so bugs show up on the bench and not on your car.


## Quick start

```
$ hondascan -a socketcan:can0 scan         # find the control units
$ hondascan -a socketcan:can0 dtc          # trouble codes, every system
$ hondascan -a socketcan:can0 live PGM-FI  # live engine values with units
$ hondascan -a sim scan                    # no car? use the built-in simulated one
```

The desktop app `hondascan-gui` wraps the same scanner. Download, udev rule, adapter setup: [Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started).

## Download and requirements

Two archives are attached to [each release](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.1.1), each with
a `.sha256` file. They hold the same program and differ only in the system they were built on.

| Archive | Runs on |
|---|---|
| `hondascan-0.1.1-linux-x86_64-glibc2.35.tar.gz` | **Use this one.** Tested on Ubuntu 22.04. Any Linux x86_64 with glibc 2.35 or newer should run it (Debian 12+, Fedora, Arch: not tested) |
| `hondascan-0.1.1-linux-x86_64-glibc2.43.tar.gz` | Only very new systems (current Arch, CachyOS, Fedora rawhide). Older ones refuse to start it |

Nothing to install for the app itself: no Python, no Rust, no Wine. The machine needs:

- A desktop with OpenGL (EGL) and X11 or Wayland (Mesa and `libxkbcommon`, which desktop distributions normally include).
- **GODIAG GD101 or another USB adapter:** copy `60-hondascan.rules` from the archive into `/etc/udev/rules.d/`, run
  `sudo udevadm control --reload` and replug the adapter, or ModemManager can grab it.
- **GD101 in J2534 mode:** a C compiler (`cc`, `gcc` or `clang`), once, to build the native driver.
- **Serial and ELM327 adapters:** your user needs the serial group (`dialout` on Debian and Ubuntu, `uucp` on Arch).
- **SocketCAN adapters:** the interface must be up (`sudo ip link set can0 up type can bitrate 500000`).

## Build from source

You need Rust 1.92 or newer (`rustup`) and, for the GD101's J2534 mode, a C compiler. The vehicle database is in the repository,
so there is nothing to generate first.

```
cd hondascan && cargo run --release --bin hondascan -- -a sim scan   # scanner, against the simulated car
cd hondascan && cargo build --release                                 # also builds hondascan-rpc, the app's backend
cd hondascan-gui && cargo run --release                               # the desktop app
```

The 3D car on the welcome screen is not in this repository, so that screen is empty here. Everything else works.

## Why trust it

- **Checked by execution** against Honda's own code, not against expectations written by hand.
- **Read-only by default, allowlisted always.** The scanner sends only a short list of diagnostic services, never to "every ECU
  at once". Details in [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety).
- **Nothing phones home.** It talks to your adapter and writes to your own disk.

## Honest status

| | |
|---|---|
| **Works on a real car** | Scan of the PCM (engine and CVT answer from one address), SRS and instrument cluster; trouble codes, live data, freeze frame, emissions readiness, recorded drives, health report, JSON export, `.tar.gz` backup. 2015 Accord, GD101 in J2534 mode. |
| **Simulator only** | SocketCAN, slcan, ELM327 and clones, K-line (ISO 14230). Clearing codes, a handful of actuator tests (fan, solenoid) and 8 writing functions (VTC learn, first stage of the CVT inclination-sensor learn, cluster maintenance resets), all allowlisted, confirmed and handing control back on error. **Never sent to a car, so not in the release.** A read-only phone gateway for an iPhone exists in the source; the app is not published. |
| **Not covered** | ABS and power steering did not answer from the connector on this car. This PCM refuses direct memory reads (a permanent calibration bit), so engine maps come from live data, not stored tables. 113 of the 182 systems use the older K-line or Honda's own older protocol (the K-line client exists; the older protocol has no driver, an ELM327 cannot speak it). Reflash, key and immobilizer functions and the second CVT learn stage are not built. Only Linux has been run. |

Field notes from the car (the misfires with no code, the ABS that never answered, the memory-read wall) are in [How it was built](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/How-It-Was-Built). Details per vehicle: [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles).

## Help wanted

- **Other Hondas, any adapter.** A scan and a code read is already valuable: [Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle).
- **Windows and macOS builds.** Not tried.
- **Bugs and ideas:** the [issue tracker](../../issues).

*We all have a right to repair. Nothing on a car should be proprietary.*

## License and disclaimer

Source: [GPL-3.0-or-later](LICENSE). The vehicle database is derived from Honda's diagnostic data and included for
interoperability; the GPL covers this project's code, not Honda's rights in the original. `hondascan/release-key.txt` opens the
release's encrypted copy of the same database. The v0.1.1 binary stays under the earlier freeware terms.

Independent project, **not affiliated with or endorsed by Honda Motor Co.** "Honda" and "HDS" are trademarks of their owners.
Provided as is, without warranty. Use it on your own vehicle, at your own risk.
