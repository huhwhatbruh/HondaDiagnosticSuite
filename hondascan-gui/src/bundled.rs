//! A packaged Linux app carries its Qt in `lib/` next to the executable (package.rs `bundle_qt`). The executable's RPATH finds it, but Qt's own
//! QML plugins have a RUNPATH, which makes the loader ignore that RPATH for what they load: they would pick up the system's Qt, and two Qts in one
//! process is a crash waiting for a machine with a different version. `LD_LIBRARY_PATH` beats both, so the app starts once more with it set (before
//! any Qt code runs) and hands the original value back to the backend it spawns, which has no use for the bundled libraries.
use std::path::Path;

/// Remembers the caller's own LD_LIBRARY_PATH across the relaunch (empty when there was none).
const ORIGINAL: &str = "HONDASCAN_ORIG_LD_LIBRARY_PATH";

/// The LD_LIBRARY_PATH to relaunch with, or None when this is not a bundled install (no `lib/libQt6Core.so.6` next to the executable) or it is
/// already set up: the bundled directory goes first, the caller's own entries after it.
#[cfg(any(target_os = "linux", test))]
pub fn bundled_path(exe_dir: &Path, current: Option<&str>) -> Option<String> {
    if !exe_dir.join("lib/libQt6Core.so.6").is_file() { return None; }
    let lib = exe_dir.join("lib").to_string_lossy().into_owned();
    match current {
        Some(c) if c.split(':').next() == Some(lib.as_str()) => None,
        Some(c) if !c.is_empty() => Some(format!("{lib}:{c}")),
        _ => Some(lib),
    }
}

/// Start the program again with the bundled libraries first, once (a no-op anywhere but a packaged Linux install). Returns only if there was
/// nothing to do or the relaunch failed: the app then runs as it is.
#[cfg(target_os = "linux")]
pub fn relaunch_on_bundled_qt() {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let current = std::env::var("LD_LIBRARY_PATH").ok();
    let Some(path) = bundled_path(dir, current.as_deref()) else { return };
    let err = std::process::Command::new(&exe).args(std::env::args_os().skip(1))
        .env("LD_LIBRARY_PATH", path).env(ORIGINAL, current.unwrap_or_default()).exec();
    eprintln!("could not restart on the bundled Qt: {err}");
}

#[cfg(not(target_os = "linux"))]
pub fn relaunch_on_bundled_qt() {}

/// Give a child process (the backend) the library path the app was started with.
pub fn restore_for_child(cmd: &mut std::process::Command) {
    if let Ok(original) = std::env::var(ORIGINAL) {
        if original.is_empty() { cmd.env_remove("LD_LIBRARY_PATH"); } else { cmd.env("LD_LIBRARY_PATH", original); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(with_qt: bool) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hondascan-bundled-{}-{with_qt}", std::process::id()));
        std::fs::create_dir_all(d.join("lib")).unwrap();
        if with_qt { std::fs::write(d.join("lib/libQt6Core.so.6"), b"x").unwrap(); }
        d
    }

    #[test]
    fn a_checkout_or_a_system_install_is_left_alone() {
        let d = install(false);
        assert_eq!(bundled_path(&d, None), None, "no bundled Qt next to the executable");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_bundled_install_puts_its_lib_first_and_keeps_the_callers_entries() {
        let d = install(true);
        let lib = d.join("lib").to_string_lossy().into_owned();
        assert_eq!(bundled_path(&d, None), Some(lib.clone()));
        assert_eq!(bundled_path(&d, Some("")), Some(lib.clone()));
        assert_eq!(bundled_path(&d, Some("/opt/x:/opt/y")), Some(format!("{lib}:/opt/x:/opt/y")));
        assert_eq!(bundled_path(&d, Some(&format!("{lib}:/opt/x"))), None, "already set up: the relaunch happens once");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_backend_gets_the_callers_own_library_path_back() {
        std::env::set_var(ORIGINAL, "/opt/x");
        let mut cmd = std::process::Command::new("true");
        restore_for_child(&mut cmd);
        assert_eq!(cmd.get_envs().find(|(k, _)| *k == "LD_LIBRARY_PATH").and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned()), Some("/opt/x".into()));
        std::env::set_var(ORIGINAL, "");
        let mut cmd = std::process::Command::new("true");
        restore_for_child(&mut cmd);
        assert!(cmd.get_envs().any(|(k, v)| k == "LD_LIBRARY_PATH" && v.is_none()), "no path before means none for the backend");
        std::env::remove_var(ORIGINAL);
    }
}
