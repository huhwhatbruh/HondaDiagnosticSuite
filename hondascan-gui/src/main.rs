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

fn main() {
    bundled::relaunch_on_bundled_qt(); // before any Qt code: a packaged install runs on its own Qt
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
