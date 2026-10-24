---
name: Vehicle report
about: Tell us what happened when you ran the tool on your Honda
title: 'Vehicle report: <year> <model> <engine>'
labels: vehicle-report
assignees: ''

---

Thank you: a scan and a code read from a car we have not seen is the most useful thing you can send.
[Contributing a vehicle](https://github.com/huhwhatbruh/HondaDiagnosticSuite/wiki/Contributing-a-Vehicle) says how.

**Vehicle**
- Year / model / trim:
- Engine and transmission (if known):
- Market (US, EU, JDM, ...):

**Adapter**
- Name and model (e.g. GODIAG GD101 J2534, CANable, ELM327 clone):
- The `-a` spec you used:

**Software**
- Version (`hondascan --version`, or the release you downloaded):
- [ ] Release archive
- [ ] Built from source (commit):
- Linux distribution:

**What worked**
- [ ] Scan found control units (which?)
- [ ] Trouble codes read
- [ ] Live data
- [ ] Recorded a drive

**What did not work**
What you ran, what you expected, and what the tool printed. If a system never answered (ABS and power steering did
not on the one tested car), say so: that is a result too.

**Attachments**
The output of `hondascan scan` and `hondascan dtc` (remove the VIN first), and optionally a raw capture from
`--log capture.log` (it can contain the VIN: remove it or ask before attaching).
