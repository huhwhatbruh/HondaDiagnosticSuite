//! The `hondascan` command.
use hondascan::cli::{main as cli_main, Io, INTERRUPTED};
use std::io::IsTerminal;
use std::sync::atomic::Ordering;

#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

#[cfg(windows)]
unsafe extern "system" fn on_ctrl_c(kind: u32) -> i32 {
    // CTRL_C_EVENT and CTRL_BREAK_EVENT only; closing the console window still ends the process
    if kind > 1 {
        return 0;
    }
    INTERRUPTED.store(true, Ordering::Relaxed);
    1
}

fn main() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as usize);
    }
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl_c), 1);
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
    let mut input = std::io::BufReader::new(std::io::stdin());
    let tty = std::io::stderr().is_terminal();
    let code = cli_main(&argv, &mut Io { out: &mut out, err: &mut err, input: &mut input, tty });
    std::process::exit(code);
}
