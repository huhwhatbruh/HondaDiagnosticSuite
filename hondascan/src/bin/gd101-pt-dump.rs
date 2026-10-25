//! `gd101-pt-dump [--ecu 0E] [--pause 8]`: copy GODIAG_PT32.dll's unpacked image out of a Wine process (see probes/gd101_pt_dump.rs).
#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match hondascan::probes::gd101_pt_dump::run(&args, &mut std::io::stdout()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gd101-pt-dump: Linux only (it uses termios2 and Wine)");
    std::process::exit(2);
}
