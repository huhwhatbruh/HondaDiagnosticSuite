<div align="center">

<img width="720" alt="HondaDiagnosticSuite" src="https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/images/spin.gif" />

# Honda Diagnostic Suite

**An open, native Linux diagnostic tool for Honda vehicles.**
Read trouble codes and live data, record every drive, and keep a health history of your car, with no Windows, no Wine and no
dealer software.

*We all as consumers have a right to repair. Nothing on a car should be proprietary.*

[Get started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) · [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) · [Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) · [Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety) · [FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ)

</div>

---

> ### 🚗 I need people with other Honda vehicles
> Right now the tool has been proven on one car. Every new model, engine and year it runs on makes it better for everyone.
> **[Open a vehicle report](../../issues/new?template=vehicle_report.md)** or let me know, and I will be more than happy to add
> you to the private repository with the source code.

## What it does

| | |
|---|---|
| 🔎 **Scan** | Finds every control unit on the car (engine/PGM-FI, transmission, ABS, SRS, meter and more) in under a second. |
| ⚠️ **Trouble codes** | Reads stored, pending and history codes with plain-English descriptions, plus freeze frame and emissions readiness. |
| 📈 **Live data** | Named values with units, gauges, and an engine-map view built from what the car reports while you drive. |
| 🧭 **Drives and history** | Every session is recorded per car: codes that appeared and cleared, trends across drives, fuel economy, misfires and knock. |
| 🩺 **Health report** | One page that compares the car today with a saved reference snapshot: new codes, resolved codes, tests not yet run. |
| 💾 **Yours to keep** | Plain JSON export and a `.tar.gz` backup of everything the app has recorded. |
| 🔌 **Many adapters** | SocketCAN, slcan, J2534, ELM327 and clones, and the GODIAG GD101 through a native Linux driver. |

## Status

| | |
|---|---|
| **Confirmed on a real car** | 2015 Honda Accord 2.4 (K24W1, CVT), GODIAG GD101 in J2534 mode: a full scan, codes, live data and a 48-minute recorded drive |
| **Platform** | Linux (only the Linux build has been run) |
| **Binary release** | Coming. The first release will be read-only: scan, codes, live data, history and reports |
| **Source** | Private for now; access on request (see above) |
| **Not in the first release** | Clearing codes, bi-directional (actuator) tests and writing functions exist in development but have **never been run on a car**, so they are not offered |

The honest summary: it reads the systems that have been checked, and says "untested" everywhere else. See [Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles).

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
- **Nothing phones home.** Optional, off-by-default [sharing of diagnostics](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Privacy-and-Sharing) exists so other
  owners can help the project; you read the exact text before anything is sent, and every send asks first.

## Documentation

The [wiki](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki) holds the details so this page stays short:

[Getting started](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Getting-Started) ·
[Supported vehicles](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Supported-Vehicles) ·
[Adapters](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Adapters) ·
[Safety](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Safety) ·
[Privacy and sharing](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Privacy-and-Sharing) ·
[Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle) ·
[FAQ](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/FAQ) ·
[Roadmap](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Roadmap)

## Help wanted

- **Other Hondas.** Any model or year, any adapter. See [Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle); a scan and a
  trouble-code read is already valuable.
- **Other platforms.** Windows and macOS builds have not been tried.
- **Bug reports and ideas** go in the [issue tracker](../../issues).

## Disclaimer

This is an independent project, **not affiliated with or endorsed by Honda Motor Co.** "Honda", "HDS" and model names are
trademarks of their owners and are used only to say what the tool works with. The software is provided as is, without
warranty. Diagnostics read data from your vehicle's computers; use it on your own vehicle and at your own risk.
