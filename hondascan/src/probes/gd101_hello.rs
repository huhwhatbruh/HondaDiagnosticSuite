//! Native GD101 J2534 handshake probe: 2534 baud (the mode switch, as in GODIAG_PT32 sub_23f90), then the PassThruOpen frames
//! 00 01 and 00 02 (sub_27540 / sub_27590), framed as sub_25740: [len/2] payload [crc8 poly 7].
//!
//!   gd101-hello [DEVICE]        (default /dev/gd101)
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

pub fn crc8(data: &[u8]) -> u8 {
    let mut c = 0u8;
    for &b in data {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
    }
    c
}

/// [len/2] payload (padded to even) [crc8 of both].
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut p = payload.to_vec();
    if p.len() % 2 == 1 {
        p.push(0);
    }
    let mut head = vec![(p.len() / 2) as u8];
    head.extend(p);
    head.push(crc8(&head));
    head
}

/// Python's `repr()` of a bytes object.
pub fn bytes_repr(b: &[u8]) -> String {
    let q = if b.contains(&b'\'') && !b.contains(&b'"') { '"' } else { '\'' };
    let mut s = format!("b{q}");
    for &x in b {
        match x {
            b'\\' => s.push_str("\\\\"),
            b'\t' => s.push_str("\\t"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            x if x as char == q => {
                s.push('\\');
                s.push(q);
            }
            0x20..=0x7e => s.push(x as char),
            x => s.push_str(&format!("\\x{x:02x}")),
        }
    }
    s.push(q);
    s
}

pub fn hex_spaced(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// The termios2/custom-baud ioctl interface is Linux-specific (the module is only built there, see probes/mod.rs).
mod linux {
    use super::*;
    /// Linux `struct termios2` (asm-generic): the kernel's custom-baud interface.
    #[repr(C)]
    #[derive(Default)]
    struct Termios2 {
        iflag: u32,
        oflag: u32,
        cflag: u32,
        lflag: u32,
        line: u8,
        cc: [u8; 19],
        ispeed: u32,
        ospeed: u32,
    }

    const TCGETS2: libc::c_ulong = 0x802C542A;
    const TCSETS2: libc::c_ulong = 0x402C542B;
    const BOTHER: u32 = 0o010000;
    const CBAUD: u32 = 0o010017;
    const TIOCMBIS: libc::c_ulong = 0x5416;
    const TIOCM_DTR: u32 = 0x002;

    fn ioctl_err(r: libc::c_int, what: &str) -> Result<(), String> {
        if r < 0 {
            Err(format!("{what}: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    /// Raw 8N1 at an arbitrary baud (BOTHER), DTR up.
    pub fn set_baud(fd: i32, baud: u32) -> Result<(), String> {
        let mut t = Termios2::default();
        ioctl_err(unsafe { libc::ioctl(fd, TCGETS2, &mut t as *mut Termios2) }, "TCGETS2")?;
        let drop = CBAUD | libc::CSIZE | libc::PARENB | libc::CSTOPB | libc::CRTSCTS;
        t.cflag = (t.cflag & !drop) | BOTHER | libc::CS8 | libc::CREAD | libc::CLOCAL;
        (t.iflag, t.oflag, t.lflag, t.cc) = (0, 0, 0, [0; 19]);
        (t.ispeed, t.ospeed) = (baud, baud);
        ioctl_err(unsafe { libc::ioctl(fd, TCSETS2, &t as *const Termios2) }, "TCSETS2")?;
        let dtr = TIOCM_DTR;
        ioctl_err(unsafe { libc::ioctl(fd, TIOCMBIS, &dtr as *const u32) }, "TIOCMBIS")
    }

    pub fn read_for(f: &mut std::fs::File, secs: f64) -> Result<Vec<u8>, String> {
        use std::io::Read as _;
        let (mut out, end) = (vec![], Instant::now() + Duration::from_secs_f64(secs));
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = left.as_millis() as i32 + i32::from(left.subsec_nanos() % 1_000_000 != 0);
            let n = unsafe { libc::poll(&mut p, 1, ms) };
            if n > 0 {
                let mut buf = [0u8; 4096];
                let k = f.read(&mut buf).map_err(|e| format!("read: {e}"))?;
                out.extend_from_slice(&buf[..k]);
            }
            if Instant::now() >= end {
                return Ok(out);
            }
        }
    }

    pub fn run_linux(args: &[String], out: &mut dyn std::io::Write) -> Result<(), String> {
        let dev = args.first().map_or("/dev/gd101", String::as_str);
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(dev)
            .map_err(|e| format!("{dev}: {e}"))?;
        set_baud(f.as_raw_fd(), 2534)?;
        unsafe { libc::tcflush(f.as_raw_fd(), libc::TCIOFLUSH) };
        for cmd in [&[0x00u8, 0x01][..], &[0x00, 0x02], &[0x02, 0x00, 0x10], &[0x00, 0x03]] {
            // 02 00 10: sub_26609, read-only
            let fr = frame(cmd);
            f.write_all(&fr).map_err(|e| format!("write: {e}"))?;
            let got = read_for(&mut f, 1.0)?;
            let _ = writeln!(out, "TX {}  RX ({}) {}  {}", hex_spaced(&fr), got.len(), hex_spaced(&got), bytes_repr(&got));
        }
        Ok(())
    }
}

/// Public run entry-point.
pub fn run(args: &[String], out: &mut dyn std::io::Write) -> Result<(), String> {
    linux::run_linux(args, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_poly_7() {
        assert_eq!(crc8(b"123456789"), 0xF4); // CRC-8 (poly 0x07, init 0) check value
        assert_eq!(crc8(&[]), 0);
    }

    #[test]
    fn frames_pad_to_even_and_count_words() {
        let f = frame(&[0x00, 0x01]);
        assert_eq!(&f[..3], &[1, 0, 1]);
        assert_eq!(f[3], crc8(&f[..3]));
        let f = frame(&[0x02, 0x00, 0x10]);
        assert_eq!(&f[..5], &[2, 2, 0, 0x10, 0]);
        assert_eq!(f.len(), 6);
    }

    #[test]
    fn python_bytes_repr() {
        assert_eq!(bytes_repr(b"a\x00\n'"), "b\"a\\x00\\n'\"");
        assert_eq!(bytes_repr(b"it's \"x\""), "b'it\\'s \"x\"'");
        assert_eq!(hex_spaced(&[1, 0xab]), "01 ab");
    }
}
