---
name: Bug report
about: Something crashed, printed nonsense, or behaved differently from the wiki
title: ''
labels: bug
assignees: ''

---

**What happened**
What you ran, what you expected, and what happened instead. Paste the exact command and the full output
(remove the VIN first: it is 17 characters starting with 1, 2, 3, J, S or W).

**Does it happen on the simulated car?**
Run `hondascan -a sim scan`. If the bug shows there too, you need no car or adapter to reproduce it.
- [ ] Yes, on the simulator
- [ ] No, only with my car or adapter

**Version and how you got it**
- Version (`hondascan --version`, or the release you downloaded):
- [ ] Release archive
- [ ] Built from source (commit):

**Your computer**
- Linux distribution and version:
- Desktop app or command line:
- Wayland or X11 (desktop app only):

**Your car and adapter** (if the car is involved)
- Year / model / engine and transmission:
- Adapter and the `-a` spec you used (e.g. GODIAG GD101 J2534, `socketcan:can0`, ELM327 clone):

**Attachments**
A capture helps most: re-run with `--log capture.log`. It can contain the VIN, so remove it or ask before attaching.
Screenshots help for the desktop app.
