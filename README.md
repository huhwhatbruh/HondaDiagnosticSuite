# Honda Diagnostic Suite

**An open, native Linux diagnostic tool for Honda vehicles.**
Read trouble codes and live data, record every drive, and keep a health history of your car, with no Windows, no Wine and no
dealer software.

*We all as consumers have a right to repair. Nothing on a car should be proprietary.*

[Get started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) · [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) · [Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) · [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety) · [FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ)

</div>

---

> ### I need people with other Honda vehicles
> Right now the tool has been proven on one car. Every new model, engine and year it runs on makes it better for everyone.
> **[Open a vehicle report](../../issues/new?template=vehicle_report.md)**. The source is in this repository

## What it does

| | |
|---|---|
| **Scan** | Finds every control unit on the car (engine/PGM-FI, transmission, ABS, SRS, meter and more) in under a second. |
| **Trouble codes** | Reads stored, pending and history codes with plain-English descriptions, plus freeze frame and emissions readiness. |
| **Live data** | Named values with units, gauges, and an engine-map view built from what the car reports while you drive. |
| **Drives and history** | Every session is recorded per car: codes that appeared and cleared, trends across drives, fuel economy, misfires and knock. |
| **Health report** | One page that compares the car today with a saved reference snapshot: new codes, resolved codes, tests not yet run. |
| **Yours to keep** | Plain JSON export and a `.tar.gz` backup of everything the app has recorded. |
| **Many adapters** | SocketCAN, slcan, J2534, ELM327 and clones, and the GODIAG GD101 through a native Linux driver. |

## Status

| | |
|---|---|
| **Confirmed on a real car** | 2015 Honda Accord 2.4 (K24W1, CVT), GODIAG GD101 in J2534 mode: a full scan, codes, live data and a 48-minute recorded drive |
| **Platform** | Linux (only the Linux build has been run) |
| **Binary release** | [v0.1.1](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.1.1), Linux x86_64, read-only: scan, codes, live data, history and reports. Two builds, see below |
| **Source** | In this repository as of v0.1.1: `hondascan/` (scanner, adapters, GODIAG GD101 J2534 driver) and `hondascan-gui/` (desktop app). **The vehicle database is generated from vendor data that is not published**, so a full build is not possible from this tree alone.|
| **Not in the first release** | Clearing codes, bi-directional (actuator) tests and writing functions exist in development but have **never been run on a car**, so they are not offered |

The honest summary: it reads the systems that have been checked, and says "untested" everywhere else. See [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles).

## What is done, and what is not

Everything below was measured on one car, a 2015 Accord 2.4 (K24W1, CVT), through a GODIAG GD101 in J2534 mode. Where something
has not run on a car, this says so.

**Works on a real car**
- A full scan: the PCM (engine and CVT answer from the same address), the SRS unit and the instrument cluster.
- Trouble codes with descriptions, live data with units, freeze frame and emissions readiness.
- Recording: a 48-minute drive and a 33-minute drive home, stored per car, with trends, fuel economy, misfire and knock views.
- Health report, JSON export, `.tar.gz` backup, and a baseline you can freeze and compare against later.
- The GD101 through a native Linux driver, with no Wine.

**Built and tested only against simulated hardware**
- SocketCAN, slcan, ELM327 and clones, and the older K-line protocol (ISO 14230). The simulators enforce the real hardware's limits,
  but none of these adapters has been on a car yet.
- Clearing trouble codes, and a handful of actuator tests (output tests that move a part, such as a fan or a solenoid). Both are
  allowlisted, need an explicit confirmation, hand control back to the ECU on any error, and have **never been sent to a car**.
- Eight writing functions that Honda's own tool offers: the engine's variable-valve-timing learn, the CVT inclination-sensor learn
  (the first of its two stages) and the instrument cluster's maintenance-reminder resets. Same rules, same status: **never run on a
  car**, so they are not in the release.
- A phone gateway (`hondascan/src/phone.rs`): a read-only view of the live data for an iPhone over Wi-Fi or Bluetooth. It cannot
  reach a write, a clear or an actuator test. The pairing code is a short code on your own network and is not encrypted. The iPhone
  app itself is not published yet.

**Known limits**
- The ABS and the power-steering unit did not answer from the diagnostic connector on this car, so they are not covered.
- This PCM refuses direct memory reads by a calibration setting, so live RAM watching cannot work on it. Engine maps are built from
  what the car reports while you drive, not read from its stored tables.
- 113 of the 182 systems Honda's tool knows are on the older K-line or Honda's own older protocol: body modules,
  older models. The K-line client exists; Honda's older protocol has no driver at all, because an ELM327 cannot speak it.
- Only the Linux build has been run. Windows and macOS have not been tried.

**Not built**
- ECU reflash, key and immobilizer functions, and the second stage of the CVT learn.

The vehicle database is generated from Honda's own data and is not in this repository. The binary release includes one.

## Download and requirements

Two archives are attached to each release. They hold the same program; they differ only in the system they were built on.

| Archive | Built on | Runs on |
|---|---|---|
| `hondascan-0.1.1-linux-x86_64-glibc2.35.tar.gz` | Ubuntu 22.04 | **Use this one.** Tested on Ubuntu 22.04. Any Linux x86_64 with glibc 2.35 or newer should run it (Debian 12 and later, Fedora, Arch: not tested) |
| `hondascan-0.1.1-linux-x86_64-glibc2.43.tar.gz` | Arch-based, glibc 2.43 | Only distributions that new (current Arch, CachyOS, Fedora rawhide). Older systems refuse to start it |

Each archive has a matching `.sha256` file.

Nothing needs installing for the app itself: no Python, no Rust, no Wine. What the machine needs:

- A desktop with OpenGL (EGL) and X11 or Wayland: Mesa and `libxkbcommon`, which desktop distributions normally include. The window itself has only been run on the author's machine.
- **GODIAG GD101 or another USB adapter:** copy the `60-hondascan.rules` file from the archive into `/etc/udev/rules.d/`, run
  `sudo udevadm control --reload` and replug the adapter. Without it ModemManager can grab the adapter.
- **GD101 in J2534 mode:** a C compiler (`cc`, `gcc` or `clang`) must be installed once, to build the small native driver.
- **Serial and ELM327 adapters:** your user needs the serial group (`dialout` on Debian and Ubuntu, `uucp` on Arch).
- **SocketCAN adapters:** the interface must be up (`sudo ip link set can0 up type can bitrate 500000`).

Step by step: [Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started).

## Quick look

```
$ hondascan -a socketcan:can0 scan        # find the control units
$ hondascan -a socketcan:can0 dtc         # trouble codes of every system
$ hondascan -a socketcan:can0 live PGM-FI # live engine values with units
$ hondascan -a sim scan                   # no car? try the built-in simulated one
$ hondascan report                        # health report (HTML + JSON)
```

The desktop app (`hondascan-gui`) wraps the same scanner: a connect bar, a car overview, per-system pages, live gauges,
history and reports.

## Why trust it

- **Checked by execution.** Protocol and data decoding are tested by running them against reference behaviour and comparing
  the results, not against expectations written by hand.
- **Strict simulators.** The simulated bus, ELM and J2534 test doubles enforce the real hardware's limits (filters,
  transmit queues, timeouts), so a bug shows up on the bench rather than on your car.
- **Read-only by default, allowlisted always.** The scanner can only send a short list of diagnostic services, and never to
  "every ECU at once". Details in [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety).
- **Nothing phones home.** The scanner talks to your adapter and writes to your own disk, and that is all.

## Documentation

The [wiki](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki) holds the details so this page stays short:

[Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) ·
[Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) ·
[Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) ·
[Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety) ·
[Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle) ·
[FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ) ·
[Roadmap](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Roadmap)

## Help wanted

- **Other Hondas.** Any model or year, any adapter. See [Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle); a scan and a
  trouble-code read is already valuable.
- **Other platforms.** Windows and macOS builds have not been tried.
- **Bug reports and ideas** go in the [issue tracker](../../issues).

## License

The source code in this repository is free software under the [GNU General Public License v3.0 or later](LICENSE): you may use,
study, modify and share it. The vehicle database that the program reads is generated from data that is not part of this
repository and is not covered by this license. The v0.1.1 binary release was published under the earlier freeware terms; they
remain what governs that download.

## Disclaimer

This is an independent project, **not affiliated with or endorsed by Honda Motor Co.** "Honda", "HDS" and model names are
trademarks of their owners and are used only to say what the tool works with. The software is provided as is, without
warranty. Diagnostics read data from your vehicle's computers; use it on your own vehicle and at your own risk.
