//! Which serial ports there are, and what is known about them: the USB ids, so the GODIAG GD101 can be told from other adapters.
//! Linux reads sysfs; macOS reads `ioreg`; Windows reads the registry (the ports present now, and the USB device each one belongs to).

/// [(device, vid, pid, product)] for USB serial ports, from sysfs.
#[cfg(all(unix, not(target_os = "macos")))]
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

/// [(device, vid, pid, product)] for USB serial ports. macOS has no sysfs: `ioreg` lists the USB tree, and the
/// callout node (`/dev/cu.*`, not `/dev/tty.*`, which waits on carrier) carries the port. vid/pid are lowercase hex,
/// the same shape Linux sysfs uses, so the GD101 id `e327:2534` matches.
#[cfg(target_os = "macos")]
pub fn serial_ports() -> Vec<(String, String, String, String)> {
    for class in ["IOUSBHostDevice", "IOUSBDevice"] {
        let Ok(out) = std::process::Command::new("ioreg").args(["-r", "-c", class, "-l"]).output() else { continue };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        if text.contains("+-o") {
            return ports_from_ioreg(&text);
        }
    }
    vec![]
}

/// One `ioreg -r -c IOUSBHostDevice -l` tree. A child inherits its parent's ids until it sets its own. Only callout devices.
#[cfg(any(target_os = "macos", test))]
fn ports_from_ioreg(text: &str) -> Vec<(String, String, String, String)> {
    struct Node {
        indent: usize,
        vendor: String,
        product: String,
        name: String,
    }
    let mut stack: Vec<Node> = vec![];
    let mut out = vec![];
    for line in text.lines() {
        if let Some(col) = line.find("+-o ") {
            while stack.last().is_some_and(|n| n.indent >= col) {
                stack.pop();
            }
            let (vendor, product, name) = stack.last().map(|n| (n.vendor.clone(), n.product.clone(), n.name.clone())).unwrap_or_default();
            stack.push(Node { indent: col, vendor, product, name });
            continue;
        }
        let Some(node) = stack.last_mut() else { continue };
        let trimmed = line.trim();
        if let Some(n) = ioreg_int(trimmed, "\"idVendor\"") {
            node.vendor = format!("{n:04x}");
        } else if let Some(n) = ioreg_int(trimmed, "\"idProduct\"") {
            node.product = format!("{n:04x}");
        } else if let Some(name) = ioreg_str(trimmed, "\"USB Product Name\"") {
            node.name = name;
        } else if let Some(dev) = ioreg_str(trimmed, "\"IOCalloutDevice\"") {
            if dev.starts_with("/dev/cu.") {
                out.push((dev, node.vendor.clone(), node.product.clone(), node.name.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(any(target_os = "macos", test))]
fn ioreg_int(line: &str, key: &str) -> Option<u32> {
    let rest = line.split_once(key)?.1.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    rest.parse().ok()
}

#[cfg(any(target_os = "macos", test))]
fn ioreg_str(line: &str, key: &str) -> Option<String> {
    let rest = line.split_once(key)?.1.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let rest = rest.strip_prefix('"')?;
    Some(rest.strip_suffix('"').unwrap_or(rest).to_string())
}

/// ModemManager probes new serial ports with AT commands, which an ELM327 answers: replies go to the wrong reader and the
/// adapter runs out of step or resets. A warning, or None when it is not running or the udev rule (packaging/60-hondascan.rules,
/// ID_MM_DEVICE_IGNORE) tells it to leave `dev` alone.
#[cfg(target_os = "linux")]
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

/// ModemManager is a Linux program: no other system probes serial ports behind our back that way.
#[cfg(not(target_os = "linux"))]
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

#[cfg(test)]
mod tests {
    use super::ports_from_ioreg;

    #[test]
    fn a_callout_device_inherits_the_usb_ids_and_the_dialin_node_is_ignored() {
        let text = r#"
+-o M-VCI@01100000  <class IOUSBHostDevice, id 0x1>
  | {
  |   "idProduct" = 24577
  |   "USB Product Name" = "M-VCI"
  |   "idVendor" = 1027
  | }
  +-o M-VCI@0  <class IOUSBHostInterface, id 0x2>
  | | {
  | |   "idProduct" = 24577
  | |   "idVendor" = 1027
  | | }
  | +-o AppleUSBFTDI  <class IOUserSerial, id 0x3>
  |   +-o IOSerialBSDClient  <class IOSerialBSDClient, id 0x4>
  |       {
  |         "IOCalloutDevice" = "/dev/cu.usbserial-A0000001"
  |         "IODialinDevice" = "/dev/tty.usbserial-A0000001"
  |       }
+-o GD101@2  <class IOUSBHostDevice, id 0x5>
  | {
  |   "idProduct" = 9524
  |   "USB Product Name" = "GD101"
  |   "idVendor" = 58151
  | }
  +-o IOSerialBSDClient  <class IOSerialBSDClient, id 0x6>
      {
        "IOCalloutDevice" = "/dev/cu.usbmodem1"
      }
"#;
        assert_eq!(
            ports_from_ioreg(text),
            vec![
                ("/dev/cu.usbmodem1".into(), "e327".into(), "2534".into(), "GD101".into()),
                ("/dev/cu.usbserial-A0000001".into(), "0403".into(), "6001".into(), "M-VCI".into()),
            ]
        );
    }
}
