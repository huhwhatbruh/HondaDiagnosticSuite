# Changelog

All notable changes to the read-only release. Format after [Keep a Changelog](https://keepachangelog.com/); versions follow
[Semantic Versioning](https://semver.org/) from 1.0.0 on. Each release's page has the archives and their checksums.

## [0.3.1]

### Added
- A 32-bit Windows archive (`windows-i686`), for 32-bit J2534 DLLs.
- Honda's older K-line protocol (DIAG-H) over a serial K-line cable (`diagh-session`). Only run against simulators.

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
