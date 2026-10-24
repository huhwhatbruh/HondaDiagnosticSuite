//! The desktop app's backend: JSON lines on stdin and stdout (rpc.rs).
use std::io::{BufReader, Write};
use std::sync::{Arc, Mutex};

fn main() {
    let out: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(std::io::stdout()));
    if let Err(e) = hondascan::rpc::serve(Box::new(BufReader::new(std::io::stdin())), out, None, None) {
        eprintln!("hondascan-rpc: {e}");
        std::process::exit(2);
    }
}
