//! Turn a session capture (a recording's frames.log or an ELM `--log` file) into a simulated-ECU profile (profiles/*.json.gz).
//!
//! Every request the car answered is kept with all its distinct answers in time order, so the simulated ECU plays the drive back
//! (engine speed, speed, temperatures move as they did); requests that got NO DATA every time are listed as silent. The VIN
//! (22 F190) is replaced by --vin, so the profile can be committed.
//!
//!   profile-from-capture FRAMES.LOG OUT.json.gz [--vin 1HGCR2F3XFA000000]
//!
//! Output (gzip JSON): {"source", "seconds", "ecus": {"0E": {"replies": {"22F190": [[t, "62F190..."], ...]}, "silent": ["22F187", ...]}}},
//! t in seconds from the first request.
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};

const MIN_GAP_S: f64 = 0.25; // a long drive keeps one answer per request per 0.25 s

fn hex_upper(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Python's `round(x, 3)`: the correctly rounded decimal, read back.
fn round3(x: f64) -> f64 {
    format!("{x:.3}").parse().unwrap()
}

/// ISO-TP receive side for one direction of one address: feed frames, get complete payloads.
#[derive(Default)]
struct Reassembly {
    want: usize,
    buf: Vec<u8>,
}

impl Reassembly {
    fn feed(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        match data[0] >> 4 {
            0 => Some(data.get(1..).unwrap_or(&[])[..((data[0] & 0xF) as usize).min(data.len() - 1)].to_vec()),
            1 if data.len() >= 2 => {
                self.want = (((data[0] & 0xF) as usize) << 8) | data[1] as usize;
                self.buf = data[2..].to_vec();
                None
            }
            2 if self.want > 0 => {
                self.buf.extend_from_slice(&data[1..]);
                if self.buf.len() >= self.want {
                    let out = self.buf[..self.want].to_vec();
                    self.want = 0;
                    Some(out)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

type Answers = Vec<(u8, Vec<Vec<u8>>)>;

/// ISO-TP-reassemble the adapter's headers-on lines: {ecu address: [payload, ...]}, addresses in order of first payload.
fn payloads(lines: &[String], line: &Regex) -> Answers {
    let mut out: Answers = vec![];
    let mut partial: HashMap<u8, Reassembly> = HashMap::new();
    for ln in lines {
        let Some(m) = line.captures(ln) else { continue };
        let addr = u8::from_str_radix(&m[1], 16).unwrap();
        let Some(data) = from_hex(&m[2]) else { continue };
        let kind = data[0] >> 4;
        let p = match kind {
            0 => Reassembly::default().feed(&data),
            1 => {
                partial.entry(addr).or_default().feed(&data);
                None
            }
            2 if partial.contains_key(&addr) => {
                let p = partial.get_mut(&addr).unwrap().feed(&data);
                if p.is_some() {
                    partial.remove(&addr);
                }
                p
            }
            _ => None,
        };
        if let Some(p) = p {
            match out.iter_mut().find(|(a, _)| *a == addr) {
                Some((_, v)) => v.push(p),
                None => out.push((addr, vec![p])),
            }
        }
    }
    out
}

/// (time, request hex, answers) per request line in the capture, in order.
fn exchanges(text: &str) -> Vec<(f64, String, Answers)> {
    let line = Regex::new(r"^18DAF1([0-9A-F]{2})([0-9A-F]+)$").unwrap();
    let (mut out, mut req, mut got, mut t, mut t_last): (Vec<_>, Option<String>, Vec<String>, f64, f64) = (vec![], None, vec![], 0.0, 0.0);
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.starts_with('(') {
            // candump line: keep the clock
            if let Some(v) = raw.find(')').and_then(|i| raw[1..i].parse::<f64>().ok()) {
                t_last = v;
            }
        } else if let Some(cmd) = raw.strip_prefix("# ELM> ") {
            if let Some(r) = req.take() {
                out.push((t, r, payloads(&got, &line)));
            }
            let cmd = cmd.trim().to_uppercase();
            req = (!cmd.is_empty() && from_hex(&cmd).is_some()).then_some(cmd);
            got.clear();
            t = t_last;
        } else if let (Some(l), Some(_)) = (raw.strip_prefix("# ELM< "), &req) {
            got.push(l.trim().to_uppercase());
        }
    }
    if let Some(r) = req {
        out.push((t, r, payloads(&got, &line)));
    }
    out
}

fn mask_vin(req: &str, p: &[u8], vin: &str) -> Vec<u8> {
    if req == "22F190" && p.starts_with(&[0x62, 0xF1, 0x90]) {
        [&p[..3], vin.as_bytes()].concat()
    } else {
        p.to_vec()
    }
}

/// One captured ECU: (address, [(request, [(t, hex)])]) in first-answer order.
type EcuCaptures = Vec<(u8, Vec<(String, Vec<(f64, String)>)>)>;

/// Insertion-ordered ECU table.
#[derive(Default)]
struct Ecus(EcuCaptures);

impl Ecus {
    fn seq(&mut self, addr: u8, req: &str) -> &mut Vec<(f64, String)> {
        let i = self.0.iter().position(|(a, _)| *a == addr).unwrap_or_else(|| {
            self.0.push((addr, vec![]));
            self.0.len() - 1
        });
        let replies = &mut self.0[i].1;
        let j = replies.iter().position(|(r, _)| r == req).unwrap_or_else(|| {
            replies.push((req.to_string(), vec![]));
            replies.len() - 1
        });
        &mut replies[j].1
    }
    fn has(&self, addr: u8, req: &str) -> bool {
        self.0.iter().any(|(a, r)| *a == addr && r.iter().any(|(q, _)| q == req))
    }
    fn to_json(&self, silent: impl Fn(u8) -> Vec<String>) -> Value {
        let mut ecus = Map::new();
        for (addr, replies) in &self.0 {
            let mut r = Map::new();
            for (req, seq) in replies {
                r.insert(req.clone(), Value::Array(seq.iter().map(|(t, h)| json!([t, h])).collect()));
            }
            ecus.insert(format!("{addr:02X}"), json!({"replies": r, "silent": silent(*addr)}));
        }
        Value::Object(ecus)
    }
}

/// An ELM session (`# ELM>` / `# ELM<` lines, headers on).
pub fn build(text: &str, vin: &str) -> Value {
    let mut ecus = Ecus::default();
    let mut asked: Vec<(String, BTreeSet<u8>)> = vec![];
    let (mut t0, mut t_end) = (None, 0.0);
    for (t, req, answers) in exchanges(text) {
        let t0 = *t0.get_or_insert(t);
        t_end = t - t0;
        if !asked.iter().any(|(r, _)| *r == req) {
            asked.push((req.clone(), BTreeSet::new()));
        }
        for (addr, ps) in answers {
            asked.iter_mut().find(|(r, _)| *r == req).unwrap().1.insert(addr);
            ecus.seq(addr, &req); // the table entry exists even when every payload is a repeat
            for p in ps {
                let p = hex_upper(&mask_vin(&req, &p, vin));
                let seq = ecus.seq(addr, &req);
                if seq.last().is_none_or(|l| l.1 != p) {
                    seq.push((round3(t - t0), p));
                }
            }
        }
    }
    let silent = |addr: u8| {
        let mut v: Vec<String> = asked.iter().filter(|(r, who)| who.is_empty() && !ecus.has(addr, r)).map(|(r, _)| r.clone()).collect();
        v.sort();
        v
    };
    json!({"source": "ELM capture, 2015 Accord on the GD101 (VIN masked)", "seconds": round3(t_end), "ecus": ecus.to_json(silent)})
}

/// A raw-CAN recording (frames.log, candump lines with `; TX` / `; RX`): each physical request (18DA<addr>F1) is paired with the
/// next answer from that address that matches it (positive, or negative other than 'pending').
pub fn build_raw(text: &str, vin: &str, source: &str) -> Value {
    let re = Regex::new(r"^\(([\d.]+)\) \S+ (18DA(?:F1)?[0-9A-F]{2}(?:F1)?)#([0-9A-F]+) ; (TX|RX)").unwrap();
    let mut ecus = Ecus::default();
    let (mut tx, mut rx): (HashMap<u8, Reassembly>, HashMap<u8, Reassembly>) = (HashMap::new(), HashMap::new());
    let mut cur: HashMap<u8, String> = HashMap::new();
    let mut asked: Vec<(u8, Vec<(String, bool)>)> = vec![];
    let (mut t0, mut t_end) = (None, 0.0);
    for raw in text.split('\n') {
        let Some(m) = re.captures(raw) else { continue };
        let (Ok(t), Some(data)) = (m[1].parse::<f64>(), from_hex(&m[3])) else { continue };
        let (cid, way) = (&m[2], &m[4]);
        let t0 = *t0.get_or_insert(t);
        t_end = t - t0;
        if way == "TX" && cid.ends_with("F1") && cid.starts_with("18DA") {
            let addr = u8::from_str_radix(&cid[4..6], 16).unwrap();
            let p = if data[0] >> 4 != 3 { tx.entry(addr).or_default().feed(&data) } else { None };
            if let Some(p) = p.filter(|p| !p.is_empty()) {
                let h = hex_upper(&p);
                let i = asked.iter().position(|(a, _)| *a == addr).unwrap_or_else(|| {
                    asked.push((addr, vec![]));
                    asked.len() - 1
                });
                if !asked[i].1.iter().any(|(r, _)| *r == h) {
                    asked[i].1.push((h.clone(), false));
                }
                cur.insert(addr, h);
            }
        } else if way == "RX" && cid.starts_with("18DAF1") {
            let addr = u8::from_str_radix(&cid[6..8], 16).unwrap();
            let p = if data[0] >> 4 != 3 { rx.entry(addr).or_default().feed(&data) } else { None };
            let (Some(p), Some(req)) = (p.filter(|p| !p.is_empty()), cur.get(&addr)) else { continue };
            let sid = u32::from_str_radix(&req[..2], 16).unwrap();
            let (p0, ok) = (p[0] as u32, p[0] == 0x7F);
            if ok && (p.len() < 3 || p[1] as u32 != sid || p[2] == 0x78) || !ok && p0 != sid + 0x40 {
                continue;
            }
            let req = req.clone();
            asked.iter_mut().find(|(a, _)| *a == addr).unwrap().1.iter_mut().find(|(r, _)| *r == req).unwrap().1 = true;
            let h = hex_upper(&mask_vin(&req, &p, vin));
            let seq = ecus.seq(addr, &req);
            if seq.last().is_none_or(|l| l.1 != h && t - t0 - l.0 >= MIN_GAP_S) {
                seq.push((round3(t - t0), h));
            }
        }
    }
    let silent = |addr: u8| {
        let mut v: Vec<String> = asked
            .iter()
            .filter(|(a, _)| *a == addr)
            .flat_map(|(_, reqs)| reqs.iter().filter(|(_, answered)| !answered).map(|(r, _)| r.clone()))
            .collect();
        v.sort();
        v
    };
    json!({"source": source, "seconds": round3(t_end), "ecus": ecus.to_json(silent)})
}

/// Build the profile for a capture's text: the raw-CAN kind starts with `# hondascan session`.
pub fn profile(text: &str, vin: &str) -> Value {
    if text.lines().next().is_some_and(|l| l.starts_with("# hondascan session")) {
        build_raw(text, vin, "J2534 recording, 2015 Accord on the GD101 (VIN masked)")
    } else {
        build(text, vin)
    }
}

pub fn run(args: &[String], out: &mut dyn std::io::Write) -> Result<(), String> {
    use std::io::Write as _;
    let mut vin = "1HGCR2F3XFA000000".to_string();
    let mut pos = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--vin" {
            vin = it.next().ok_or("--vin needs a value")?.clone();
        } else if let Some(v) = a.strip_prefix("--vin=") {
            vin = v.to_string();
        } else if a.starts_with("--") {
            return Err(format!("unknown option {a}"));
        } else {
            pos.push(a);
        }
    }
    let [capture, outp] = pos[..] else { return Err("usage: profile-from-capture FRAMES.LOG OUT.json.gz [--vin VIN]".into()) };
    let text = String::from_utf8_lossy(&std::fs::read(capture).map_err(|e| format!("{capture}: {e}"))?).into_owned();
    let prof = profile(&text, &vin);
    let file = std::fs::File::create(outp).map_err(|e| format!("{outp}: {e}"))?;
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    gz.write_all(serde_json::to_string(&prof).unwrap().as_bytes()).and_then(|_| gz.finish().map(|_| ())).map_err(|e| format!("{outp}: {e}"))?;
    for (addr, e) in prof["ecus"].as_object().unwrap() {
        let replies = e["replies"].as_object().unwrap();
        let n: usize = replies.values().map(|v| v.as_array().unwrap().len()).sum();
        let _ = writeln!(out, "0x{addr}: {} requests answered ({n} distinct answers), {} silent", replies.len(), e["silent"].as_array().unwrap().len());
    }
    let _ = writeln!(out, "{:?} s -> {outp}", prof["seconds"].as_f64().unwrap());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elm_capture_keeps_distinct_answers_and_masks_the_vin() {
        let text = "(0.000) can0 x\n# ELM> 22F190\n# ELM< 18DAF10E100A62F190313233\n# ELM< 18DAF10E2134353637000000\n\
                    (1.5004) can0 x\n# ELM> 22F187\n# ELM< NO DATA\n(2.0) can0 x\n# ELM> 22F190\n";
        let p = build(text, "1HGCR2F3XFA000000");
        let e = &p["ecus"]["0E"];
        assert_eq!(e["replies"]["22F190"][0][1], "62F190".to_string() + &hex_upper(b"1HGCR2F3XFA000000"));
        assert_eq!(e["silent"], json!(["22F187"]));
        assert_eq!(p["seconds"], json!(2.0));
    }

    #[test]
    fn raw_capture_pairs_requests_with_answers_and_skips_pending() {
        let text = "# hondascan session\n(1.000) can0 18DA0EF1#0322F18700 ; TX\n(1.010) can0 18DAF10E#037F2278 ; RX\n\
                    (1.020) can0 18DAF10E#0462F18705 ; RX\n(1.030) can0 18DA0EF1#0322F18800 ; TX\n(2.000) can0 18DA0EF1#0322F18700 ; TX\n\
                    (2.100) can0 18DAF10E#0462F18706 ; RX\n(3.000) can0 18DA0EF1#0322F18700 ; TX\n(3.100) can0 18DAF10E#0462F18707 ; RX\n";
        let p = profile(text, "X");
        let e = &p["ecus"]["0E"];
        assert_eq!(e["replies"]["22F187"], json!([[0.02, "62F18705"], [1.1, "62F18706"], [2.1, "62F18707"]]));
        assert_eq!(e["silent"], json!(["22F188"]));
        assert_eq!(p["seconds"], json!(2.1));
    }

    #[test]
    fn multiframe_reassembly() {
        let mut r = Reassembly::default();
        assert_eq!(r.feed(&[0x10, 0x09, 1, 2, 3, 4, 5, 6]), None);
        assert_eq!(r.feed(&[0x21, 7, 8, 9, 0, 0, 0, 0]), Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9]));
    }
}
