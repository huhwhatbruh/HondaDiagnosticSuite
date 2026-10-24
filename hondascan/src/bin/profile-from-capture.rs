//! `profile-from-capture FRAMES.LOG OUT.json.gz [--vin VIN]`: turn a recording into a simulator profile (see probes/profile_from_capture.rs).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = hondascan::probes::profile_from_capture::run(&args, &mut std::io::stdout()) {
        eprintln!("{e}");
        std::process::exit(2);
    }
}
