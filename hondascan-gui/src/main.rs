//! hondascan, the Qt / QML front end: the logic is Rust (this crate), the screens are QML (`qml/`). It speaks JSON lines to
//! the backend `hondascan-rpc` (a binary of `../hondascan`), so what it may send is decided by the backend's allowlist.
// a window, not a console program: on Windows no console opens behind the app
#![cfg_attr(windows, windows_subsystem = "windows")]

mod actuator;
mod adapters;
mod api;
mod backend;
mod bundled;
mod car;
mod condition;
mod baseline;
mod drive;
mod format;
mod glance;
mod handback;
mod history;
mod intro;
mod live;
mod maintenance;
mod maps;
mod model;
mod names;
mod ranges;
mod reference;
mod scannote;
mod selfcheck;
mod selftests;
mod settings;
mod systems;
mod sysview;
#[cfg(test)]
mod text_audit;
mod vehicle;
mod writes;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

/// The windowing system to ask Qt for, or `None` to leave it alone. Qt picks the session's own system by itself (Wayland on a
/// Wayland session, X11 on X11, Windows and macOS natively); the one thing that defeats that is a desktop-wide `QT_QPA_PLATFORM`
/// that is only a preference list of `xcb` and `wayland` (e.g. `xcb;wayland`, which runs a Wayland session through XWayland).
/// On a Wayland session such a list is reordered to `wayland;xcb`. A single name (`xcb`, `wayland`) or any other plugin
/// (`offscreen` for the headless tests, `eglfs`, `minimal`) is a choice the person made and stays. The graphics API is Qt's
/// per-platform default (Direct3D 11 on Windows, OpenGL on Linux: as fast as Vulkan and lighter, README "Graphics"), and Metal on macOS, set in `main`.
#[cfg(any(target_os = "linux", test))]
fn session_platform(var: Option<&str>, wayland_session: bool) -> Option<&'static str> {
    let names: Vec<&str> = var?.split([';', ':']).map(str::trim).filter(|n| !n.is_empty()).collect();
    let is_preference_list = names.len() > 1 && names.iter().all(|n| matches!(*n, "xcb" | "wayland"));
    (wayland_session && is_preference_list && names.first() != Some(&"wayland")).then_some("wayland;xcb")
}

fn main() {
    bundled::relaunch_on_bundled_qt(); // before any Qt code: a packaged install runs on its own Qt
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if let Some(p) = session_platform(std::env::var("QT_QPA_PLATFORM").ok().as_deref(), wayland) {
            std::env::set_var("QT_QPA_PLATFORM", p);
        }
    }
    // macOS: Metal, explicitly (Qt's own default there; OpenGL is deprecated by Apple). A value the person set wins.
    #[cfg(target_os = "macos")]
    if std::env::var_os("QSG_RHI_BACKEND").is_none() {
        std::env::set_var("QSG_RHI_BACKEND", "metal");
    }
    // Greyscale glyphs: Qt takes RGB subpixel antialiasing from the desktop (GNOME's XSettings) even where the desktop itself draws
    // greyscale, and the coloured fringes read as blur on the black panes. A value the person set wins.
    if std::env::var_os("QT_SUBPIXEL_AA_TYPE").is_none() {
        std::env::set_var("QT_SUBPIXEL_AA_TYPE", "NONE");
    }
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/hondascan/app/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}

#[cfg(test)]
mod platform_tests {
    use super::session_platform;

    #[test]
    fn a_generic_preference_list_follows_the_session() {
        assert_eq!(session_platform(Some("xcb;wayland"), true), Some("wayland;xcb")); // XWayland by accident -> native
        assert_eq!(session_platform(Some("xcb:wayland"), true), Some("wayland;xcb"));
        assert_eq!(session_platform(Some("xcb;wayland"), false), None); // X11 session: as set
        assert_eq!(session_platform(Some("wayland;xcb"), true), None); // already right
    }

    #[test]
    fn an_explicit_choice_stays() {
        for v in ["xcb", "wayland", "offscreen", "minimal", "eglfs", "offscreen;xcb", "xcb;eglfs", ""] {
            assert_eq!(session_platform(Some(v), true), None, "{v}");
        }
        assert_eq!(session_platform(None, true), None); // unset: Qt already picks the session's own
    }
}
