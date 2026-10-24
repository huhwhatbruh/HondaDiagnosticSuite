#!/usr/bin/env python3
"""Research only: a stand-in for the GODIAG GD101 on a pty, to trace GODIAG's J2534 DLL under Wine without the adapter.

The adapter's framing is in README.md, "GD101" ([n words] payload [crc8 poly 7], or [7F len16 LE] payload [crc8]; bit 7 of the
first byte = XOR-encrypted). This answers the open handshake like tests/common/gd101_emulator.rs and logs every frame it receives
(raw hex and, when the CRC holds, the payload) so the commands the DLL builds for a protocol we have never traced (ISO 9141,
ISO 14230) can be read. Unknown commands get `<cmd|40> 0B 00 00`, as the real adapter does for a malformed one. It never claims
a K-line answer; what the DLL does with that is the point of the trace.

  gd101_fake.py PTYLINK LOG      creates a pty, symlinks it at PTYLINK (e.g. the Wine dosdevices/com1), logs to LOG
"""
import os, pty, select, sys, termios, time, tty

SERIAL = b"MT0000001"


def crc8(data):
    c = 0
    for b in data:
        c ^= b
        for _ in range(8):
            c = ((c << 1) ^ 7) & 0xFF if c & 0x80 else (c << 1) & 0xFF
    return c


def frame(payload, enc=False):
    if len(payload) % 2 == 0 and len(payload) < 0xFE:
        head = bytes([len(payload) // 2 | (0x80 if enc else 0)])
    else:
        head = bytes([0xFF if enc else 0x7F, len(payload) & 255, len(payload) >> 8])
    h = head + bytes(payload)
    return h + bytes([crc8(h)])


def reply_for(p):
    if p[0] == 0:
        if p[1] in (1, 2):
            return bytes([0x40, p[1], 0, 0])
        if p[1] == 3:
            return bytes([0x40, 3, 0, 0x0E, 0x01]) + SERIAL + bytes([2, 0x17, 0x55, 0x1D]) + b"XXXX0000" + bytes([0x19, 0x51, 0x1E])
        if p[1] == 0 and os.environ.get("IDENT") and set(p[2:]) == {0x55}:
            m = os.environ["IDENT"]
            return {"none": None, "echo": bytes([0x40, 0]) + p[2:], "zero64": bytes([0x40, 0]) + bytes(64),
                    "long": bytes([0x40, 0, 0, 0]) + SERIAL + bytes(128)}[m]
        if p[1] == 0:
            return (bytes([0x40, 0, 0, 0]) + SERIAL).ljust(66, b'\0') if set(p[2:]) == {0x55} else bytes([0x40, 0, 0, 0]) + SERIAL + bytes(128)
    if p[:4] == bytes([2, 0, 0x10, 0]):
        return bytes([0x42, 0, 0, 0x10]) + (12500).to_bytes(4, "little") + bytes([0, 1])
    if p[0] == 1 and len(p) >= 4:
        return bytes([0x41, p[1], 0, 0])
    return bytes([p[0] | 0x40, 0x0B, 0, 0])


def main():
    link, logf = sys.argv[1:3]
    master, slave = pty.openpty()
    tty.setraw(slave)
    name = os.ttyname(slave)
    if os.path.lexists(link):
        os.unlink(link)
    os.symlink(name, link)
    log = open(logf, "a", buffering=1)
    t0 = time.time()

    def out(s):
        log.write(f"{time.time() - t0:8.3f} {s}\n")

    out(f"fake GD101 on {name} -> {link}")
    buf = b""
    while True:
        if not select.select([master], [], [], 0.2)[0]:
            continue
        try:
            buf += os.read(master, 4096)
        except OSError:
            continue
        while buf:
            long = buf[0] & 0x7F == 0x7F
            h = 3 if long else 1
            if len(buf) < h:
                break
            n = (buf[1] | buf[2] << 8) if long else (buf[0] & 0x7F) * 2
            if len(buf) < h + n + 1:
                break
            f, buf = buf[:h + n + 1], buf[h + n + 1:]
            ok = crc8(f[:-1]) == f[-1]
            enc = bool(f[0] & 0x80)
            out(f"RX {'enc ' if enc else ''}{'crc-ok' if ok else 'crc-BAD'} raw {f.hex(' ')}")
            if ok and not enc:
                p = f[h:-1]
                out(f"   cmd {p.hex(' ')}")
                rp = reply_for(p)
                if rp is not None:
                    r = frame(rp, enc=bool(os.environ.get("ENC")) and rp[0] != 0x40 or os.environ.get("ENC") == "all")
                    out(f"   reply {r.hex(' ')}")
                    os.write(master, r)
            elif ok:
                out("   (encrypted: decode with the key the probe prints)")
                os.write(master, frame(bytes([(f[h] ^ 0) | 0x40 & 0x7F, 0x0B, 0, 0]), False))


if __name__ == "__main__":
    main()
