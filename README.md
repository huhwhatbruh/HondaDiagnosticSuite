# Honda Diagnostic Suite

**The Honda dealer scan tool, rebuilt in the open, for Linux.**

A generic OBD-II reader talks to the emissions computer and knows a few standard codes. A modern Honda carries a dozen or more
control units (engine, CVT, airbags, instrument cluster, ABS, steering, tyre pressure and more), and each answers in Honda's own
diagnostic language. This tool speaks it, to each unit by name, with no Windows, no Wine and no dealer software.

- **Every system, not just the engine.** It finds the car's control units by asking for them, the way Honda's own tool does, and
  reads each one: engine (PGM-FI), CVT, SRS airbags, instrument cluster and the rest of the 182 systems Honda knows about.
- **Honda's own trouble codes, 36,000 of them,** with Honda's descriptions and its repair-help text, not just the standard
  P-codes.
- **Live data a generic reader never shows.** Misfires counted per cylinder, knock retard in degrees, the valve-timing angle,
  fuel-learning trims, the CVT's shift state: 25,882 signals, each converted with Honda's own formula. Every converter was checked
  against Honda's own code with 0 mismatches.
- **Honda's tool, read from the inside.** Its diagnostic logic was decompiled and every function run next to Honda's own
  interpreter: 127,313 comparisons, 0 mismatches.
- **A health history of your car.** Every drive is recorded; a healthy scan can be frozen as a baseline and compared against later.
- **Native Linux, even for a cheap adapter.** The GODIAG GD101 gets a driver written for this project.

It has been proven on one car so far, for the engine, CVT, SRS and instrument cluster; the other systems are built from Honda's
data and are untested. [What is done and what is not](#what-is-done-and-what-is-not).

*We all as consumers have a right to repair. Nothing on a car should be proprietary.*

[Get started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) · [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) · [Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) · [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety) · [FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ)

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
| **Confirmed on a real car** | 2015 Honda Accord 2.4 (K24W1, CVT), GODIAG GD101 in J2534 mode: a full scan, trouble codes, live data and recorded drives |
| **Platform** | Linux (only the Linux build has been run) |
| **Binary release** | [v0.1.1](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.1.1), Linux x86_64, read-only: scan, codes, live data, history and reports. Two builds, see below |
| **Source** | In this repository as of v0.1.1: `hondascan/` (scanner, adapters, GODIAG GD101 J2534 driver) and `hondascan-gui/` (desktop app). The vehicle database (`hondascan/data/hondascan.db`, 21 MB) is included, so a clone builds and runs.|
| **Not in the first release** | Clearing codes, bi-directional (actuator) tests and writing functions exist in development but have **never been run on a car**, so they are not offered |

The honest summary: it reads the systems that have been checked, and says "untested" everywhere else. See [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles).

## What is done, and what is not

Everything below was measured on one car, a 2015 Accord 2.4 (K24W1, CVT), through a GODIAG GD101 in J2534 mode. Where something
has not run on a car, this says so.

**Works on a real car**
- A full scan: the PCM (engine and CVT answer from the same address), the SRS unit and the instrument cluster.
- Trouble codes with descriptions, live data with units, freeze frame and emissions readiness.
- Recording drives, stored per car, with trends, fuel economy, misfire and knock views.
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


## How it was built

Nobody handed over documentation. Everything below was worked out from Honda's own tool and hardware, then checked by running
it, not by trusting it.

- **Honda's tool, read from the inside.** HDS runs its diagnostic logic as bytecode. A decompiler written for this project turns
  it back into readable logic: 139 modules, 10,021 functions, no opcode left undecoded. Every in-scope function was then run
  next to Honda's own interpreter under emulation, on 127,313 inputs, and **all 9,660 agree, with 0 mismatches**. Where the
  decompiler disagreed with Honda's own code, it was the decompiler that was wrong, and it was fixed (one such bug was 3,586
  call sites with arguments in the wrong place).
- **The wire, from the adapter's firmware.** Honda's interface builds the CAN frames itself, so the addressing, the ISO-TP
  framing, the bit rate and the CAN address table were read out of its firmware with a disassembler, not guessed from captures.
- **A native driver for a cheap adapter.** The GODIAG GD101 ships a Windows driver. Its J2534 protocol was traced and rewritten as
  a small C library for Linux (`hondascan/drivers/gd101_j2534.c`), with a fake that behaves like the real device, down to its
  10-frame transmit queue.
- **The database.** 182 systems, 36,000 trouble codes, 35,397 help texts, 16,031 ECU identifiers, 25,882 CAN signal definitions and
  3,607 unit converters, rebuilt from Honda's tables. All 3,607 converters were run against Honda's own conversion code on 12 to
  20 inputs each: 67,553 runs, 0 mismatches.
- **Finding the car's control units without a lookup table.** HDS decides which ECU is which by running a small rule program
  against what the car answers. That program is run here too, and the Rust version agrees with it.
- **Strict fakes.** The simulated bus filters like SocketCAN, the ELM327 emulator waits for its timeout like the real chip, and
  the simulated ECUs refuse what the hardware refuses. Scanning the simulated car takes 0.17 s wall, 0.02 s of CPU.
- **One real car, measured.** Live on a 2015 Accord, the PCM's memory-read gate turned out to be a permanent calibration bit, which
  ended a whole line of work and is written down as a negative result.

## Field notes from the car

Every number here was measured on the Accord and logged; none is assumed. Longer write-ups are in the
[wiki](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/How-It-Was-Built).

- **A long drive, 44,149 reads.** 135 failed, and every one of them was the same 27 data groups the car refuses (asked five times
  each). A refusal is the car saying "not here", not a fault, and the tool shows it that way. Reply times: the CVT answers in 3 to
  11 ms, the engine computer in 1 to 49 ms, the instrument cluster in about 100 ms, and the SRS unit as slowly as 220 ms. That last
  one is why the timeout cannot be lowered.
- **Three misfires and no trouble code.** On a drive home the engine counted three misfires, cylinder 3 once and cylinder 1 twice,
  and set no code. All three fell inside bursts of knock retard (12.0, 6.5 and 11.5 degrees within 15 seconds). Knock retard climbs
  with revs: 0.01 degrees below 1,000 rpm, 2.2 at 2,500, 5.5 at 3,500, 9.4 at 5,500. The long-term fuel trim stayed at 0.96 to 0.99, so it
  showed no lean or rich correction. Cause not established; octane is a guess, labelled as one.
- **Coolant at -3 degrees.** The first version of the drive analysis decoded coolant temperature with the wrong formula and
  reported -1 to -3 degrees C on a warm engine. The real values, through the database's own converters, were 71 to 85 degrees. The
  same trap: an empty freeze-frame group also carries a temperature and an rpm, and showed -40 degrees and 0 rpm until it was
  skipped.
- **The car that went quiet.** At ignition off the engine computer stops answering; the recorder counted four failed reads. The
  SRS unit, still powered, answered a presence check and made the recorder forget which reads to excuse. Now it is tracked per
  address.
- **The ABS and power steering never answered.** From the diagnostic connector, with 29-bit and 11-bit addressing, and with a
  passive listen on all frames for four seconds each: silence. Honda's own tool has a second CAN controller on connector pins 3 and
  11 that the GD101 cannot use, but the vendor's connection code asks this adapter for the same pins as this tool does, so the
  difference is somewhere else. Still unsolved.
- **A wall found by building the tool to climb it.** `ramwatch` was built to read the engine computer's RAM. It found the
  gate instead: memory reads are refused by a permanent calibration bit. The tool stays, as the instrument that found the wall,
  and the plan to find the ignition map moved to modelling it from live data (best single-candidate correlation 0.79) before the
  project turned to diagnosis and reliability instead of tuning.
- **The engine computer's unlock is not the one Honda's tool uses.** It has its own check, which was read out of its ROM image and
  accepted by the car on the first live try.
- **Speed.** A full scan of the simulated car takes 0.17 seconds on the wall and 0.02 seconds of CPU. The scanner waits for the
  bus, not for the program.

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

## Build from source

You need Rust 1.92 or newer (`rustup`) and, for the GODIAG GD101's J2534 mode, a C compiler. The vehicle database is in the
repository, so there is nothing to generate first.

```
cd hondascan && cargo run --release --bin hondascan -- -a sim scan   # scanner, against the simulated car
cd hondascan && cargo build --release                                 # also builds hondascan-rpc, the app's backend
cd hondascan-gui && cargo run --release                               # the desktop app
```

The 3D car on the welcome screen is not in this repository, so that screen is empty here. Everything else works.

**Record your own car.** The recordings of the author's car are not in this repository. Make yours: run any command with
`--log capture.log` (`hondascan -a <adapter> --log capture.log scan`), then turn the capture into a simulator profile with
`cargo run --release --bin profile-from-capture -- capture.log profiles/mycar.json.gz --vin 00000000000000000`, which replaces your
VIN with the one you give it, so the profile can be shared.

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
study, modify and share it. The vehicle database (`hondascan/data/hondascan.db`) is derived from Honda's own diagnostic data
(trouble-code texts, signal definitions, conversion formulas) and is included for interoperability. The GPL covers this project's
code, not Honda's data: Honda's rights in the original remain. `hondascan/release-key.txt` is the key of the release's encrypted
copy of the same database, published so that copy can be opened too. The v0.1.1 binary release was published under the earlier freeware terms; they
remain what governs that download.

## Disclaimer

This is an independent project, **not affiliated with or endorsed by Honda Motor Co.** "Honda", "HDS" and model names are
trademarks of their owners and are used only to say what the tool works with. The software is provided as is, without
warranty. Diagnostics read data from your vehicle's computers; use it on your own vehicle and at your own risk.
