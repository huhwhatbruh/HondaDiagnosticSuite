# Security policy

## What counts

This tool talks to a car's diagnostic bus, so the security-relevant bugs are:

- a way to make the **read-only release send something it should refuse** (anything outside the allowlisted services, or a request
  to every control unit at once);
- a flaw in the **simulator or the checks** that would let such a request look safe;
- a problem in **how releases are built or distributed** (a checksum that does not match, an archive that carries private data);
- the usual ones: memory-safety, path handling, or unsafe parsing of a capture, a profile or a backup file.

## How to report

Please do not open a public issue for these. Use **GitHub's private vulnerability reporting**: on the repository page choose
*Security*, then *Report a vulnerability*. Include the version (`hondascan --version`), how you installed it, and the smallest
steps that show the problem. Remove your VIN from anything you attach.

This is a one-person project. Expect an acknowledgement within about a week; a fix lands in the next release, and sooner if the
problem could put a car at risk.

## Supported versions

Only the latest release gets fixes.
