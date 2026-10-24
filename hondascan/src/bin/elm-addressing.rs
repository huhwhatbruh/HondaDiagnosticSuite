//! `elm-addressing [--port /dev/gd101] [--target 0E] [--log FILE] [--monitor SECONDS]`: which header style does an ELM clone accept?
//! Read-only (3E 00 / 3E 80 only); see probes/elm_addressing.rs.
/// A usage error: the usage line and the reason, then exit 2.
fn die(m: &str) -> ! {
    eprintln!("usage: elm-addressing [--port /dev/gd101] [--target 0E] [--log FILE] [--monitor SECONDS]\nerror: {m}");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let known = ["--port", "--target", "--log", "--monitor"];
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if known.contains(&a) {
            i += 2; // the flag and its value
        } else if a.starts_with('-') {
            die(&format!("unrecognized arguments: {a}"));
        } else {
            i += 1;
        }
    }
    let value = |name: &str, default: &str| -> String {
        match args.iter().position(|a| a == name) {
            Some(i) => args.get(i + 1).cloned().unwrap_or_else(|| die(&format!("argument {name}: expected one argument"))),
            None => default.to_string(),
        }
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let target_s = value("--target", "0E");
    let target = u32::from_str_radix(target_s.trim_start_matches("0x"), 16).unwrap_or_else(|_| die(&format!("argument --target: {target_s:?} is not a hexadecimal address")));
    let monitor_s = value("--monitor", "0");
    let monitor = monitor_s.parse::<f64>().unwrap_or_else(|_| die(&format!("argument --monitor: {monitor_s:?} is not a number of seconds")));
    match hondascan::probes::elm_addressing::run(&value("--port", "/dev/gd101"), target, &value("--log", &format!("{home}/hondascan-addressing.log")), monitor, &mut std::io::stdout()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}
