# About this source

This is the source of Honda Diagnostic Suite as of release [v0.1.1](https://github.com/huhwhatbruh/HondaDiagnosticSuite/releases/tag/v0.1.1).

- `hondascan/`: the scanner and its command line, the protocol layers (UDS, ISO-TP, ISO 14230), the adapter backends (SocketCAN, slcan, J2534, ELM327) and the GODIAG GD101 J2534 driver in C (`drivers/gd101_j2534.c`).
- `hondascan-gui/`: the desktop app, which runs the scanner as its backend.
- `rust-toolchain.toml`: the pinned Rust version.

## What is not here, and what that means for building

- The vehicle database (`hondascan.db`) is generated from Honda's own data files, which cannot be redistributed. Without it the scanner cannot be built or run in full from this tree.
- A few modules are left out: the seed/key routine, the simulator's capture replay, the release key and the packaging files. Code that refers to them will not compile as it stands.
- The test suite and the recorded car profiles are left out; they depend on the database or on captures from the owner's car.
- The 3D car model used by the desktop app is left out.

The iPhone app is not part of this release.

Read it as a reference for how the protocols and the adapters work. The code is licensed under GPL-3.0-or-later (see `LICENSE`); the generated database is not part of it.

Honda and HDS are trademarks of their owners. This project is independent and not affiliated with or endorsed by them.
