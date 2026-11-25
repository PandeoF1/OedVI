//! Running git, build and game tasks, one per builder at a time, with their
//! output kept in memory (for the UI), written to `<root>/logs/` and sent to
//! the UI as it comes.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};

use oedvi_client::process::{LogSink, Stream};
use oedvi_common::BuilderDef;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

const MAX_LINES: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    /// Installing tools.
    Setup,
    Git,
    Build,
    Run,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub kind: TaskKind,
    pub action: String,
    pub started_at: u64,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub stream: Stream,
    pub text: String,
    /// Unix time, milliseconds.
    pub t: u64,
}

/// Lines sent together: a game printing thousands per second would
/// otherwise drown the UI in events.
#[derive(Clone, Serialize)]
struct LogBatch<'a> {
    id: &'a str,
    lines: &'a [LogLine],
}

/// How long lines wait to be sent as one batch.
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

#[derive(Clone, Serialize)]
struct EndEvent {
    id: String,
    kind: TaskKind,
    action: String,
    ok: bool,
    message: String,
}

struct Entry {
    info: TaskInfo,
    checkout: String,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct Tasks {
    running: Mutex<HashMap<String, Entry>>,
    logs: Arc<Mutex<HashMap<String, VecDeque<LogLine>>>>,
    /// Lines not yet sent to the UI, per builder.
    pending: Arc<Mutex<HashMap<String, Vec<LogLine>>>>,
}

fn flush(app: &AppHandle, pending: &Mutex<HashMap<String, Vec<LogLine>>>, id: &str) {
    let lines = pending.lock().unwrap().remove(id).unwrap_or_default();
    if !lines.is_empty() {
        let _ = app.emit("task-logs", LogBatch { id, lines: &lines });
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Tasks {
    pub fn start(
        &self,
        app: &AppHandle,
        def: &BuilderDef,
        kind: TaskKind,
        action: &str,
    ) -> Result<(CancellationToken, LogSink), String> {
        let mut running = self.running.lock().unwrap();
        if let Some(e) = running.get(&def.id) {
            return Err(format!("{} is already busy ({})", def.name, e.info.action));
        }
        let checkout = def.checkout_key().to_string();
        for (other, e) in running.iter() {
            if e.checkout != checkout {
                continue;
            }
            let clash = match (kind, e.info.kind) {
                (TaskKind::Run, TaskKind::Run) => false,
                (TaskKind::Run, _) | (_, TaskKind::Run) => {
                    // The game's executable is locked while it runs.
                    kind == TaskKind::Build || e.info.kind == TaskKind::Build
                }
                _ => true,
            };
            if clash {
                return Err(format!("the clone “{checkout}” is used by {other} ({})", e.info.action));
            }
        }
        let cancel = CancellationToken::new();
        running.insert(
            def.id.clone(),
            Entry {
                info: TaskInfo { kind, action: action.to_string(), started_at: now_ms(), pid: None },
                checkout,
                cancel: cancel.clone(),
            },
        );
        drop(running);

        // Log file: <root>/logs/<builder>/<time>-<action>.log
        let file = {
            let state = app.state::<crate::AppState>();
            let dir = state.settings().layout().logs.join(&def.id);
            std::fs::create_dir_all(&dir).ok();
            let name = format!("{}-{action}.log", now_ms() / 1000);
            std::fs::File::create(dir.join(name)).ok().map(Mutex::new)
        };
        let _ = app.emit(
            "task-start",
            EndEvent { id: def.id.clone(), kind, action: action.to_string(), ok: true, message: String::new() },
        );
        let id = def.id.clone();
        let logs = self.logs.clone();
        let pending = self.pending.clone();
        let app = app.clone();
        let sink: LogSink = Arc::new(move |stream, text: &str| {
            let line = LogLine { stream, text: text.to_string(), t: now_ms() };
            if let Some(f) = &file {
                let _ = writeln!(f.lock().unwrap(), "{text}");
            }
            {
                let mut logs = logs.lock().unwrap();
                let buf = logs.entry(id.clone()).or_default();
                if buf.len() >= MAX_LINES {
                    buf.pop_front();
                }
                buf.push_back(line.clone());
            }
            // The first line of a batch schedules its sending.
            let first = {
                let mut p = pending.lock().unwrap();
                let queue = p.entry(id.clone()).or_default();
                queue.push(line);
                queue.len() == 1
            };
            if first {
                let (app, pending, id) = (app.clone(), pending.clone(), id.clone());
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(FLUSH_EVERY).await;
                    flush(&app, &pending, &id);
                });
            }
        });
        sink(Stream::Info, &format!("── {action} · {} ──", def.name));
        Ok((cancel, sink))
    }

    pub fn set_pid(&self, id: &str, pid: u32) {
        if let Some(e) = self.running.lock().unwrap().get_mut(id) {
            e.info.pid = Some(pid);
        }
    }

    pub fn finish(&self, app: &AppHandle, id: &str, result: anyhow::Result<String>) {
        let entry = self.running.lock().unwrap().remove(id);
        let Some(entry) = entry else { return };
        let (ok, message) = match result {
            Ok(m) => (true, m),
            Err(e) if entry.cancel.is_cancelled() => (false, format!("interrompu ({e:#})")),
            Err(e) => (false, format!("{e:#}")),
        };
        let line =
            LogLine {
                stream: Stream::Info, text: format!("{} {message}", if ok { "✔" } else { "✘" }), t: now_ms()
            };
        // Earlier lines first, so the order is kept.
        flush(app, &self.pending, id);
        let _ = app.emit("task-logs", LogBatch { id, lines: std::slice::from_ref(&line) });
        self.logs.lock().unwrap().entry(id.to_string()).or_default().push_back(line);
        let _ = app.emit(
            "task-end",
            EndEvent { id: id.to_string(), kind: entry.info.kind, action: entry.info.action, ok, message },
        );
    }

    pub fn current(&self, id: &str) -> Option<TaskInfo> {
        self.running.lock().unwrap().get(id).map(|e| e.info.clone())
    }

    pub fn any_running(&self, kind: TaskKind) -> bool {
        self.running.lock().unwrap().values().any(|e| e.info.kind == kind)
    }

    pub fn cancel(&self, id: &str) {
        if let Some(e) = self.running.lock().unwrap().get(id) {
            e.cancel.cancel();
        }
    }

    pub fn cancel_all(&self, kinds: &[TaskKind]) {
        for e in self.running.lock().unwrap().values() {
            if kinds.contains(&e.info.kind) {
                e.cancel.cancel();
            }
        }
    }

    pub fn logs(&self, id: &str) -> Vec<LogLine> {
        self.logs.lock().unwrap().get(id).map(|b| b.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn clear_logs(&self, id: &str) {
        self.logs.lock().unwrap().remove(id);
    }
}
