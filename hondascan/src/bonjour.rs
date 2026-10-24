//! Bonjour (multicast DNS) for the phone gateway, so the phone finds the laptop by itself instead of by a typed address.
//!
//! The gateway announces `_openhonda._tcp` and answers the questions a phone asks about it: which instances exist (PTR),
//! where one is (SRV: host name and port), its details (TXT), and the host's addresses (A). The host name is the laptop's own
//! with an `openhonda-` prefix, so it never competes with a name another program (avahi, systemd-resolved) already
//! publishes; its addresses are the laptop's IPv4 addresses on the network the question came from. Answers go out on that
//! network only. The multicast port is shared with whoever else listens on it (SO_REUSEADDR and SO_REUSEPORT). std only,
//! plus libc for the interface list and the shared socket.
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::{Duration, Instant};

pub const SERVICE_TYPE: &str = "_openhonda._tcp";
const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const TTL: u32 = 120;
const REJOIN: Duration = Duration::from_secs(30); // a network that appears later (a hotspot) gets joined

const T_A: u16 = 1;
const T_PTR: u16 = 12;
const T_TXT: u16 = 16;
const T_SRV: u16 = 33;
const T_ANY: u16 = 255;

/// What is announced: `instance` and `host` are single DNS labels.
#[derive(Clone, Debug)]
pub struct Ad {
    pub instance: String,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Iface {
    pub addr: Ipv4Addr,
    pub mask: Ipv4Addr,
}

impl Iface {
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let m = u32::from(self.mask);
        u32::from(self.addr) & m == u32::from(ip) & m
    }
}

/// A label made of letters, digits and hyphens, from a host name.
fn label(s: &str) -> String {
    let l: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    l.trim_matches('-').chars().take(40).collect()
}

impl Ad {
    pub fn for_this_host(port: u16) -> Ad {
        let name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
        let name = label(name.trim());
        let name = if name.is_empty() { "laptop".to_string() } else { name };
        Ad { instance: format!("OpenHonda on {name}"), host: format!("openhonda-{name}"), port }
    }

    /// `_openhonda._tcp.local` as its three labels.
    fn service(&self) -> Vec<String> {
        SERVICE_TYPE.split('.').chain(["local"]).map(String::from).collect()
    }

    fn instance_name(&self) -> Vec<String> {
        std::iter::once(self.instance.clone()).chain(self.service()).collect()
    }

    fn host_name(&self) -> Vec<String> {
        vec![self.host.clone(), "local".into()]
    }
}

// -- wire format --------------------------------------------------------------------------------------------------------

fn put_name(out: &mut Vec<u8>, labels: &[String]) {
    for l in labels {
        out.push(l.len().min(63) as u8);
        out.extend_from_slice(&l.as_bytes()[..l.len().min(63)]);
    }
    out.push(0);
}

fn put_record(out: &mut Vec<u8>, name: &[String], rtype: u16, flush: bool, rdata: &[u8]) {
    put_name(out, name);
    out.extend_from_slice(&rtype.to_be_bytes());
    out.extend_from_slice(&(if flush { 0x8001u16 } else { 1u16 }).to_be_bytes());
    out.extend_from_slice(&TTL.to_be_bytes());
    out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    out.extend_from_slice(rdata);
}

/// Name at `pos` (following compression pointers) and the position after it in the message.
fn get_name(msg: &[u8], mut pos: usize) -> Option<(Vec<String>, usize)> {
    let mut labels = vec![];
    let mut after = None;
    for _ in 0..32 {
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            return Some((labels, after.unwrap_or(pos + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let low = *msg.get(pos + 1)? as usize;
            after.get_or_insert(pos + 2);
            pos = (len & 0x3F) << 8 | low;
        } else {
            labels.push(String::from_utf8_lossy(msg.get(pos + 1..pos + 1 + len)?).into_owned());
            pos += 1 + len;
        }
    }
    None
}

fn same(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// What one mDNS message asks about us.
#[derive(Clone, Copy, Default)]
pub struct Asked {
    ptr: bool,
    srv: bool,
    txt: bool,
    a: bool,
}

impl Asked {
    fn any(&self) -> bool {
        self.ptr || self.srv || self.txt || self.a
    }
}

/// The questions in one mDNS message that are about us, or None when it asks nothing about us (or is itself a reply). Cheap, so
/// the caller can look up the network interfaces only for the messages that need an answer.
pub fn asked(query: &[u8], ad: &Ad) -> Option<Asked> {
    if query.len() < 12 || query[2] & 0x80 != 0 {
        return None;
    }
    let questions = u16::from_be_bytes([query[4], query[5]]) as usize;
    let mut asked = Asked::default();
    let mut pos = 12;
    for _ in 0..questions {
        let (name, next) = get_name(query, pos)?;
        let qtype = u16::from_be_bytes([*query.get(next)?, *query.get(next + 1)?]);
        pos = next + 4;
        let want = |t: u16| qtype == t || qtype == T_ANY;
        if same(&name, &ad.service()) && want(T_PTR) {
            asked.ptr = true;
        }
        if same(&name, &ad.instance_name()) {
            asked.srv |= want(T_SRV);
            asked.txt |= want(T_TXT);
        }
        if same(&name, &ad.host_name()) && want(T_A) {
            asked.a = true;
        }
    }
    asked.any().then_some(asked)
}

/// The reply to one mDNS message, or None when it asks nothing about us (or is itself a reply). `addrs` are the addresses to
/// give for the host: the laptop's, on the network the question came from.
pub fn respond(query: &[u8], ad: &Ad, addrs: &[Ipv4Addr]) -> Option<Vec<u8>> {
    if addrs.is_empty() {
        return None;
    }
    Some(reply(asked(query, ad)?, ad, addrs))
}

fn reply(asked: Asked, ad: &Ad, addrs: &[Ipv4Addr]) -> Vec<u8> {
    let Asked { ptr, srv, txt, a } = asked;
    // what was asked goes in the answers; a question about the service also gets the details a phone needs next, as
    // additional records
    let (mut answers, mut extra) = (Records::default(), Records::default());
    if ptr {
        let mut r = vec![];
        put_name(&mut r, &ad.instance_name());
        answers.add(&ad.service(), T_PTR, false, &r);
    }
    if srv || ptr {
        let mut r = vec![0, 0, 0, 0];
        r.extend_from_slice(&ad.port.to_be_bytes());
        put_name(&mut r, &ad.host_name());
        (if srv { &mut answers } else { &mut extra }).add(&ad.instance_name(), T_SRV, true, &r);
    }
    if txt || ptr {
        (if txt { &mut answers } else { &mut extra }).add(&ad.instance_name(), T_TXT, true, &[3, b'v', b'=', b'1']);
    }
    if a || ptr || srv {
        for ip in addrs {
            (if a { &mut answers } else { &mut extra }).add(&ad.host_name(), T_A, true, &ip.octets());
        }
    }
    let mut out = vec![0, 0, 0x84, 0, 0, 0];
    out.extend_from_slice(&answers.count.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&extra.count.to_be_bytes());
    out.extend_from_slice(&answers.bytes);
    out.extend_from_slice(&extra.bytes);
    out
}

#[derive(Default)]
struct Records {
    bytes: Vec<u8>,
    count: u16,
}

impl Records {
    fn add(&mut self, name: &[String], rtype: u16, flush: bool, rdata: &[u8]) {
        put_record(&mut self.bytes, name, rtype, flush, rdata);
        self.count += 1;
    }
}

// -- sockets and interfaces ---------------------------------------------------------------------------------------------

/// The laptop's IPv4 interfaces that are up and can multicast (not loopback).
pub fn interfaces() -> Vec<Iface> {
    let mut out = vec![];
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut p = head;
        while !p.is_null() {
            let ifa = &*p;
            let flags = ifa.ifa_flags as i32;
            if !ifa.ifa_addr.is_null()
                && (*ifa.ifa_addr).sa_family as i32 == libc::AF_INET
                && flags & libc::IFF_UP != 0
                && flags & libc::IFF_LOOPBACK == 0
                && flags & libc::IFF_MULTICAST != 0
            {
                let addr = u32::from_be((*(ifa.ifa_addr as *const libc::sockaddr_in)).sin_addr.s_addr);
                let mask = if ifa.ifa_netmask.is_null() { u32::MAX } else { u32::from_be((*(ifa.ifa_netmask as *const libc::sockaddr_in)).sin_addr.s_addr) };
                out.push(Iface { addr: Ipv4Addr::from(addr), mask: Ipv4Addr::from(mask) });
            }
            p = ifa.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    out
}

/// A UDP socket on the mDNS port that shares it with other listeners.
fn shared_socket() -> std::io::Result<UdpSocket> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let one: libc::c_int = 1;
        for opt in [libc::SO_REUSEADDR, libc::SO_REUSEPORT] {
            libc::setsockopt(fd, libc::SOL_SOCKET, opt, &one as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as u32);
        }
        let addr = libc::sockaddr_in { sin_family: libc::AF_INET as u16, sin_port: MDNS_PORT.to_be(), sin_addr: libc::in_addr { s_addr: 0 }, sin_zero: [0; 8] };
        if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as u32) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

fn send_from(sock: &UdpSocket, iface: Ipv4Addr, packet: &[u8]) {
    let a = libc::in_addr { s_addr: u32::from(iface).to_be() };
    unsafe {
        libc::setsockopt(sock.as_raw_fd(), libc::IPPROTO_IP, libc::IP_MULTICAST_IF, &a as *const _ as *const libc::c_void, std::mem::size_of::<libc::in_addr>() as u32);
    }
    let _ = sock.send_to(packet, SocketAddrV4::new(GROUP, MDNS_PORT));
}

/// Announce the gateway until the process ends. Questions from a network the laptop is not on are ignored.
pub fn advertise(ad: Ad) -> Result<(), String> {
    let sock = shared_socket().map_err(|e| format!("bonjour: cannot open the mDNS port: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    let _ = sock.set_multicast_ttl_v4(255);
    let join = |sock: &UdpSocket| {
        for i in interfaces() {
            let _ = sock.join_multicast_v4(&GROUP, &i.addr); // joining twice is an error we do not need to hear about
        }
    };
    join(&sock);
    std::thread::spawn(move || {
        let mut joined = Instant::now();
        let mut buf = [0u8; 2048];
        loop {
            if joined.elapsed() > REJOIN {
                join(&sock);
                joined = Instant::now();
            }
            let Ok((n, SocketAddr::V4(src))) = sock.recv_from(&mut buf) else { continue };
            // the network interfaces are looked up only for a message that asks about us: mDNS is chatty
            let Some(asked) = asked(&buf[..n], &ad) else { continue };
            let Some(iface) = interfaces().into_iter().find(|i| i.contains(*src.ip())) else { continue };
            send_from(&sock, iface.addr, &reply(asked, &ad, &[iface.addr]));
        }
    });
    Ok(())
}
