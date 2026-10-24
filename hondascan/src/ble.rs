//! Bluetooth for the phone gateway: the same HTTP the Wi-Fi listener speaks, over a Bluetooth LE stream channel (an L2CAP
//! connection-oriented channel), so the phone can read the laptop where there is no Wi-Fi at all (a car). The phone finds
//! the laptop by its advertised service UUID and opens channel PSM 0xF5 (in the LE dynamic range 0x80-0xFF, Bluetooth Core
//! Spec Vol 3 Part A section 4.22); everything after that is the gateway's own protocol, with the same pairing code, lockout,
//! keep-alive and logs (the peer is `ble`).
//!
//! No new dependency: the channel is a kernel socket (libc), and the advertisement is made by the BlueZ `bluetoothctl` tool,
//! which has to stay running for the advertisement to last, so a child process holds it and is restarted if it ends.
//! Nothing here needs root. The adapter must be powered and not rfkill-blocked (`rfkill unblock bluetooth`).
//!
//! The channel is not encrypted: it carries the same read-only car data as the Wi-Fi link, which is plain HTTP too.
use crate::phone::{Conn, Logs};
use serde_json::json;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

/// The service the phone scans for.
pub const SERVICE_UUID: &str = "5a7e1f2c-8b3d-4e6a-9c10-4f0d8e2b7a11";
pub const PSM: u16 = 0x00F5;

const BTPROTO_L2CAP: libc::c_int = 0; // linux/bluetooth.h
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_SNDMTU: libc::c_int = 12;
const BT_RCVMTU: libc::c_int = 13;
const BDADDR_LE_PUBLIC: u8 = 1;
const RECEIVE_MTU: u16 = 4096; // the largest message the phone may send in one piece: a request is a few hundred bytes

#[repr(C)]
struct SockaddrL2 {
    family: u16,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

/// An accepted channel. A write is cut into pieces of at most the channel's outgoing MTU (the kernel refuses a larger one).
pub struct BleStream {
    fd: OwnedFd,
    omtu: usize,
}

impl Read for BleStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }
}

impl Write for BleStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.omtu);
        let sent = unsafe { libc::send(self.fd.as_raw_fd(), buf.as_ptr() as *const libc::c_void, n, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(sent as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Conn for BleStream {
    fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        let tv = libc::timeval { tv_sec: d.map_or(0, |d| d.as_secs() as libc::time_t), tv_usec: d.map_or(0, |d| d.subsec_micros() as libc::suseconds_t) };
        let r = unsafe { libc::setsockopt(self.fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32) };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn peer(&self) -> String {
        "ble".into()
    }
}

fn listen() -> std::io::Result<OwnedFd> {
    unsafe {
        let raw = libc::socket(libc::AF_BLUETOOTH, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, BTPROTO_L2CAP);
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(raw);
        let addr = SockaddrL2 { family: libc::AF_BLUETOOTH as u16, psm: PSM.to_le(), bdaddr: [0; 6], cid: 0, bdaddr_type: BDADDR_LE_PUBLIC };
        if libc::bind(raw, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<SockaddrL2>() as u32) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mtu = RECEIVE_MTU;
        libc::setsockopt(raw, SOL_BLUETOOTH, BT_RCVMTU, &mtu as *const _ as *const libc::c_void, 2);
        if libc::listen(raw, 4) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(fd)
    }
}

/// The accepted channel's outgoing MTU: the most one write may carry.
fn outgoing_mtu(fd: &OwnedFd) -> usize {
    let mut mtu: u16 = 0;
    let mut len = 2u32;
    let r = unsafe { libc::getsockopt(fd.as_raw_fd(), SOL_BLUETOOTH, BT_SNDMTU, &mut mtu as *mut _ as *mut libc::c_void, &mut len) };
    if r == 0 && mtu >= 23 {
        mtu as usize
    } else {
        23
    }
}

/// Listen for the phone and advertise the laptop. Without a usable adapter this says so in the log and leaves Wi-Fi alone.
pub(crate) fn start(serve: Arc<dyn Fn(Box<dyn Conn>) + Send + Sync>, logs: Arc<Logs>) {
    let listener = match listen() {
        Ok(l) => l,
        Err(e) => {
            logs.gateway.write(json!({"ev": "ble", "state": "unavailable", "error": e.to_string()}));
            eprintln!("hondascan-rpc: bluetooth is not available: {e}");
            return;
        }
    };
    logs.gateway.write(json!({"ev": "ble", "state": "listening", "psm": PSM, "uuid": SERVICE_UUID}));
    let lg = logs.clone();
    std::thread::spawn(move || loop {
        let raw = unsafe { libc::accept4(listener.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        if raw < 0 {
            lg.gateway.write(json!({"ev": "ble", "state": "accept failed", "error": std::io::Error::last_os_error().to_string()}));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let omtu = outgoing_mtu(&fd);
        lg.gateway.write(json!({"ev": "ble", "state": "connected", "omtu": omtu}));
        serve(Box::new(BleStream { fd, omtu }));
    });
    std::thread::spawn(move || advertise(&logs));
}

/// Hold the advertisement: `bluetoothctl` drops it when it exits, so it is kept running and started again if it dies.
fn advertise(logs: &Logs) {
    loop {
        let child = Command::new("bluetoothctl").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Err(e) => {
                logs.gateway.write(json!({"ev": "ble", "state": "no bluetoothctl", "error": e.to_string()}));
                std::thread::sleep(Duration::from_secs(60));
            }
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    for cmd in ["power on", "menu advertise", &format!("uuids {SERVICE_UUID}"), "discoverable on", "back", "advertise on"] {
                        let _ = writeln!(stdin, "{cmd}");
                        let _ = stdin.flush();
                        std::thread::sleep(Duration::from_millis(300));
                    }
                    logs.gateway.write(json!({"ev": "ble", "state": "advertising"}));
                    // closing stdin ends bluetoothctl and with it the advertisement, so hold it until the child ends
                    while child.try_wait().ok().flatten().is_none() {
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
                logs.gateway.write(json!({"ev": "ble", "state": "advertising stopped"}));
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}
