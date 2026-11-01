//! The hondascan backend (`hondascan-rpc`, hondascan/src/rpc.rs) as a child process speaking JSON lines.
//!
//! `call` never blocks: it writes the request and returns; the reader thread hands the reply to the handler given with
//! the call (matched by id) and anything without an id to the sink as an `Event`. If the process exits, every call still
//! waiting fails with the tail of its stderr and the next call starts a fresh process.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Reply = Result<Value, String>;
/// Runs on the reader thread: it must only forward the reply (to the Qt thread), not use it.
pub type Handler = Box<dyn FnOnce(Reply) + Send>;
type Pending = Arc<Mutex<HashMap<u64, Handler>>>;
/// Receives what the backend sends on its own, on the reader thread.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

pub enum Event {
    Backend { event: String, data: Value },
    Exit(String),
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    pending: Pending,
}

pub struct Backend {
    proc: Mutex<Option<Proc>>,
    next_id: AtomicU64,
    sink: Sink,
    generation: Arc<AtomicU64>, // which process is current: a process that was replaced must not announce its exit
}

impl Backend {
    pub fn new(sink: Sink) -> Arc<Self> {
        Arc::new(Backend { proc: Mutex::new(None), next_id: AtomicU64::new(1), sink, generation: Arc::default() })
    }

    pub fn call(&self, method: &str, params: Value, handler: Handler) {
        let mut guard = self.proc.lock().unwrap();
        if guard.as_mut().is_none_or(|p| p.child.try_wait().ok().flatten().is_some()) {
            match self.spawn() {
                Ok(p) => *guard = Some(p),
                Err(e) => return handler(Err(e)),
            }
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let p = guard.as_mut().unwrap();
        p.pending.lock().unwrap().insert(id, handler);
        let line = json!({ "id": id, "method": method, "params": params }).to_string() + "\n";
        if let Err(e) = p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()) {
            if let Some(h) = p.pending.lock().unwrap().remove(&id) { h(Err(format!("backend not reachable: {e}"))); }
        }
    }

    pub fn shutdown(&self) {
        if let Some(mut p) = self.proc.lock().unwrap().take() {
            drop(p.stdin); // EOF: rpc.serve closes the adapter and returns
            let deadline = Instant::now() + Duration::from_secs(3);
            while p.child.try_wait().ok().flatten().is_none() {
                if Instant::now() >= deadline {
                    let _ = p.child.kill();
                    let _ = p.child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn spawn(&self) -> Result<Proc, String> {
        let bin = backend_binary().ok_or("hondascan-rpc not found (build it with `cargo build` in hondascan/, or set HONDASCAN_RPC)")?;
        let (mut cmd, name) = (Command::new(&bin), bin.display().to_string());
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        crate::bundled::restore_for_child(&mut cmd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("cannot start {name}: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let t = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut t = t.lock().unwrap();
                t.push_back(line);
                if t.len() > 20 {
                    t.pop_front();
                }
            }
        });

        let pending: Pending = Arc::default();
        let waiting = pending.clone();
        let sink = self.sink.clone();
        let (generation, mine) = (self.generation.clone(), self.generation.fetch_add(1, Ordering::SeqCst) + 1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(ev) = msg.get("event").and_then(Value::as_str) {
                    sink(Event::Backend { event: ev.to_string(), data: msg["data"].clone() });
                } else if let Some(h) = msg["id"].as_u64().and_then(|id| waiting.lock().unwrap().remove(&id)) {
                    h(match msg.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("backend error").to_string()),
                        None => Ok(msg["result"].clone()),
                    });
                }
            }
            // EOF: the process is gone. Fail what is still waiting, with its last words.
            std::thread::sleep(Duration::from_millis(50));
            let why = Vec::from(tail.lock().unwrap().clone()).join("\n");
            let why = if why.is_empty() { "backend exited".to_string() } else { why };
            let all: Vec<Handler> = waiting.lock().unwrap().drain().map(|(_, h)| h).collect();
            for h in all { h(Err(why.clone())); }
            if generation.load(Ordering::SeqCst) == mine { sink(Event::Exit(why)); }
        });
        Ok(Proc { child, stdin, pending })
    }
}

impl Drop for Backend {
    /// The app is closing: let the backend close the adapter (and hand control back) before the process is gone.
    fn drop(&mut self) { self.shutdown(); }
}

/// The backend binary: `$HONDASCAN_RPC` if set, else the one bundled next to this executable, then (development builds only)
/// the one built in the source tree.
fn backend_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HONDASCAN_RPC").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let name = if cfg!(windows) { "hondascan-rpc.exe" } else { "hondascan-rpc" };
    let mut dirs: Vec<PathBuf> = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).into_iter().collect();
    if cfg!(debug_assertions) {
        let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../hondascan/target");
        dirs.extend([tree.join("debug"), tree.join("release")]);
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

// the stand-in backend is a sh script and the test sets a Unix file mode
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver};

    /// A stand-in for hondascan-rpc (a POSIX sh script; requests look like `{"id":1,"method":"hold","params":{}}`): holds replies
    /// back and sends them in reverse order, mixes in an event and a line that is not JSON, answers an error, and dies on request
    /// with something on stderr.
    const FAKE: &str = r#"#!/bin/sh
held=""
n=0
while read -r line; do
  id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
  m=$(printf '%s' "$line" | sed 's/.*"method":"\([a-z]*\)".*/\1/')
  case "$m" in
    hold) held="$id $held"; n=$((n + 1));;
    release)
      echo "{\"event\":\"note\",\"data\":{\"held\":$n}}"
      echo "not json"
      for i in $id $held; do echo "{\"id\":$i,\"result\":$i}"; done
      held=""; n=0;;
    fail) echo "{\"id\":$id,\"error\":{\"message\":\"boom\",\"type\":\"ValueError\"}}";;
    die) echo "last words" >&2; exit 3;;
  esac
done
"#;

    fn ask(be: &Backend, method: &str) -> Receiver<Reply> {
        let (tx, rx) = channel();
        be.call(method, json!({}), Box::new(move |r| { let _ = tx.send(r); }));
        rx
    }
    fn get(rx: &Receiver<Reply>) -> Reply { rx.recv_timeout(Duration::from_secs(10)).expect("a reply") }

    #[test]
    fn replies_are_matched_by_id_and_a_dead_backend_fails_its_calls() {
        let dir = std::env::temp_dir().join(format!("hondascan-gui-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("hondascan-rpc");
        std::fs::write(&fake, FAKE).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("HONDASCAN_RPC", &fake);
        let (etx, events) = channel();
        let etx = Mutex::new(etx);
        let be = Backend::new(Arc::new(move |e| { let _ = etx.lock().unwrap().send(e); }));

        let (a, b) = (ask(&be, "hold"), ask(&be, "hold"));
        let c = ask(&be, "release");
        // ids 1, 2, 3, answered 3, 2, 1: each receiver gets its own
        assert_eq!((get(&a), get(&b), get(&c)), (Ok(json!(1)), Ok(json!(2)), Ok(json!(3))));
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Backend { event, data } => assert_eq!((event.as_str(), data), ("note", json!({"held": 2}))),
            Event::Exit(w) => panic!("exit: {w}"),
        }
        assert_eq!(get(&ask(&be, "fail")), Err("boom".into()));

        let waiting = ask(&be, "hold");
        let _ = ask(&be, "die");
        let e = get(&waiting).unwrap_err();
        assert!(e.contains("last words"), "{e}");
        assert!(matches!(events.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Exit(w) if w.contains("last words")));

        // the next call starts a fresh process
        let (a, c) = (ask(&be, "hold"), ask(&be, "release"));
        assert!(get(&a).is_ok() && get(&c).is_ok());
        be.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }
}
