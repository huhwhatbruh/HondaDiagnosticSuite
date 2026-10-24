//! A simulated ECU that answers exactly as a recorded car did (profiles/, made by the profile-from-capture binary).
//! Port of hondascan/simcapture.py: each request the car answered gets the answer it had at the same point of the
//! recording, so data lists play the drive back in a loop; a request the car never answered gets no answer. The 2015
//! Accord's PCM (0x0E, PGM-FI and CVT) answered only on the OBD-II functional id 0x18DB33F1, so that is the only id it
//! listens on (unless `physical`: a J2534 recording, where it listens on its own id).
use crate::honda::{tx_id, OBD_FUNCTIONAL_ID};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

/// The recorded-car profiles directory.
pub fn profiles_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HONDASCAN_PROFILES") {
        return p.into();
    }
    crate::resource("profiles")
}

pub struct CaptureEcu {
    pub addr: u32,
    pub system: String,
    pub also: Vec<String>,
    pub listens: Vec<u32>,
    replies: HashMap<String, (Vec<f64>, Vec<Vec<u8>>)>,
    seconds: f64,
    t0: Instant,
    pub requests: Vec<Vec<u8>>,
    pub offline: bool,
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

impl CaptureEcu {
    pub fn handle(&mut self, req: &[u8]) -> Option<Vec<u8>> {
        self.requests.push(req.to_vec());
        let key: String = req.iter().map(|b| format!("{b:02X}")).collect();
        let rec = self.replies.get(&key);
        if self.offline {
            return None;
        }
        let (times, answers) = rec?;
        // a profile entry with an empty reply sequence has no answer to give
        if answers.is_empty() || times.is_empty() { return None; }
        let t = self.t0.elapsed().as_secs_f64() % self.seconds;
        // bisect_right(times, t) - 1, floored at 0
        let i = times.partition_point(|x| *x <= t).saturating_sub(1);
        Some(answers[i].clone())
    }
}

/// The ECUs of profile `name` (`systems` says which database systems each address is: [(address, [system, also...])]).
pub fn load(name: &str, systems: &[(u32, &[&str])], physical: bool) -> Result<Vec<CaptureEcu>, String> {
    let path = profiles_dir().join(format!("{name}.json.gz"));
    let f = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(f).read_to_string(&mut text).map_err(|e| e.to_string())?;
    let prof: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let seconds = prof["seconds"].as_f64().unwrap_or(1.0).max(1.0);
    let mut out = vec![];
    for (a, e) in prof["ecus"].as_object().ok_or("no ecus")? {
        let addr = u32::from_str_radix(a, 16).map_err(|e| e.to_string())?;
        let sys = systems.iter().find(|(x, _)| *x == addr).map(|(_, s)| *s).ok_or_else(|| format!("no system for 0x{addr:02X}"))?;
        let mut replies = HashMap::new();
        for (req, seq) in e["replies"].as_object().ok_or("no replies")? {
            let seq = seq.as_array().ok_or("bad replies")?;
            replies.insert(
                req.clone(),
                (seq.iter().map(|p| p[0].as_f64().unwrap_or(0.0)).collect(), seq.iter().map(|p| unhex(p[1].as_str().unwrap_or(""))).collect()),
            );
        }
        out.push(CaptureEcu {
            addr,
            system: sys[0].to_string(),
            also: sys[1..].iter().map(|s| s.to_string()).collect(),
            listens: vec![if physical { tx_id(addr) } else { OBD_FUNCTIONAL_ID }],
            replies,
            seconds,
            t0: Instant::now(),
            requests: vec![],
            offline: false,
        });
    }
    Ok(out)
}
