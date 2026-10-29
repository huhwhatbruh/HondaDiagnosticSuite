pub mod actservice;
pub mod actuator;
pub mod actuatorrun;
pub mod alerts;
pub mod audit;
pub mod backup;
#[cfg(target_os = "linux")]
pub mod ble;
#[cfg(unix)]
pub mod bonjour;
pub mod big;
pub mod cli;
pub mod cvtsim;
pub mod db;
pub mod dbcrypt;
pub mod diagh;
pub mod diagh_dtc;
pub mod handback;
pub mod history;
pub mod honda;
pub mod ident;
pub mod journal;
pub mod kwp;
pub mod livedata;
pub mod localtime;
pub mod maintenance;
pub mod mapbaseline;
pub mod maps;
pub mod memory;
pub mod memservice;
pub mod monitor;
pub mod obd9141;
pub mod ramwatch;
pub mod phone;
pub mod probes;
pub mod recorder;
pub mod report;
pub mod rpc;
pub mod seedkey;
pub mod selfcheck;
pub mod selftests;
pub mod service;
pub mod signalhelp;
pub mod sim;
pub mod simcapture;
pub mod simdrive;
pub mod store;
pub mod transport;
pub mod trip;
pub mod uds;
pub mod value;
pub mod write;
pub mod writerun;
pub mod writeservice;

/// True in the public release (`package.rs` builds with `--cfg hondascan_packaged`): that build only reads. The UDS and
/// KWP clients refuse every service that changes the car (clear codes, IO control, routines) and SecurityAccess, so the
/// CLI, the backend and the app cannot send them whatever they are asked; the public docs call the release read-only
/// and this is what makes that true. A checkout build is unaffected.
pub const READ_ONLY_RELEASE: bool = cfg!(hondascan_packaged);

/// The refusal text both clients give for a service the release does not send.
pub const NOT_IN_RELEASE: &str = "not available in this release: it only reads";

/// A data file the program needs (the database, the GD101 driver source, the recorded-car profiles): the one packaged next to the
/// executable if there is one (hondascan-gui/src/bin/package.rs lays it out so), else the copy in this crate's source tree.
/// The source-tree fallback never fires in a real package (everything ships beside the executable), but it would
/// otherwise bake this machine's absolute build path into the shipped binary as a string literal — `env!()` is a
/// compile-time literal, so `remap-path-prefix` (which only rewrites compiler-diagnostic paths) does not touch it.
/// `package.rs` builds with `--cfg hondascan_packaged` to drop it instead.
pub fn resource(name: &str) -> std::path::PathBuf {
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.exists());
    beside.unwrap_or_else(|| dev_source_dir().join(name))
}

#[cfg(hondascan_packaged)]
fn dev_source_dir() -> std::path::PathBuf {
    std::path::PathBuf::new() // never reached in a packaged build; see resource() above
}

#[cfg(not(hondascan_packaged))]
fn dev_source_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).into()
}
