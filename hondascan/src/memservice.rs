//! RPC methods for memory reads (memory.rs). Port of hondascan/memservice.py. The memory probe ran once on the car
//! (2026-10-15: 8 requests; format 0x14 answered NRC 0x33 in the default and extended sessions, 0x24/0x44/0x34 got
//! NRC 0x13; no read succeeded -- README.md "Open work"). Each needs `confirm: true`, like `clear`: the person has
//! chosen to send a service the ECU has not been asked before. Addresses and lengths come as numbers or hex strings.
use crate::honda::PROBE_ADDRESS;
use crate::ident::read_ecu_info;
use crate::ramwatch::{check_ecu, profile, profile_system, read_profile, BLOCK, PROFILE_NAMES};
use crate::service::{Service, ServiceError};
use crate::uds::UdsError;
use serde_json::{json, Value};

/// Python's `int(v, 0)` for a string, `int(v)` for a number.
pub fn num(v: &Value) -> Result<i64, ServiceError> {
    match v {
        Value::String(s) => {
            let t = s.trim().replace('_', "");
            let (neg, t) = match t.strip_prefix('-') {
                Some(r) => (true, r.to_string()),
                None => (false, t.trim_start_matches('+').to_string()),
            };
            let low = t.to_lowercase();
            let parsed = if let Some(h) = low.strip_prefix("0x") {
                i64::from_str_radix(h, 16)
            } else if let Some(b) = low.strip_prefix("0b") {
                i64::from_str_radix(b, 2)
            } else if let Some(o) = low.strip_prefix("0o") {
                i64::from_str_radix(o, 8)
            } else {
                low.parse()
            };
            parsed.map(|x| if neg { -x } else { x }).map_err(|_| ServiceError::value(format!("invalid literal for int() with base 0: {s:?}")))
        }
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).ok_or_else(|| ServiceError::value("bad number")),
        other => Err(ServiceError { kind: "TypeError".into(), message: format!("int() argument must be a string or a number, not {}", crate::journal::py(other)) }),
    }
}

impl Service {
    /// With `extended`, the reads run in session 10 03 and the ECU is put back in 10 01 after.
    fn memory_ecu<T>(&mut self, address: i64, confirm: bool, extended: bool, body: impl FnOnce(&mut Service, u32) -> Result<T, ServiceError>) -> Result<T, ServiceError> {
        if !confirm {
            return Err(ServiceError::value("memory reads need confirm: true"));
        }
        let a = address as u32;
        if extended {
            self.hc()?.client(a).session(0x03)?;
        } else {
            self.hc()?;
        }
        let r = body(self, a);
        if extended {
            // the ECU's S3 timeout drops it back to 10 01 anyway
            if let Some(hc) = self.hc.as_mut() {
                let _ = hc.client(a).session(0x01);
            }
        }
        r
    }

    pub fn memory_read(&mut self, address: i64, memory_address: &Value, length: i64, fmt: &Value, extended: bool, confirm: bool) -> Result<Value, ServiceError> {
        let (m, f) = (num(memory_address)? as u64, num(fmt)? as u8);
        let data = self.memory_ecu(address, confirm, extended, |s, a| Ok(s.hc()?.read_memory(a, m, length as u64, f)?))?;
        Ok(json!({"hex": data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")}))
    }

    pub fn memory_probe(&mut self, address: i64, memory_address: Option<&Value>, confirm: bool) -> Result<Value, ServiceError> {
        let m = match memory_address {
            Some(v) if !v.is_null() => num(v)? as u64,
            _ => PROBE_ADDRESS,
        };
        let rows = self.memory_ecu(address, confirm, false, |s, a| Ok(s.hc()?.probe_memory(a, m, 2)?))?;
        Ok(json!({"rows": rows.iter().map(|r| json!({"session": r.session, "format": r.format, "outcome": r.outcome, "data": r.data})).collect::<Vec<_>>()}))
    }

    /// One read of a RAM profile. The ECU must be one the profile was measured on (ramwatch::check_ecu) unless `force_profile`.
    #[allow(clippy::too_many_arguments)]
    pub fn ram_watch(&mut self, address: i64, profile_name: &str, fmt: &Value, extended: bool, block: Option<i64>, force_profile: bool, confirm: bool) -> Result<Value, ServiceError> {
        let Some(words) = profile(profile_name) else {
            return Err(ServiceError::value(format!("unknown profile {profile_name:?} ({})", PROFILE_NAMES.join(", "))));
        };
        let f = num(fmt)? as u8;
        self.hc()?;
        if confirm && !force_profile {
            // the identity read is a plain DID read; refuse before any 0x23
            let gate = (address as u32, profile_name.to_string());
            if self.gate.as_ref().map(|g| (&g.0, g.1)) != Some((&gate, self.scan_generation)) {
                let systems: Vec<String> = self.ecus.iter().filter(|e| e.address == gate.0).map(|e| e.system.clone()).collect();
                let system = profile_system(profile_name, &systems).map_err(|e| ServiceError::value(e.0))?;
                let db = self.db.clone();
                let info = read_ecu_info(&db, system, self.hc()?.client(gate.0))?;
                check_ecu(profile_name, info["ecu_model"].as_str(), false).map_err(|e| ServiceError::value(e.0))?;
                self.gate = Some((gate, self.scan_generation)); // only a pass is remembered: a refusal returned above
            }
        }
        let block = block.map_or(BLOCK, |b| b as u64);
        let values = self.memory_ecu(address, confirm, extended, |s, a| {
            let hc = s.hc()?;
            let mut read = |addr: u64, n: u64| -> Result<Vec<u8>, UdsError> { hc.read_memory(a, addr, n, f) };
            Ok(read_profile(&mut read, words, block)?)
        })?;
        Ok(json!({"values": values.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<String, Value>>()}))
    }
}
