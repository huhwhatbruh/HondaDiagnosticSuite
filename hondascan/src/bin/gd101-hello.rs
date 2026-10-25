//! `gd101-hello [DEVICE]`: native GD101 handshake probe at 2534 baud (see probes/gd101_hello.rs).
#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = hondascan::probes::gd101_hello::run(&args, &mut std::io::stdout()) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gd101-hello: Linux only (it uses termios2 and Wine)");
    std::process::exit(2);
}
