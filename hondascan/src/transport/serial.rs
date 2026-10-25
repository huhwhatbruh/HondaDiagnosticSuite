//! A raw serial port: termios on Unix, the Win32 comm API on Windows. One reader per port (a second one takes replies meant
//! for us and puts the adapter out of step), `read` returns what arrived within a timeout, `write` all of it or a timeout.
use super::can::secs;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const BAUDS: [u32; 10] = [9600, 38400, 57600, 115200, 230400, 460800, 500000, 921600, 1000000, 2000000];

/// The port is held by another program.
pub fn busy() -> io::Error {
    #[cfg(unix)]
    return io::Error::from_raw_os_error(libc::EBUSY);
    #[cfg(windows)]
    return io::Error::from_raw_os_error(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32);
}

/// Windows opens a port that another program holds with ERROR_ACCESS_DENIED, or ERROR_SHARING_VIOLATION.
pub fn is_busy(e: &io::Error) -> bool {
    #[cfg(unix)]
    return e.raw_os_error() == Some(libc::EBUSY);
    #[cfg(windows)]
    return matches!(e.raw_os_error(), Some(5) | Some(32));
}

pub struct SerialPort {
    #[cfg(unix)]
    fd: libc::c_int,
    #[cfg(windows)]
    handle: isize,
    lost: AtomicBool,
}

impl SerialPort {
    pub fn supported_baud(baud: u32) -> bool {
        BAUDS.contains(&baud)
    }

    pub fn bauds() -> Vec<u32> {
        BAUDS.to_vec()
    }

    /// the port is gone: poll says HUP/ERR or a read hit EOF (Windows: a read or write failed).
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    fn lose(&self) {
        self.lost.store(true, Ordering::Relaxed);
    }
}

#[cfg(unix)]
mod sys {
    use super::*;

    fn baud_const(baud: u32) -> Option<libc::speed_t> {
        Some(match baud {
            9600 => libc::B9600,
            38400 => libc::B38400,
            57600 => libc::B57600,
            115200 => libc::B115200,
            230400 => libc::B230400,
            460800 => libc::B460800,
            500000 => libc::B500000,
            921600 => libc::B921600,
            1000000 => libc::B1000000,
            2000000 => libc::B2000000,
            _ => return None,
        })
    }

    impl SerialPort {
        pub fn open(path: &str, baud: u32) -> io::Result<SerialPort> {
            let speed = baud_const(baud).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported baud {baud}")))?;
            let c = std::ffi::CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            unsafe {
                // one reader only: a second one (another hondascan, ModemManager probing with AT commands) takes replies
                // meant for us and puts the adapter out of step. TIOCEXCL refuses later opens, flock another hondascan.
                let fd = libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK);
                if fd < 0 {
                    let e = io::Error::last_os_error();
                    return Err(if is_busy(&e) { busy() } else { e });
                }
                if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) < 0 || libc::ioctl(fd, libc::TIOCEXCL) < 0 {
                    libc::close(fd);
                    return Err(busy());
                }
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(fd, &mut t);
                t.c_iflag = 0;
                t.c_oflag = 0;
                t.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
                t.c_lflag = 0; // raw
                libc::cfsetispeed(&mut t, speed);
                libc::cfsetospeed(&mut t, speed);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(fd, libc::TCSANOW, &t);
                libc::tcflush(fd, libc::TCIOFLUSH);
                Ok(SerialPort { fd, lost: AtomicBool::new(false) })
            }
        }

        /// All of `data`, or ETIMEDOUT when the device takes nothing for `timeout`.
        pub fn write(&self, data: &[u8], timeout: f64) -> io::Result<()> {
            let end = Instant::now() + secs(timeout);
            let mut rest = data;
            while !rest.is_empty() {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
                }
                let mut p = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
                unsafe { libc::poll(&mut p, 1, left.as_millis().max(1) as i32) };
                let n = unsafe { libc::write(self.fd, rest.as_ptr() as *const _, rest.len()) };
                if n < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                        std::thread::sleep(std::time::Duration::from_millis(10)); // writable per poll, then EAGAIN: wait again
                        continue;
                    }
                    return Err(io::Error::last_os_error());
                }
                rest = &rest[n as usize..];
            }
            Ok(())
        }

        /// Bytes that arrived within `timeout`, or empty. USB CDC ports can report readable and then fail with EAGAIN;
        /// that is "nothing yet", not an error, and not lost: only HUP/ERR from the poll or EOF from the read set `lost`.
        pub fn read(&self, timeout: f64) -> Vec<u8> {
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            let ms = (timeout.max(0.0) * 1000.0).ceil().min(i32::MAX as f64) as i32;
            if unsafe { libc::poll(&mut p, 1, ms) } <= 0 {
                return vec![];
            }
            if p.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                self.lose(); // unplugged, or the driver gave the port up
                return vec![];
            }
            let mut buf = [0u8; 4096];
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n > 0 { buf[..n as usize].to_vec() } else if n == 0 {
                self.lose(); // EOF: the adapter is gone
                vec![]
            } else if matches!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
                vec![] // nothing yet, as before
            } else {
                self.lose();
                vec![]
            }
        }

        pub fn close(&self) {
            unsafe {
                libc::ioctl(self.fd, libc::TIOCNXCL); // the flag outlives us while another fd is open (a pty)
                libc::close(self.fd);
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::*;
    use windows_sys::Win32::Devices::Communication::{GetCommState, PurgeComm, SetCommState, SetCommTimeouts, COMMTIMEOUTS, DCB, PURGE_RXCLEAR, PURGE_TXCLEAR};
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING};

    /// How long one ReadFile / WriteFile waits. `read` and `write` repeat until their own deadline, so the timeouts set on
    /// the handle once at open are the same for both and never change while another thread is inside a call.
    const SLICE_MS: u32 = 5;
    const DCB_BINARY: u32 = 1; // fBinary
    const DCB_DTR_ENABLE: u32 = 1 << 4; // fDtrControl = DTR_CONTROL_ENABLE: a Unix open raises DTR and RTS too
    const DCB_RTS_ENABLE: u32 = 1 << 12; // fRtsControl = RTS_CONTROL_ENABLE

    /// COM10 and up only open as `\\.\COM10`; the prefix does no harm on the lower numbers.
    fn device_path(path: &str) -> String {
        if path.starts_with(r"\\") { path.to_string() } else { format!(r"\\.\{path}") }
    }

    impl SerialPort {
        fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
            self.handle as windows_sys::Win32::Foundation::HANDLE
        }

        pub fn open(path: &str, baud: u32) -> io::Result<SerialPort> {
            if !SerialPort::supported_baud(baud) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported baud {baud}")));
            }
            let wide: Vec<u16> = device_path(path).encode_utf16().chain(Some(0)).collect();
            // share mode 0: nobody else may open the port while we hold it (the Windows counterpart of TIOCEXCL + flock)
            let h = unsafe { CreateFileW(wide.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
            if h == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let port = SerialPort { handle: h as isize, lost: AtomicBool::new(false) };
            let configured = unsafe {
                let mut dcb: DCB = std::mem::zeroed();
                dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
                let mut ok = GetCommState(h, &mut dcb) != 0;
                dcb.BaudRate = baud;
                dcb.ByteSize = 8;
                dcb.Parity = 0; // NOPARITY
                dcb.StopBits = 0; // ONESTOPBIT
                dcb._bitfield = DCB_BINARY | DCB_DTR_ENABLE | DCB_RTS_ENABLE; // no flow control, no software handshake, no parity check
                ok &= SetCommState(h, &dcb) != 0;
                // ReadIntervalTimeout and Multiplier MAXDWORD with a constant: return at once with what is there, else wait
                // up to the constant for the first byte
                let t = COMMTIMEOUTS { ReadIntervalTimeout: u32::MAX, ReadTotalTimeoutMultiplier: u32::MAX, ReadTotalTimeoutConstant: SLICE_MS, WriteTotalTimeoutMultiplier: 0, WriteTotalTimeoutConstant: SLICE_MS };
                ok &= SetCommTimeouts(h, &t) != 0;
                ok
            };
            if !configured {
                let e = io::Error::last_os_error();
                port.close();
                return Err(e);
            }
            unsafe { PurgeComm(h, PURGE_RXCLEAR | PURGE_TXCLEAR) };
            Ok(port)
        }

        /// All of `data`, or a TimedOut error when the device takes nothing for `timeout`.
        pub fn write(&self, data: &[u8], timeout: f64) -> io::Result<()> {
            let end = Instant::now() + secs(timeout);
            let mut rest = data;
            while !rest.is_empty() {
                if end.saturating_duration_since(Instant::now()).is_zero() {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                let mut n = 0u32;
                let ok = unsafe { WriteFile(self.raw(), rest.as_ptr(), rest.len().min(u32::MAX as usize) as u32, &mut n, std::ptr::null_mut()) };
                if ok == 0 {
                    self.lose();
                    return Err(io::Error::last_os_error());
                }
                rest = &rest[n as usize..]; // fewer than asked when the write timed out: ask again until the deadline
            }
            Ok(())
        }

        /// Bytes that arrived within `timeout`, or empty. A failed read means the adapter is gone (unplugged USB serial).
        pub fn read(&self, timeout: f64) -> Vec<u8> {
            let end = Instant::now() + secs(timeout);
            let mut buf = [0u8; 4096];
            loop {
                let mut n = 0u32;
                let ok = unsafe { ReadFile(self.raw(), buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
                if ok == 0 {
                    self.lose();
                    return vec![];
                }
                if n > 0 {
                    return buf[..n as usize].to_vec();
                }
                if Instant::now() >= end {
                    return vec![];
                }
            }
        }

        pub fn close(&self) {
            unsafe { CloseHandle(self.raw()) };
        }
    }
}
