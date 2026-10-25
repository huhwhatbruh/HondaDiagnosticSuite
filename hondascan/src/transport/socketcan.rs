//! Linux SocketCAN raw sockets (can0, vcan0, slcan0 after slcand, gs_usb/candleLight, PEAK, Kvaser...). Windows has no
//! equivalent; there the same adapters are reached through J2534 or slcan.
use super::can::{now, secs, BusCore, CanBus, CanFrame, CaptureLog, CAN_EFF_FLAG, CAN_EFF_MASK, CAN_SFF_MASK};
use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::Instant;

/// CAN_RAW_FILTER value: one exact-match filter per extended id (the kernel ORs them).
pub fn socketcan_filters(ids: &BTreeSet<u32>) -> Vec<u8> {
    let mut out = vec![];
    for i in ids {
        out.extend(((i & CAN_EFF_MASK) | CAN_EFF_FLAG).to_ne_bytes());
        out.extend((CAN_EFF_MASK | CAN_EFF_FLAG).to_ne_bytes());
    }
    out
}

/// Linux SocketCAN raw socket (can0, vcan0, slcan0 after slcand, gs_usb/candleLight, PEAK, Kvaser...).
pub struct SocketCanBus {
    core: BusCore,
    fd: libc::c_int,
}

impl SocketCanBus {
    pub fn open(iface: &str, log: Option<Arc<CaptureLog>>) -> io::Result<SocketCanBus> {
        unsafe {
            let fd = libc::socket(libc::AF_CAN, libc::SOCK_RAW, libc::CAN_RAW);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name = std::ffi::CString::new(iface).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let b = name.as_bytes_with_nul();
            if b.len() > ifr.ifr_name.len() {
                libc::close(fd);
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "interface name too long"));
            }
            for (d, s) in ifr.ifr_name.iter_mut().zip(b) {
                *d = *s as libc::c_char;
            }
            if libc::ioctl(fd, libc::SIOCGIFINDEX, &mut ifr) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            let mut addr: libc::sockaddr_can = std::mem::zeroed();
            addr.can_family = libc::AF_CAN as libc::sa_family_t;
            addr.can_ifindex = ifr.ifr_ifru.ifru_ifindex;
            if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_can>() as u32) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            Ok(SocketCanBus { core: BusCore::with_log(log), fd })
        }
    }
}

impl CanBus for SocketCanBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn apply_filters(&self) {
        let f = socketcan_filters(&self.ids());
        unsafe {
            libc::setsockopt(self.fd, libc::SOL_CAN_RAW, libc::CAN_RAW_FILTER, f.as_ptr() as *const _, f.len() as u32);
        }
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let cid = if frame.extended { (frame.can_id & CAN_EFF_MASK) | CAN_EFF_FLAG } else { frame.can_id & CAN_SFF_MASK };
        let mut raw = [0u8; 16]; // struct can_frame: can_id (u32), can_dlc (u8), 3 pad, data[8]
        raw[..4].copy_from_slice(&cid.to_ne_bytes());
        raw[4] = frame.data.len().min(8) as u8;
        raw[8..8 + frame.data.len().min(8)].copy_from_slice(&frame.data[..frame.data.len().min(8)]);
        let n = unsafe { libc::send(self.fd, raw.as_ptr() as *const _, 16, 0) };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + secs(timeout);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut p, 1, left.as_millis().min(i32::MAX as u128).max(1) as i32) };
            if r <= 0 {
                if r < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    return None;
                }
                continue;
            }
            let mut raw = [0u8; 16];
            let n = unsafe { libc::recv(self.fd, raw.as_mut_ptr() as *mut _, 16, 0) };
            if n < 16 {
                continue;
            }
            let cid = u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if cid & 0x6000_0000 != 0 {
                continue; // RTR / error frames
            }
            let ext = cid & CAN_EFF_FLAG != 0;
            let dlc = (raw[4] as usize).min(8);
            return Some(CanFrame { can_id: cid & if ext { CAN_EFF_MASK } else { CAN_SFF_MASK }, data: raw[8..8 + dlc].to_vec(), extended: ext, timestamp: now() });
        }
    }

    fn close(&self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}
