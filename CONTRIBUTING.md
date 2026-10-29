# Contributing

Thanks for helping. The most useful contribution is still **a report from a car that has not been tried**: run a scan and a
code read, then open a [vehicle report](../../issues/new?template=vehicle_report.md). The wiki page
[Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle) says how.

## Before you change code

- **The release is read-only, and that is a promise.** The client refuses everything except a short allowlist of diagnostic
  services. A change that widens what can be sent to a car needs an issue first, and will not be merged without a plan for
  checking it on a simulator that is at least as strict as the real ECU.
- **Nothing private in a pull request.** No VINs, no captures from your own car, no personal paths. A capture can be turned into a
  shareable simulator profile with `profile-from-capture ... --vin 00000000000000000`.
- **Say how you checked it.** The project's habit is to test against Honda's own code by execution rather than against
  expectations written by hand. If your change touches a converter, a rule or a decoder, say which check you ran.

## Build and check

You need Rust 1.92 or newer and, for the GODIAG GD101's J2534 mode, a C compiler. The vehicle database is in the repository.

```
cd hondascan
cargo run --release --bin hondascan -- -a sim scan   # the simulated car, no hardware needed
cargo test
cargo clippy --all-targets -- -D warnings
```

Run the same three commands in `hondascan-gui` for changes to the desktop app. The app's front end is under active change, so
open an issue before a large GUI pull request.

## Pull requests

Keep one change per pull request, with a message that says what changed and why. There is no automatic CI, so the checks above
run on your machine; please paste the result of `cargo test` in the description.

## Conduct and security

Be kind and be specific. See the [code of conduct](CODE_OF_CONDUCT.md). To report a vulnerability, follow [SECURITY.md](SECURITY.md)
instead of opening a public issue.
