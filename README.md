# Honda Diagnostic Suite

**A dealer-level diagnostic tool, built from real data. Free and open source.**

HDS is the software Honda dealers use. It only runs on Windows. This does the reading side on a cheap USB adapter, and much faster. Built for Linux and Windows.

> **Confirmed on a 2015 Accord (K24W1, CVT); a 2019 and a 2021 Civic have been scanned over an ELM327 with partial results** (see [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles)). I still need more Hondas:
> **[open a vehicle report](../../issues/new?template=vehicle_report.md)**

[Download v0.6.1](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.6.1) · [Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) · [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) · [Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) · [FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ)


A generic OBD-II reader talks to the emissions computer and knows a few standard codes. A modern Honda carries a dozen or more
control units (engine, CVT, airbags, instrument cluster, ABS, steering, tyre pressure and more), and each answers in Honda's own
diagnostic language. This software speaks it.

## What is this?

- **Every system.** It finds the car's control units by asking for them, the way Honda's own tool does, in
  under a second: engine (PGM-FI), CVT, SRS airbags, instrument cluster and the rest of the 182 systems Honda knows about.
- **36,000 of Honda's own trouble codes** with Honda's descriptions and repair-help text, not just the standard P-codes.
  Stored, pending and history codes, freeze frame, emissions readiness.
- **Live data a generic reader never shows.** Misfires counted per cylinder, knock retard in degrees, the valve-timing angle,
  fuel-learning trims, the CVT's shift state: 25,882 signal definitions, each converted with Honda's own formula. Gauges, plus an
  engine-map view built from what the car reports as you drive.
- **Every drive recorded.** Codes that came and went, fuel economy, misfires, knock, and a health report against a saved baseline.
- **Native Linux.** The GODIAG GD101 in J2534 mode runs through a small C driver written for this
  project. SocketCAN, slcan, J2534, ELM327 and clones are supported too.
- **Control units found without a lookup table.** HDS decides which ECU is which by running a small rule program against what the
  car answers. That program is run here too, and the Rust version agrees with it.


## How it got here

I took HDS apart piece by piece, without installing Windows.

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

The desktop app `hondascan-gui` wraps the same scanner.

## Install and build

Each [release](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.6.1) has three archives:
`hondascan-0.6.1-linux-x86_64.tar.gz`, `hondascan-0.6.1-windows-x86_64.zip` (with the desktop app) and
`hondascan-0.6.1-windows-i686.zip` (command line only, for 32-bit J2534 DLLs). There is nothing to install for the program
itself: no Python, no Rust, no Wine. Which Linux and Windows versions run it, the udev rule, the C compiler the GD101's J2534 mode
needs and the serial and CAN set-up are on [Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started).

From source, with `rustup` (it installs the version pinned in `rust-toolchain.toml`):

```
cd hondascan && cargo run --release --bin hondascan -- -a sim scan   # scanner, against the simulated car
cd hondascan-gui && cargo run --release                               # the desktop app
```

To record your own car, run any command with `--log capture.log`; `profile-from-capture` turns the capture into a shareable
simulator profile and replaces your VIN: [Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle).

## Why should you trust it?

- **Checked by execution** against Honda's own code, not against expectations written by hand.
- **Allowlisted.** It sends a short list of diagnostic services, never to "every ECU at once", and refuses everything else inside the
  program. The ones that change something (clearing codes, four actuator tests, eight writing functions) each need a confirmation. Clearing
  codes has run successfully on a car; the actuator tests and the writing functions have never been run on one, so do not use them on yours. Details in [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety).
- **Private by default.** Nothing leaves your computer unless you choose to send a short report when something goes wrong; what it holds,
  and what it never holds, is on [Privacy](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Privacy).
- **Open source.** GPL-3.0-or-later, with the vehicle database included, so a clean clone builds and runs.

## Status

| | |
|---|---|
| **Works on a real car** | Scan of the PCM (engine and CVT answer from one address), SRS and instrument cluster; trouble codes, live data, freeze frame, emissions readiness, recorded drives, health report, JSON export, `.tar.gz` backup. 2015 Accord, GD101 in J2534 mode. Clearing trouble codes has also run successfully on a car, from a build from source: it needs a confirmation. |
| **Simulator only** | SocketCAN, slcan, ELM327 and clones, K-line (ISO 14230, with its own screen in the app). Four actuator tests (radiator fan, A/C compressor clutch, 12 V battery fan, EVAP purge solenoid) and 8 writing functions (VTC learn, first stage of the CVT inclination-sensor learn, cluster maintenance resets), all allowlisted, confirmed and handing control back on error. **Never sent to a car.** In the release, behind a confirmation at each step. A phone gateway for an iPhone exists in the source, each action behind a confirmation; the app is not published. |
| **Not covered** | ABS and power steering did not answer from the connector on this car. This PCM refuses direct memory reads (a permanent calibration bit), so engine maps come from live data, not stored tables. 113 of the 182 systems use the older K-line or Honda's own older protocol (the K-line client exists; for the older protocol there is only a session probe in the source, and an ELM327 cannot speak it). Reflash, key and immobilizer functions and the second CVT learn stage are not built. Linux is the best-tested platform; both Windows builds have run on a real Windows machine, one of them against a car. |

Field notes from the car (the misfires with no code, the ABS that never answered, the memory-read wall) are in [How it was built](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/How-It-Was-Built). Details per vehicle: [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles).

## Help wanted

- **Other Hondas, any adapter.** A scan and a code read is already valuable: [Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle).
- **Windows testing.** Both builds have run on real Windows, one against a car. A scan and a code read with the adapter you have, especially a 32-bit J2534 DLL with the 32-bit build, tells us what is left.
- **macOS.** Untried; the serial and J2534 code is the Unix path, so it may be close.
- **Bugs and ideas:** the [issue tracker](../../issues).

*We all have a right to repair. Nothing on a car should be proprietary.*

## License and disclaimer

Source: [GPL-3.0-or-later](LICENSE). The vehicle database is derived from Honda's diagnostic data and included for
interoperability; the GPL covers this project's code, not Honda's rights in the original. `hondascan/release-key.txt` opens the
release's encrypted copy of the same database. The binary releases are built from this source and carry the same license.

Independent project, **not affiliated with or endorsed by Honda Motor Co.** "Honda" and "HDS" are trademarks of their owners.
Provided as is, without warranty. Use it on your own vehicle, at your own risk.
