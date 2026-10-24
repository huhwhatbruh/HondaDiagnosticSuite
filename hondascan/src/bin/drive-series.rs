//! `drive-series FRAMES.LOG [--system CANFI] [--address 0E] [SYMBOL ...]`: a recording's data-list replies as a time series.
use hondascan::db::Database;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut frames, mut system, mut address, mut symbols) = (None, "CANFI".to_string(), 0x0Eu32, vec![]);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--system" => {
                i += 1;
                system = args.get(i).cloned().unwrap_or_default();
            }
            "--address" => {
                i += 1;
                address = u32::from_str_radix(args.get(i).map_or("0E", String::as_str).trim_start_matches("0x"), 16).unwrap_or(0x0E);
            }
            a if frames.is_none() => frames = Some(a.to_string()),
            a => symbols.push(a.to_string()),
        }
        i += 1;
    }
    let Some(frames) = frames else {
        eprintln!("usage: drive-series FRAMES.LOG [--system CANFI] [--address 0E] [SYMBOL ...]");
        std::process::exit(2);
    };
    let db = match Database::open(None) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    match hondascan::probes::drive_series::series(&frames, db, &system, address, &symbols) {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("error: {frames}: {e}");
            std::process::exit(2);
        }
    }
}
