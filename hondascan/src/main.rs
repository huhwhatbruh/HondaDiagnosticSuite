//! The `hondascan` command.
use hondascan::cli::{main as cli_main, Io, INTERRUPTED};
use std::io::IsTerminal;
use std::sync::atomic::Ordering;

extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

fn main() {
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as usize);
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
    let mut input = std::io::BufReader::new(std::io::stdin());
    let tty = std::io::stderr().is_terminal();
    let code = cli_main(&argv, &mut Io { out: &mut out, err: &mut err, input: &mut input, tty });
    std::process::exit(code);
}
