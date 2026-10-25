//! Which serial ports there are, and what is known about them: the USB ids, so the GODIAG GD101 can be told from other adapters.
//! Linux reads sysfs; Windows reads the registry (the ports present now, and the USB device each one belongs to).

/// [(device, vid, pid, product)] for USB serial ports, from sysfs.
#[cfg(unix)]
pub fn serial_ports() -> Vec<(String, String, String, String)> {
    let mut devs: Vec<String> = vec![];
    for prefix in ["ttyACM", "ttyUSB"] {
        if let Ok(rd) = std::fs::read_dir("/dev") {
            devs.extend(rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(prefix)).map(|n| format!("/dev/{n}")));
        }
    }
    devs.sort();
    let mut out = vec![];
    for dev in devs {
        let base = std::path::Path::new(&dev).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let mut d = std::fs::canonicalize(format!("/sys/class/tty/{base}/device")).unwrap_or_default();
        let mut info: std::collections::HashMap<&str, String> = Default::default();
        while d.as_os_str() != "/" && !d.as_os_str().is_empty() && !(info.contains_key("idVendor") && info.contains_key("idProduct")) {
            for k in ["idVendor", "idProduct", "product"] {
                if !info.contains_key(k) {
                    if let Ok(v) = std::fs::read_to_string(d.join(k)) {
                        info.insert(k, v.trim().to_string());
                    }
                }
            }
            d = d.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        }
        out.push((dev, info.remove("idVendor").unwrap_or_default(), info.remove("idProduct").unwrap_or_default(), info.remove("product").unwrap_or_default()));
    }
    out
}

/// ModemManager probes new serial ports with AT commands, which an ELM327 answers: replies go to the wrong reader and the
/// adapter runs out of step or resets. A warning, or None when it is not running or the udev rule (packaging/60-hondascan.rules,
/// ID_MM_DEVICE_IGNORE) tells it to leave `dev` alone.
#[cfg(unix)]
pub fn modem_manager_warning(dev: &str) -> Option<String> {
    let running = std::fs::read_dir("/proc").ok()?.filter_map(Result::ok).any(|e| {
        e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "ModemManager")
    });
    if !running {
        return None;
    }
    use std::os::unix::fs::MetadataExt;
    if let Ok(m) = std::fs::metadata(dev) {
        let rdev = m.rdev();
        let (major, minor) = (libc::major(rdev), libc::minor(rdev));
        if std::fs::read_to_string(format!("/run/udev/data/c{major}:{minor}")).is_ok_and(|t| t.contains("E:ID_MM_DEVICE_IGNORE=1")) {
            return None;
        }
    }
    Some(format!("ModemManager is running and may probe {dev} with AT commands: install packaging/60-hondascan.rules and replug the adapter, or stop it (sudo systemctl stop ModemManager)"))
}

/// No program on Windows probes serial ports behind our back the way ModemManager does.
#[cfg(not(unix))]
pub fn modem_manager_warning(_dev: &str) -> Option<String> {
    None
}

/// [(device, vid, pid, product)] for the serial ports present. HKLM\HARDWARE\DEVICEMAP\SERIALCOMM lists the COM ports that
/// exist now; HKLM\SYSTEM\CurrentControlSet\Enum\USB\VID_xxxx&PID_xxxx[&MI_xx]\<instance>\Device Parameters\PortName says which
/// USB device owns each (the key stays after the device is unplugged, which is why SERIALCOMM is checked first). A port with
/// no USB entry (a Bluetooth ELM327, a motherboard port) is listed with empty ids.
#[cfg(windows)]
pub fn serial_ports() -> Vec<(String, String, String, String)> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let lm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut present: Vec<String> = lm
        .open_subkey(r"HARDWARE\DEVICEMAP\SERIALCOMM")
        .map(|k| k.enum_values().filter_map(Result::ok).filter_map(|(name, _)| k.get_value::<String, _>(&name).ok()).collect())
        .unwrap_or_default();
    present.sort_by_key(|p| (p.len(), p.clone())); // COM2 before COM10
    let mut usb: std::collections::HashMap<String, (String, String, String)> = Default::default();
    if let Ok(root) = lm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\USB") {
        for id in root.enum_keys().filter_map(Result::ok) {
            let lower = id.to_lowercase();
            let (Some(v), Some(p)) = (lower.strip_prefix("vid_").and_then(|r| r.get(..4)), lower.split("&pid_").nth(1).and_then(|r| r.get(..4))) else { continue };
            let Ok(devices) = root.open_subkey(&id) else { continue };
            for instance in devices.enum_keys().filter_map(Result::ok) {
                let Ok(dev) = devices.open_subkey(&instance) else { continue };
                let Ok(port) = dev.open_subkey("Device Parameters").and_then(|k| k.get_value::<String, _>("PortName")) else { continue };
                let name = dev.get_value::<String, _>("FriendlyName").map(|f| f.split(" (COM").next().unwrap_or("").to_string()).or_else(|_| dev.get_value::<String, _>("DeviceDesc").map(|d| d.rsplit(';').next().unwrap_or("").to_string())).unwrap_or_default();
                usb.insert(port, (v.to_string(), p.to_string(), name));
            }
        }
    }
    present.into_iter().map(|port| { let (v, p, n) = usb.remove(&port).unwrap_or_default(); (port, v, p, n) }).collect()
}
