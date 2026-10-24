//! `abs-probe ADAPTER [--log FILE]`: what makes the ABS, EPS and TPMS answer on the DLC? Read-only (see probes/abs_probe.rs).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(spec) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: abs-probe ADAPTER [--log FILE]   (adapter spec as for `hondascan -a`)");
        std::process::exit(2);
    };
    let log = args.iter().position(|a| a == "--log").and_then(|i| args.get(i + 1)).map(String::as_str);
    match hondascan::probes::abs_probe::run(spec, log, &mut std::io::stdout()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}
