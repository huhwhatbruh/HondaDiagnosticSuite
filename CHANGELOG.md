# Changelog

All notable changes to the release. Format after [Keep a Changelog](https://keepachangelog.com/); versions follow
[Semantic Versioning](https://semver.org/) from 1.0.0 on. Each release's page has the archives and their checksums.

## [0.7.0]

### Added
- **Brake pad maintenance mode for cars with an electric parking brake in the VSA/ABS unit** (suggested by a 2019 Odyssey owner
  replacing rear pads). Two new writing functions, `epb_pad_enter` and `epb_pad_leave`, do what Honda's diagnostic software does:
  they put the parking brake into pad-change mode and back to normal. Like every writing function they need your confirmation
  and the car's VIN, the battery must be above 10 V, and the car must answer the check Honda's software makes before it offers
  the function. Tested against a simulated car only; it has not been run on a real car yet. Cars with a separate parking brake
  unit are not covered.

## [0.6.2]

### Changed
- **The sidebar no longer has a "Share car details" tick box.** The question is asked once, the first time the app opens.
  Change the answer any time with `hondascan share on` / `off`.

## [0.6.1]

### Added
- **Opt-in problem report.** The first time the app opens, it asks whether to send a short report when something goes wrong,
  or when a car works for the first time (at most five a day): the errors the app ran into, the car's make, model and year from
  the VIN, the control units that answered, trouble codes and how the scan went. Never the VIN, port names, folders, driving
  data or anything personal. Nothing is sent unless you press Share; change it any time with
  `hondascan share on` / `off`, and see the exact report with `hondascan share preview`. Details on the
  [Privacy](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Privacy) page.

### Changed
- **Live data with a system screen open is read once, not twice.** The Engine and Transmission cards on the Overview kept
  polling the same values as the open system screen of that control unit, so the adapter asked for them twice. The cards
  now leave those to the screen while it is open. On a slow adapter (ELM327) the screen updates faster.

### Fixed
- **A long session in which a control unit stopped answering kept growing in memory.** The recorder kept one entry per
  unanswered read; it now keeps a count per value.

### Removed
- The program-reflash groundwork added in 0.6.0. It was never wired to a command, and its frames were later found not to
  match what Honda's own tool sends. hondascan does not reprogram control units: it no longer enters the programming session
  or sends any program-download service.

## [0.6.0]

### Added
- The program-reflash wire sequence (unlock, erase, per-block transfer, finish) and a strict simulated control unit to test it
  against. Not wired to any command yet, and nothing has been sent to a real control unit.

### Fixed
- **Phone gateway: a request the phone was told had failed could still run later.** An actuator test started while a scan held
  the backend got a timeout answer and then ran up to a minute afterwards. Requests now carry an expiry and are dropped when it
  passes. A start that timed out is also covered by the stop-when-the-phone-goes-quiet rule.
- **Phone gateway: clearing trouble codes is held for a real car by the backend**, on the adapter connected at that moment, not on
  a cached snapshot; a clear goes only to a system the scan found.
- **Phone gateway: guessing the pairing code from many addresses at once** is held to 20 wrong codes a minute in total, and a phone
  that has already paired keeps its link meanwhile. The code is still sent unencrypted over Wi-Fi and Bluetooth: use the gateway
  only on a network you control, and set `HONDASCAN_PHONE_BLE=off` if you do not need Bluetooth.
- **GODIAG GD101 J2534 driver:** an empty frame from the adapter is skipped instead of being read as a stale command byte, and a
  failed baud-rate switch is reported instead of leaving the adapter in its ELM mode.
- **Write audit trail:** the outcome (cancelled, unconfirmed, error) no longer depends on the wording of an error message.

## [0.5.1]

### Fixed
- **ELM327 adapters: `Could not read: 1902ff: BUFFERFULL`.** Control units with long trouble-code lists (engine, transmission and
  ABS on a 2021 Civic) overran the adapter's buffer. The request is now asked again with the control unit told to send its frames
  further apart (up to 40 ms). Not yet tried on a real adapter.

### Added
- **Engine condition** (command line `condition`, desktop app Health screen): the newest recorded drive is compared with the car's
  own earlier drives, per operating regime, and unusual readings are listed with the trouble-code reasoning behind them.

## [0.5.0]

### Changed
- **The release is no longer read-only.** Clearing trouble codes, four actuator tests (radiator fan, A/C compressor clutch, 12 V
  battery fan, EVAP purge solenoid) and eight writing functions (the VTC learn, the first stage of the CVT inclination-sensor learn, the instrument
  cluster's maintenance resets) are in the release, each allowlisted per system, behind a confirmation at every step, with a
  time limit and control handed back to the control unit at the end. Only clearing codes has been run on a car; the actuator
  tests and the writing functions have not, so do not use them on yours. The desktop app shows the Tests and Writes tabs.
- Both Windows builds have run on a real Windows machine, one of them against a car.

## [0.4.0]

### Added
- A new desktop app (Qt): dials that sweep at key-on, a Honda-red accent, a black matte look, dialogs over a blurred app. The 64-bit
  Windows zip carries it with its Qt libraries. The 32-bit Windows zip is the command line only (Qt 6 has no 32-bit Windows build).
  Not yet run on a real Windows machine or a car.
- A build recipe for a macOS (Apple silicon) version of the app. No macOS archive was published, and it has **never been started on a Mac**.
- `hondascan preflight`: before a drive, checks that the adapter is real, the VIN is the one expected and the engine, ABS/VSA,
  power steering, SRS and meter computers answer. Exit code 1 if one does not.
- The scan sends the all-ECU wakeup that Honda's own tool sends (`3E 00` to `0x18DBEFF1`) before it asks each address, over raw
  CAN adapters. Whether it wakes ABS and power steering on a real car has not been tried.
- A session summary says "NOT ANSWERING" when a core system was silent, and the app shows a "SIMULATED CAR" badge.

### Changed
- ELM327 adapters: a damaged multi-frame reply (a frame missing, a line with extra or missing characters) is now refused and asked
  again with more time between the ECU's frames (5, 10, then 20 ms), where before a line with extra bytes was taken as data and
  a reply missing its last frame was dropped as "no answer" (public issue 9). Whether the longer spacing cures a given clone is
  unconfirmed; if the serial or Bluetooth link itself corrupts text, a wired adapter is the fix.
- A simulated adapter is never saved as the next adapter to open.
- Builds and tests are faster (nextest, a lighter SQLite in debug builds).

## [0.3.1]

### Added
- A 32-bit Windows archive (`windows-i686`), for 32-bit J2534 DLLs.
- A first probe of Honda's older K-line protocol (DIAG-H) in the source: `cargo run --bin diagh-session` wakes a system over a serial
  K-line cable, keeps it awake and stops it. Not in the release archives; only run against simulators.

## [0.3.0] - 2026-10-27

### Changed
- Trouble codes are read when you ask. A scan no longer reads them: the Overview's Trouble codes card has a **Read codes** button
  that opens a dialog where you tick the systems. A system that was not read shows as "Not read", never as clean.

## [0.2.1]

### Fixed
- Windows: light-grey edges and a white search box when Windows was in light mode.

## [0.2.0]

### Added
- A native Windows build (64-bit zip of `.exe` files).

## [0.1.3] - 2026-10-24

### Added
- `elm-obd`: generic OBD-II for cars before about 2008, read-only. Only run against a simulator.
