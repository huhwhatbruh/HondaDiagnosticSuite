# Changelog

All notable changes to the read-only release. Format after [Keep a Changelog](https://keepachangelog.com/); versions follow
[Semantic Versioning](https://semver.org/) from 1.0.0 on. Each release's page has the archives and their checksums.

## [0.4.0]

### Added
- A new desktop app (Qt): dials that sweep at key-on, a Honda-red accent, a black matte look, dialogs over a blurred app. The 64-bit
  Windows zip carries it with its Qt libraries. The 32-bit Windows zip is the command line only (Qt 6 has no 32-bit Windows build).
  Not yet run on a real Windows machine or a car.
- A macOS (Apple silicon) build of the app, built for release but **never started on a Mac**.
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
