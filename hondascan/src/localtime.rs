//! The wall-clock date and time of a Unix timestamp in the machine's time zone, as the C library's localtime and strftime
//! give it (journal lines, recording folder names).

/// `"%Y-%m-%d %H:%M:%S"` of `t` seconds since the epoch, local time.
pub fn stamp(t: f64) -> String {
    let (y, mo, d, h, mi, s) = fields(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// `"%Y-%m-%d_%H%M%S"`: the same, as a folder name.
pub fn folder_stamp(t: f64) -> String {
    let (y, mo, d, h, mi, s) = fields(t);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}")
}

/// (year, month, day, hour, minute, second)
#[cfg(unix)]
fn fields(t: f64) -> (i32, u32, u32, u32, u32, u32) {
    unsafe {
        let tt = t as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
    }
}

#[cfg(windows)]
fn fields(t: f64) -> (i32, u32, u32, u32, u32, u32) {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    const EPOCH_TO_1601: i64 = 11_644_473_600; // seconds from 1601-01-01 (FILETIME) to 1970-01-01
    let ticks = ((t as i64 + EPOCH_TO_1601).max(0) as u64) * 10_000_000; // 100 ns units
    unsafe {
        let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
        let (mut utc, mut local): (SYSTEMTIME, SYSTEMTIME) = (std::mem::zeroed(), std::mem::zeroed());
        FileTimeToSystemTime(&ft, &mut utc);
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local); // null zone: the current one
        (local.wYear as i32, local.wMonth as u32, local.wDay as u32, local.wHour as u32, local.wMinute as u32, local.wSecond as u32)
    }
}
