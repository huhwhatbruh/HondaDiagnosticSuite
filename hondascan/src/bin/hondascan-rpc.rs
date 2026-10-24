//! The desktop app's backend: JSON lines on stdin and stdout (rpc.rs).
//! With HONDASCAN_PHONE=addr:port it also serves the read-only phone gateway (phone.rs).
use std::io::{BufReader, Write};
use std::sync::{Arc, Mutex};

fn main() {
    let out: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(std::io::stdout()));
    let result = hondascan::phone::from_env(Box::new(BufReader::new(std::io::stdin())), out).and_then(|(inp, out)| hondascan::rpc::serve(inp, out, None, None));
    if let Err(e) = result {
        eprintln!("hondascan-rpc: {e}");
        std::process::exit(2);
    }
}
