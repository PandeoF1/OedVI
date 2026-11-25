//! Child processes with their output streamed line by line, and stopped with
//! all their descendants (cargo starts rustc, the game may start helpers).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;

use anyhow::{Context, bail};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
    /// Messages of the launcher itself.
    Info,
}

pub type LogSink = Arc<dyn Fn(Stream, &str) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl Cmd {
    pub fn new(argv: &[String], cwd: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let (program, args) = argv.split_first().context("empty command")?;
        Ok(Self { program: program.clone(), args: args.to_vec(), cwd: cwd.into(), env: BTreeMap::new() })
    }

    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|a| if a.contains(' ') || a.is_empty() { format!("\"{a}\"") } else { a.clone() })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn command(&self) -> anyhow::Result<Command> {
        // Look the program up on PATH (and PATHEXT on Windows) ourselves, so
        // a missing tool gives a clear message.
        let program = if self.program.contains(['/', '\\']) {
            let p = PathBuf::from(&self.program);
            let p = if p.is_relative() { self.cwd.join(p) } else { p };
            // `exists` cannot see Windows app execution aliases (winget.exe);
            // `symlink_metadata` does not follow them.
            if !p.exists() && std::fs::symlink_metadata(&p).is_err() {
                bail!("{} not found (not built yet?)", p.display());
            }
            p
        } else {
            crate::prereq::which(&self.program).with_context(|| format!("`{}` not found on PATH", self.program))?
        };
        let mut c = Command::new(program);
        if let Some(path) = crate::prereq::current_path() {
            c.env("PATH", path);
        }
        c.args(&self.args)
            .current_dir(&self.cwd)
            .envs(&self.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(unix)]
        c.process_group(0);
        Ok(c)
    }
}

/// A started process whose output goes to a sink.
pub struct Running {
    pub child: Child,
    pub pid: u32,
    readers: Vec<tokio::task::JoinHandle<()>>,
}

pub fn spawn(cmd: &Cmd, sink: LogSink) -> anyhow::Result<Running> {
    if !cmd.cwd.is_dir() {
        bail!("working folder {} not found", cmd.cwd.display());
    }
    sink(Stream::Info, &format!("$ {}", cmd.display()));
    let mut child = cmd.command()?.spawn().with_context(|| format!("lancement de {}", cmd.program))?;
    let pid = child.id().unwrap_or(0);
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(tokio::spawn(pump(out, Stream::Stdout, sink.clone())));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(tokio::spawn(pump(err, Stream::Stderr, sink.clone())));
    }
    Ok(Running { child, pid, readers })
}

impl Running {
    /// Waits for the exit and for the last lines of output.
    pub async fn wait(mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        for r in self.readers.drain(..) {
            let _ = r.await;
        }
        Ok(status)
    }
}

async fn pump(reader: impl AsyncRead + Unpin, stream: Stream, sink: LogSink) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                // Progress bars rewrite their line with `\r`: keep the last state.
                let line = line.trim_end_matches(['\n', '\r']);
                let line = line.rsplit('\r').next().unwrap_or(line);
                sink(stream, &crate::metrics::strip_ansi(line));
            }
        }
    }
}

/// Runs to completion; stops the process tree when `cancel` fires.
pub async fn run(cmd: &Cmd, sink: LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    run_notify(cmd, sink, cancel, |_| {}).await
}

/// Like `run`, telling `on_start` the process id once it started.
pub async fn run_notify(
    cmd: &Cmd,
    sink: LogSink,
    cancel: &CancellationToken,
    on_start: impl FnOnce(u32),
) -> anyhow::Result<()> {
    let running = spawn(cmd, sink.clone())?;
    let pid = running.pid;
    on_start(pid);
    let wait = running.wait();
    tokio::pin!(wait);
    let status = tokio::select! {
        s = &mut wait => s?,
        _ = cancel.cancelled() => {
            kill_tree(pid);
            let _ = wait.await;
            bail!("cancelled");
        }
    };
    if !status.success() {
        bail!("`{}` failed ({})", cmd.program, describe(status));
    }
    Ok(())
}

pub fn describe(status: ExitStatus) -> String {
    match status.code() {
        Some(c) => format!("code {c}"),
        None => "killed by a signal".into(),
    }
}

/// Is process `pid` running, and (on Windows) is it `image`, e.g. `winget.exe`?
/// The image check keeps a recycled process id from looking like ours.
pub fn is_alive(pid: u32, image: &str) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .creation_flags(0x0800_0000)
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout)
                .to_lowercase()
                .contains(&format!("\"{}\",\"{pid}\"", image.to_lowercase())),
            Err(_) => false,
        }
    }
    #[cfg(unix)]
    {
        let _ = image;
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

/// Stops a process and everything it started.
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Twice: a child started while the first pass ran escapes it.
        for _ in 0..2 {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(unix)]
    unsafe {
        // The child leads its own process group (see `Cmd::command`).
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
