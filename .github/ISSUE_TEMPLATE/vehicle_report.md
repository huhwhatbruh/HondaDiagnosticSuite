---
name: Vehicle report
about: Tell us what happened when you ran the tool on your Honda
title: 'Vehicle report: <year> <model> <engine>'
labels: vehicle-report
assignees: ''

---

**Vehicle**
- Year / model / trim:
- Engine and transmission (if known):
- Market (US, EU, JDM, ...):

**Adapter**
- Name and model (e.g. GODIAG GD101 J2534, CANable, ELM327 clone):
- The `-a` spec you used:

**What worked**
- [ ] Scan found control units (which?)
- [ ] Trouble codes read
- [ ] Live data
- [ ] Recorded a drive

**What did not work**
What you ran, what you expected, and what the tool printed.

**Attachments**
`hondascan telemetry export --out bundle.json` (read it first: no VIN, name or home folder), and optionally a raw capture
from `--log capture.log`.
