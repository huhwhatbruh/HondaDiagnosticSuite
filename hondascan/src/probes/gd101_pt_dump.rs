//! Research only: run gd101_pt_probe.exe under Wine (and strace), and while it pauses with GODIAG_PT32.dll loaded, copy the DLL's
//! unpacked image out of the process. The DLL ships packed (README.md, "GD101"), so this memory image is the readable form of its
//! code. Vendor bytes: everything goes to $GD101_PKG (default ~/.cache/hdsvendor/gd101), never into the repo.
//!
//!   gd101-pt-dump [--ecu 0E] [--pause 8]
//!
//! Yama (ptrace_scope 1) lets only an ancestor read /proc/<pid>/mem. Wine detaches the Windows process (its parent becomes
//! systemd), so this program makes itself the child subreaper first: the probe is then re-parented to it.
//! Writes: probe.strace (every device call), probe.out (the probe's printout), image.bin (base .. base + SizeOfImage, unreadable
//! pages as zeros), image.base and image.maps (the process's modules).
use std::io::{BufRead, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const DLL_WIN: &str = r"C:\Program Files (x86)\GODIAG\GODIAG J2534 Driver\GODIAG_PT32.dll";

fn pkg() -> PathBuf {
    std::env::var_os("GD101_PKG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache/hdsvendor/gd101"))
}

/// SizeOfImage from a PE header (optional header offset 56).
pub fn size_of_image(head: &[u8]) -> Option<u32> {
    let u32_at = |o: usize| head.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    u32_at(u32_at(0x3C)? as usize + 24 + 56)
}

/// (lo, hi, readable) of one /proc/<pid>/maps line.
pub fn parse_map_line(ln: &str) -> Option<(u64, u64, bool)> {
    let mut f = ln.split_whitespace();
    let (lo, hi) = f.next()?.split_once('-')?;
    Some((u64::from_str_radix(lo, 16).ok()?, u64::from_str_radix(hi, 16).ok()?, f.next()?.contains('r')))
}

/// The pid whose maps hold `name`, and the module's base.
fn find(name: &str) -> Option<(u32, u64)> {
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(maps) = std::fs::read(format!("/proc/{pid}/maps")) else { continue };
        for ln in String::from_utf8_lossy(&maps).lines() {
            if ln.contains(name) {
                return Some((pid, u64::from_str_radix(ln.split('-').next()?, 16).ok()?));
            }
        }
    }
    None
}

fn dump(pid: u32, base: u64, size: usize) -> Result<Vec<u8>, String> {
    let mut img = vec![0u8; size];
    let mem = std::fs::File::open(format!("/proc/{pid}/mem")).map_err(|e| format!("/proc/{pid}/mem: {e}"))?;
    let maps = std::fs::read(format!("/proc/{pid}/maps")).map_err(|e| format!("/proc/{pid}/maps: {e}"))?;
    for ln in String::from_utf8_lossy(&maps).lines() {
        let Some((lo, hi, readable)) = parse_map_line(ln) else { continue };
        let (lo, hi) = (lo.max(base), hi.min(base + size as u64));
        if lo >= hi || !readable {
            continue;
        }
        // an unreadable page leaves zeros
        let dst = &mut img[(lo - base) as usize..(hi - base) as usize];
        let mut done = 0;
        while done < dst.len() {
            match mem.read_at(&mut dst[done..], lo + done as u64) {
                Ok(0) | Err(_) => break,
                Ok(n) => done += n,
            }
        }
    }
    Ok(img)
}

pub fn run(args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    let (mut ecu, mut pause) = ("0E".to_string(), 8u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ecu" => ecu = it.next().ok_or("--ecu needs a value")?.clone(),
            "--pause" => pause = it.next().and_then(|v| v.parse().ok()).ok_or("--pause needs an integer")?,
            _ => return Err("usage: gd101-pt-dump [--ecu 0E] [--pause 8]".into()),
        }
    }
    let pkg = pkg();
    let dll = pkg.join("wine/drive_c/Program Files (x86)/GODIAG/GODIAG J2534 Driver/GODIAG_PT32.dll");
    let dir = pkg.join("probe");
    let at = |n: &str| -> PathBuf { dir.join(n) };
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    let (reader, writer) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;
    let mut child = Command::new("strace")
        .args(["-f", "-y", "-tt", "-xx", "-s", "4096", "-e", "trace=openat,read,write,ioctl,close", "-o"])
        .arg(at("probe.strace"))
        .arg("wine")
        .arg(at("gd101_pt_probe.exe"))
        .args([DLL_WIN, &ecu, &pause.to_string()])
        .env("WINEPREFIX", pkg.join("wine"))
        .env("WINEDEBUG", "-all")
        .stdout(writer.try_clone().map_err(|e| e.to_string())?)
        .stderr(writer)
        .spawn()
        .map_err(|e| format!("strace: {e}"))?;
    let mut log = std::fs::File::create(at("probe.out")).map_err(|e| format!("probe.out: {e}"))?;
    let mut dumped = false;
    let mut rd = std::io::BufReader::new(reader);
    let mut raw = vec![];
    while rd.read_until(b'\n', &mut raw).map_err(|e| e.to_string())? > 0 {
        let ln = String::from_utf8_lossy(&raw).into_owned();
        raw.clear();
        let _ = log.write_all(ln.as_bytes());
        if ln.contains("MESA") || ln.contains("pci id") {
            continue;
        }
        let _ = write!(out, "{ln}");
        if ln.starts_with("PAUSE") && !dumped {
            std::thread::sleep(Duration::from_secs(1));
            let Some((pid, base)) = find("GODIAG_PT32.dll") else {
                let _ = writeln!(out, "DLL not mapped in any process");
                continue;
            };
            let head = std::fs::read(&dll).map_err(|e| format!("{}: {e}", dll.display()))?;
            let size = size_of_image(&head[..head.len().min(4096)]).ok_or("not a PE file")? as usize;
            let img = dump(pid, base, size)?;
            let w = |n: &str, d: &[u8]| std::fs::write(at(n), d).map_err(|e| format!("{n}: {e}"));
            w("image.bin", &img)?;
            w("image.base", format!("{base:#x}\n").as_bytes())?;
            w("image.maps", &std::fs::read(format!("/proc/{pid}/maps")).unwrap_or_default())?; // names the API pointers
            let pages = img.chunks(4096).filter(|c| c.iter().any(|&b| b != 0)).count();
            let _ = writeln!(out, "dumped {:#x} bytes at {base:#x} from pid {pid} ({pages} non-empty pages)", img.len());
            dumped = true;
        }
    }
    Ok(child.wait().map_err(|e| e.to_string())?.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pe_size_of_image() {
        let mut h = vec![0u8; 0x200];
        h[0x3C] = 0x80;
        h[0x80 + 24 + 56..0x80 + 24 + 60].copy_from_slice(&0x1234_5000u32.to_le_bytes());
        assert_eq!(size_of_image(&h), Some(0x1234_5000));
        assert_eq!(size_of_image(&h[..0x40]), None);
    }

    #[test]
    fn maps_lines() {
        let l = "7f00-7f80 r-xp 00000000 08:01 1234 /x/GODIAG_PT32.dll";
        assert_eq!(parse_map_line(l), Some((0x7f00, 0x7f80, true)));
        assert_eq!(parse_map_line("1000-2000 -w-p 0 0:0 0"), Some((0x1000, 0x2000, false)));
        assert_eq!(parse_map_line("junk"), None);
    }
}
