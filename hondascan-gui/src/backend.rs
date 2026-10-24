//! The hondascan backend (`hondascan-rpc`, hondascan/src/rpc.rs) as a child process speaking JSON lines.
//!
//! `call` never blocks: it writes the request and returns a receiver for the reply. A reader thread matches replies to
//! requests by id, forwards id-less lines as `Event::Backend` and wakes the UI. If the process exits, every call still
//! waiting fails with the tail of its stderr and the next call starts a fresh process.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Reply = Result<Value, String>;
type Pending = Arc<Mutex<HashMap<u64, Sender<Reply>>>>;

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
    events: Sender<Event>,
    ctx: eframe::egui::Context,
    generation: Arc<AtomicU64>, // which process is current: a process that was replaced must not announce its exit
}

impl Backend {
    pub fn new(ctx: eframe::egui::Context) -> (Arc<Self>, Receiver<Event>) {
        let (events, rx) = channel();
        (Arc::new(Backend { proc: Mutex::new(None), next_id: AtomicU64::new(1), events, ctx, generation: Arc::default() }), rx)
    }

    pub fn call(&self, method: &str, params: Value) -> Receiver<Reply> {
        let (tx, rx) = channel();
        let mut guard = self.proc.lock().unwrap();
        if guard.as_mut().is_none_or(|p| p.child.try_wait().ok().flatten().is_some()) {
            match self.spawn() {
                Ok(p) => *guard = Some(p),
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return rx;
                }
            }
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let p = guard.as_mut().unwrap();
        p.pending.lock().unwrap().insert(id, tx.clone());
        let line = json!({ "id": id, "method": method, "params": params }).to_string() + "\n";
        if let Err(e) = p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()) {
            p.pending.lock().unwrap().remove(&id);
            let _ = tx.send(Err(format!("backend not reachable: {e}")));
        }
        rx
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
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
        let (events, ctx) = (self.events.clone(), self.ctx.clone());
        let (generation, mine) = (self.generation.clone(), self.generation.fetch_add(1, Ordering::SeqCst) + 1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(ev) = msg.get("event").and_then(Value::as_str) {
                    let _ = events.send(Event::Backend { event: ev.to_string(), data: msg["data"].clone() });
                } else if let Some(tx) = msg["id"].as_u64().and_then(|id| waiting.lock().unwrap().remove(&id)) {
                    let _ = tx.send(match msg.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("backend error").to_string()),
                        None => Ok(msg["result"].clone()),
                    });
                }
                ctx.request_repaint();
            }
            // EOF: the process is gone. Fail what is still waiting, with its last words.
            std::thread::sleep(std::time::Duration::from_millis(50));
            let why = Vec::from(tail.lock().unwrap().clone()).join("\n");
            let why = if why.is_empty() { "backend exited".to_string() } else { why };
            for (_, tx) in waiting.lock().unwrap().drain() {
                let _ = tx.send(Err(why.clone()));
            }
            if generation.load(Ordering::SeqCst) == mine { let _ = events.send(Event::Exit(why)); }
            ctx.request_repaint();
        });
        Ok(Proc { child, stdin, pending })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn get(rx: &Receiver<Reply>) -> Reply { rx.recv_timeout(Duration::from_secs(10)).expect("a reply") }

    #[cfg(unix)]
    #[test]
    fn replies_are_matched_by_id_and_a_dead_backend_fails_its_calls() {
        let dir = std::env::temp_dir().join(format!("hondascan-gui-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("hondascan-rpc");
        std::fs::write(&fake, FAKE).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("HONDASCAN_RPC", &fake);
        let (be, events) = Backend::new(eframe::egui::Context::default());

        let (a, b) = (be.call("hold", json!({})), be.call("hold", json!({})));
        let c = be.call("release", json!({}));
        // ids 1, 2, 3, answered 3, 2, 1: each receiver gets its own
        assert_eq!((get(&a), get(&b), get(&c)), (Ok(json!(1)), Ok(json!(2)), Ok(json!(3))));
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Backend { event, data } => assert_eq!((event.as_str(), data), ("note", json!({"held": 2}))),
            Event::Exit(w) => panic!("exit: {w}"),
        }
        assert_eq!(get(&be.call("fail", json!({}))), Err("boom".into()));

        let waiting = be.call("hold", json!({}));
        let _ = be.call("die", json!({}));
        let e = get(&waiting).unwrap_err();
        assert!(e.contains("last words"), "{e}");
        assert!(matches!(events.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Exit(w) if w.contains("last words")));

        // the next call starts a fresh process
        let (a, c) = (be.call("hold", json!({})), be.call("release", json!({})));
        assert!(get(&a).is_ok() && get(&c).is_ok());
        be.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }
}
