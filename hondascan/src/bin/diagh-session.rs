//! `diagh-session PORT ROW`: HDS's own wake-up, one keep-awake and stop for a Honda DIAG-H row over a serial K-line cable
//! (see probes/diagh_session.rs).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = hondascan::probes::diagh_session::run(&args, &mut std::io::stdout()) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}
