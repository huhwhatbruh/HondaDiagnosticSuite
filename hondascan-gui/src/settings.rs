//! What the app remembers between runs: the adapter choice, each system's pinned gauges, and whether the installation
//! check has been done. One small JSON file (`gui.json`) in the same data folder the backend records sessions in.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub adapter: String,
    pub custom: String,
    pub pins: HashMap<String, Vec<(u32, usize)>>,
    /// Unix seconds of the last installation check, 0 = never (the first run opens the check).
    pub self_check_at: f64,
    /// Whether that check found everything in order (backend version, settings folder writable).
    pub self_check_ok: bool,
}

pub struct Store { path: PathBuf, pub last: Settings }

impl Store {
    pub fn open(data_dir: &str) -> Store {
        let path = PathBuf::from(data_dir).join("gui.json");
        let last = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Store { path, last }
    }

    /// Write `s` if it differs from what is on disk. A failure to write is not worth interrupting anyone for.
    pub fn save(&mut self, s: &Settings) {
        if *s == self.last { return; }
        if let Ok(t) = serde_json::to_string_pretty(s) {
            if let Some(d) = self.path.parent() { let _ = std::fs::create_dir_all(d); }
            // written beside and renamed over: a crash mid-write cannot leave a truncated file (which would lose everything)
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, t).is_ok() { let _ = std::fs::rename(&tmp, &self.path); }
        }
        self.last = s.clone();
    }

    /// Whether the settings folder can be written at all: a probe file of its own is written and removed, so `gui.json`
    /// is never touched and a stale one is not mistaken for a writable folder.
    pub fn writable(&self) -> bool {
        let Some(dir) = self.path.parent() else { return false };
        if std::fs::create_dir_all(dir).is_err() { return false; }
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let probe = dir.join(format!(".gui-writable-{}-{unique}", std::process::id()));
        let wrote = std::fs::write(&probe, b"hondascan").is_ok();
        let removed = std::fs::remove_file(&probe).is_ok();
        wrote && removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf { std::env::temp_dir().join(format!("hondascan-gui-test-{name}-{}", std::process::id())) }

    #[test]
    fn saved_settings_come_back_and_unchanged_ones_are_not_rewritten() {
        let dir = temp("roundtrip");
        let d = dir.to_str().unwrap();
        let mut s = Store::open(d);
        assert!(s.last == Settings::default());
        let mut want = Settings { adapter: "gd101-j2534:/dev/gd101".into(), self_check_at: 1_760_000_000.5, self_check_ok: true, ..Default::default() };
        want.pins.insert("CANFI".into(), vec![(0x2610, 6), (0x2610, 11)]);
        s.save(&want);
        let back = Store::open(d).last;
        assert!(back == want);
        assert_eq!(back.self_check_at, 1_760_000_000.5);
        assert!(back.self_check_ok, "the check result survives a restart");
        let t0 = std::fs::metadata(dir.join("gui.json")).unwrap().modified().unwrap();
        s.save(&want);
        assert_eq!(std::fs::metadata(dir.join("gui.json")).unwrap().modified().unwrap(), t0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_file_is_ignored() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("gui.json"), "{ not json").unwrap();
        let st = Store::open(dir.to_str().unwrap()).last;
        assert!(st == Settings::default(), "0 means the check has never been done");
        assert_eq!((st.self_check_at, st.self_check_ok), (0.0, false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_folder_is_probed_without_touching_the_settings_file() {
        let dir = temp("writable");
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(dir.to_str().unwrap());
        assert!(s.writable(), "the folder is created and a probe written and removed");
        assert!(dir.is_dir());
        assert!(!dir.join("gui.json").exists(), "the probe never writes the settings file itself");
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(left.is_empty(), "the probe is removed: {left:?}");

        // a path whose parent is a file cannot be a writable folder
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let blocked = Store { path: file.join("gui.json"), last: Settings::default() };
        assert!(!blocked.writable());
        let _ = std::fs::remove_dir_all(dir);
    }
}
