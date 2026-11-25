//! Tools a builder needs (git, Rust, Visual Studio Build Tools, Python…),
//! and installing the missing ones with winget (itself installed first when
//! Windows does not have it).
//!
//! Only what the builder at hand needs is checked, and only what is missing
//! is installed: a Python builder never pulls Visual Studio, and a tool that
//! is already there is left alone.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, bail};
use oedvi_common::BuilderDef;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::process::{self, Cmd, LogSink, Stream};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detect {
    /// Every program on PATH.
    OnPath(&'static [&'static str]),
    /// MSVC compiler and Windows SDK, found by vswhere.
    Msvc,
}

#[derive(Debug)]
struct Tool {
    id: &'static str,
    label: &'static str,
    detect: Detect,
    winget: &'static str,
    /// Extra winget arguments (component selection).
    winget_args: &'static [&'static str],
    /// The installer asks for administrator rights (Windows shows a prompt).
    admin: bool,
    /// Size order of magnitude, for the UI.
    size: &'static str,
}

const CATALOG: &[Tool] = &[
    Tool {
        id: "git",
        label: "Git",
        detect: Detect::OnPath(&["git"]),
        winget: "Git.Git",
        winget_args: &[],
        admin: false,
        size: "300 Mo",
    },
    Tool {
        id: "msvc",
        label: "Visual Studio Build Tools (C++)",
        detect: Detect::Msvc,
        winget: "Microsoft.VisualStudio.2022.BuildTools",
        winget_args: &[
            "--override",
            "--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended",
        ],
        admin: true,
        size: "6 Go",
    },
    Tool {
        id: "rust",
        label: "Rust (rustup)",
        detect: Detect::OnPath(&["cargo", "rustup"]),
        winget: "Rustlang.Rustup",
        winget_args: &[],
        admin: false,
        size: "1 Go",
    },
    Tool {
        id: "python",
        label: "Python 3.12",
        detect: Detect::OnPath(&["python"]),
        winget: "Python.Python.3.12",
        winget_args: &[],
        admin: false,
        size: "100 Mo",
    },
    Tool {
        id: "uv",
        label: "uv (Python)",
        detect: Detect::OnPath(&["uv"]),
        winget: "astral-sh.uv",
        winget_args: &[],
        admin: false,
        size: "40 Mo",
    },
    Tool {
        id: "node",
        label: "Node.js LTS",
        detect: Detect::OnPath(&["node", "npm"]),
        winget: "OpenJS.NodeJS.LTS",
        winget_args: &[],
        admin: false,
        size: "100 Mo",
    },
    Tool {
        id: "dotnet",
        label: ".NET SDK 8",
        detect: Detect::OnPath(&["dotnet"]),
        winget: "Microsoft.DotNet.SDK.8",
        winget_args: &[],
        admin: true,
        size: "800 Mo",
    },
    Tool {
        id: "cmake",
        label: "CMake",
        detect: Detect::OnPath(&["cmake"]),
        winget: "Kitware.CMake",
        winget_args: &[],
        admin: false,
        size: "100 Mo",
    },
    Tool {
        id: "ninja",
        label: "Ninja",
        detect: Detect::OnPath(&["ninja"]),
        winget: "Ninja-build.Ninja",
        winget_args: &[],
        admin: false,
        size: "1 Mo",
    },
];

fn tool(id: &str) -> &'static Tool {
    CATALOG.iter().find(|t| t.id == id).expect("catalog id")
}

/// Catalog id for a name used in `requires` (`cargo` means Rust, and so on).
fn known(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "git" => "git",
        "msvc" | "cl" | "link" | "vs" | "visualstudio" | "buildtools" => "msvc",
        "rust" | "cargo" | "rustup" | "rustc" => "rust",
        "python" | "python3" | "pip" => "python",
        "uv" => "uv",
        "node" | "nodejs" | "npm" | "npx" => "node",
        "dotnet" => "dotnet",
        "cmake" => "cmake",
        "ninja" => "ninja",
        _ => return None,
    })
}

/// What a builder needs, in install order (Build Tools before Rust).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    Known(&'static str),
    /// A program in `requires` outside the catalog: checked, not installed.
    Program(String),
}

pub fn needs(def: &BuilderDef) -> Vec<Need> {
    let windows = cfg!(windows);
    let mut ids: Vec<&'static str> = vec!["git"];
    match def.stack.as_str() {
        "rust" => {
            if windows {
                ids.push("msvc");
            }
            ids.push("rust");
        }
        "python" => ids.push("python"),
        "node" => ids.push("node"),
        "dotnet" => ids.push("dotnet"),
        "cmake" => {
            if windows {
                ids.push("msvc");
            }
            ids.push("cmake");
        }
        _ => {}
    }
    let mut others = Vec::new();
    for r in &def.requires {
        match known(r) {
            Some("msvc") if !windows => {}
            Some(id) => {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            None => {
                if !others.contains(r) {
                    others.push(r.clone());
                }
            }
        }
    }
    // Catalog order is install order.
    let mut out: Vec<Need> = CATALOG.iter().filter(|t| ids.contains(&t.id)).map(|t| Need::Known(t.id)).collect();
    out.extend(others.into_iter().map(Need::Program));
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub id: String,
    pub label: String,
    pub installed: bool,
    /// Where it was found.
    pub found: Option<String>,
    /// The launcher can install it (winget on Windows, which it sets up first
    /// when it is missing).
    pub installable: bool,
    pub admin: bool,
    pub size: Option<String>,
}

// ----------------------------------------------------------------- PATH

/// PATH used to find tools and given to every command the launcher starts.
/// Reloaded from the registry after an install, so new tools are seen
/// without restarting the launcher.
fn path_cell() -> &'static RwLock<Option<OsString>> {
    static CELL: OnceLock<RwLock<Option<OsString>>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(None))
}

pub fn current_path() -> Option<OsString> {
    path_cell().read().unwrap().clone().or_else(|| std::env::var_os("PATH"))
}

/// Finds a program with the launcher's PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let found = which::which_in(program, current_path(), cwd).ok()?;
    // Windows' "python.exe" in WindowsApps only opens the Microsoft Store.
    if found.to_string_lossy().to_ascii_lowercase().contains("\\windowsapps\\") {
        return None;
    }
    Some(found)
}

/// Rebuilds PATH from what installers wrote in the registry, keeping the
/// entries this process already had.
pub fn refresh_path() {
    #[cfg(windows)]
    {
        let mut parts: Vec<String> = Vec::new();
        for (root, key) in [
            (winreg::enums::HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
            (winreg::enums::HKEY_CURRENT_USER, "Environment"),
        ] {
            if let Ok(k) = winreg::RegKey::predef(root).open_subkey(key)
                && let Ok(v) = k.get_value::<String, _>("Path")
            {
                parts.extend(v.split(';').filter(|s| !s.is_empty()).map(expand_env));
            }
        }
        // rustup's default location, in case its PATH entry is not written yet.
        if let Some(home) = std::env::var_os("USERPROFILE") {
            parts.push(PathBuf::from(home).join(".cargo").join("bin").to_string_lossy().into_owned());
        }
        if let Some(old) = std::env::var_os("PATH") {
            parts.extend(old.to_string_lossy().split(';').filter(|s| !s.is_empty()).map(String::from));
        }
        let mut seen = std::collections::HashSet::new();
        parts.retain(|p| seen.insert(p.to_ascii_lowercase()));
        *path_cell().write().unwrap() = Some(parts.join(";").into());
    }
}

#[cfg(windows)]
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------- detection

fn vswhere() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramFiles(x86)").or_else(|| std::env::var_os("ProgramFiles"))?;
    let p = PathBuf::from(base).join("Microsoft Visual Studio").join("Installer").join("vswhere.exe");
    p.exists().then_some(p)
}

fn detect_msvc() -> Option<String> {
    if !cfg!(windows) {
        return Some("non requis hors Windows".into());
    }
    let mut cmd = std::process::Command::new(vswhere()?);
    cmd.args([
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ]);
    // vswhere is a console program: without this a window flashes each time.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

fn detect(t: &Tool) -> Option<String> {
    match t.detect {
        Detect::Msvc => detect_msvc(),
        Detect::OnPath(programs) => {
            let mut first = None;
            for p in programs {
                let found = which(p)?;
                first.get_or_insert(found);
            }
            first.map(|p| p.to_string_lossy().into_owned())
        }
    }
}

/// winget.exe is an "app execution alias" in `WindowsApps`. `which` skips that
/// folder (the Store's python stub lives there) and `Path::exists` cannot see
/// aliases, so it is looked up by hand.
fn winget() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let mut dirs: Vec<PathBuf> = current_path().map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\WindowsApps"));
    }
    dirs.into_iter().map(|d| d.join("winget.exe")).find(|p| std::fs::symlink_metadata(p).is_ok())
}

/// Does `winget --version` run? An alias without App Installer behind it does not.
async fn winget_runs(winget: &std::path::Path) -> bool {
    let quiet: LogSink = std::sync::Arc::new(|_, _| {});
    let Ok(cmd) = Cmd::new(&[winget.to_string_lossy().into_owned(), "--version".into()], std::env::temp_dir()) else {
        return false;
    };
    process::run(&cmd, quiet, &CancellationToken::new()).await.is_ok()
}

const INSTALL_WINGET_SCRIPT: &str = include_str!("install-winget.ps1");

/// Returns a working winget, installing App Installer first when needed.
async fn ensure_winget(sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<PathBuf> {
    if let Some(w) = winget()
        && winget_runs(&w).await
    {
        return Ok(w);
    }
    if !cfg!(windows) {
        bail!("winget only exists on Windows");
    }
    sink(Stream::Info, "winget is missing: installing “App Installer” (Microsoft)…");
    let script = std::env::temp_dir().join("oedvi-install-winget.ps1");
    // A BOM makes Windows PowerShell 5 read the accents as UTF-8.
    std::fs::write(&script, format!("\u{feff}{INSTALL_WINGET_SCRIPT}")).context("writing the winget install script")?;
    let powershell = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .filter(|p| p.is_file())
        .map_or_else(|| "powershell.exe".to_string(), |p| p.to_string_lossy().into_owned());
    let argv: Vec<String> = [
        powershell.as_str(),
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let result = process::run(&Cmd::new(&argv, std::env::temp_dir())?, sink.clone(), cancel).await;
    let _ = std::fs::remove_file(&script);
    if cancel.is_cancelled() {
        bail!("cancelled");
    }
    refresh_path();
    let found = winget();
    if let Some(w) = &found
        && winget_runs(w).await
    {
        // A fresh install has no package index yet; the first install would
        // otherwise fail with "source data is missing". Best effort.
        let update =
            Cmd::new(&[w.to_string_lossy().into_owned(), "source".into(), "update".into()], std::env::temp_dir())?;
        let _ = process::run(&update, sink.clone(), cancel).await;
        sink(Stream::Info, "✔ winget installed");
        return Ok(w.clone());
    }
    let why = result.err().map(|e| format!(" ({e:#})")).unwrap_or_default();
    bail!(
        "winget could not be installed{why}. Install “App Installer” from the Microsoft Store (or https://aka.ms/getwinget) and retry. Windows 10 1809 or newer is required."
    )
}

pub fn check(def: &BuilderDef) -> Vec<ToolStatus> {
    // winget is installed on demand, so Windows can always install.
    let can_install = cfg!(windows);
    needs(def)
        .into_iter()
        .map(|n| match n {
            Need::Known(id) => {
                let t = tool(id);
                let found = detect(t);
                ToolStatus {
                    id: id.into(),
                    label: t.label.into(),
                    installed: found.is_some(),
                    found,
                    installable: can_install,
                    admin: t.admin,
                    size: Some(t.size.into()),
                }
            }
            Need::Program(p) => {
                let found = which(&p).map(|f| f.to_string_lossy().into_owned());
                ToolStatus {
                    installed: found.is_some(),
                    found,
                    label: p.clone(),
                    id: p,
                    installable: false,
                    admin: false,
                    size: None,
                }
            }
        })
        .collect()
}

pub fn missing(def: &BuilderDef) -> Vec<ToolStatus> {
    check(def).into_iter().filter(|t| !t.installed).collect()
}

// --------------------------------------------------------------- install

/// Id of the winget an installer window started. A launcher closed during an
/// install leaves winget running; the next launcher finds it here instead of
/// starting a second install on top.
fn install_pidfile() -> PathBuf {
    std::env::temp_dir().join("oedvi-install.pid")
}

fn running_install() -> Option<(u32, String)> {
    let text = std::fs::read_to_string(install_pidfile()).ok()?;
    let (pid, image) = text.trim().split_once(' ')?;
    let pid: u32 = pid.parse().ok()?;
    if process::is_alive(pid, image) {
        Some((pid, image.to_string()))
    } else {
        let _ = std::fs::remove_file(install_pidfile());
        None
    }
}

/// Waits for an install started earlier (by another launcher window).
async fn wait_for_previous_install(sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    let mut told = false;
    while let Some((pid, image)) = running_install() {
        if !told {
            sink(
                Stream::Info,
                &format!(
                    "An install started earlier is still running ({image}, PID {pid}): waiting for it instead of starting a second one…"
                ),
            );
            told = true;
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            _ = cancel.cancelled() => bail!("cancelled"),
        }
    }
    if told {
        sink(Stream::Info, "The previous install has finished.");
        refresh_path();
    }
    Ok(())
}

/// One winget at a time: it refuses concurrent installs.
fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Installs what `def` needs and does not have. Returns the labels of the
/// installed tools (empty when nothing was missing).
pub async fn install_missing(
    def: &BuilderDef,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<String>> {
    let _guard = install_lock().lock().await;
    // After the wait: what the earlier install brought is not missing anymore.
    wait_for_previous_install(sink, cancel).await?;
    let todo = missing(def);
    if todo.is_empty() {
        return Ok(Vec::new());
    }
    let manual: Vec<String> = todo.iter().filter(|t| !t.installable).map(|t| t.label.clone()).collect();
    let auto: Vec<&ToolStatus> = todo.iter().filter(|t| t.installable).collect();
    let mut done = Vec::new();
    if !auto.is_empty() {
        let winget = ensure_winget(sink, cancel).await?;
        for t in auto {
            done.push(install_one(&winget, tool(&t.id), sink, cancel).await?);
        }
    }
    if !manual.is_empty() {
        if cfg!(windows) {
            bail!("the launcher cannot install: {} (install it and add it to PATH)", manual.join(", "));
        }
        bail!("install with the system package manager: {}", manual.join(", "));
    }
    // rustup installs the toolchain pinned by the repository on first use;
    // nothing more to do here.
    Ok(done)
}

async fn install_one(
    winget: &std::path::Path,
    entry: &'static Tool,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<String> {
    sink(
        Stream::Info,
        &format!(
            "Installing {} (~{}){}…",
            entry.label,
            entry.size,
            if entry.admin { " — Windows will ask for administrator permission" } else { "" }
        ),
    );
    let mut argv: Vec<String> = vec![
        winget.to_string_lossy().into_owned(),
        "install".into(),
        "--id".into(),
        entry.winget.into(),
        "--exact".into(),
        "--source".into(),
        "winget".into(),
        "--silent".into(),
        "--accept-package-agreements".into(),
        "--accept-source-agreements".into(),
        "--disable-interactivity".into(),
    ];
    argv.extend(entry.winget_args.iter().map(|s| s.to_string()));
    let cmd = Cmd::new(&argv, std::env::temp_dir())?;
    // winget's exit code is not reliable ("already installed", "reboot
    // needed"…): what counts is whether the tool is there afterwards.
    let image = winget.file_name().map_or_else(|| "winget.exe".into(), |n| n.to_string_lossy().into_owned());
    let result = process::run_notify(&cmd, sink.clone(), cancel, |pid| {
        let _ = std::fs::write(install_pidfile(), format!("{pid} {image}"));
    })
    .await;
    let _ = std::fs::remove_file(install_pidfile());
    if cancel.is_cancelled() {
        bail!("cancelled");
    }
    refresh_path();
    if detect(entry).is_none() {
        let why = result.err().map(|e| format!(" ({e:#})")).unwrap_or_default();
        bail!("{} is still not detected after the install{why}. Restarting Windows may be needed.", entry.label);
    }
    sink(Stream::Info, &format!("✔ {} installed", entry.label));
    Ok(entry.label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(stack: &str, requires: &[&str]) -> BuilderDef {
        let mut text = format!("name = \"x\"\nrepo = \"r\"\nrun = [\"a\"]\nstack = \"{stack}\"\n");
        text.push_str(&format!("requires = {:?}\n", requires));
        let mut d: BuilderDef = toml::from_str(&text).unwrap();
        d.id = "x".into();
        d
    }

    #[test]
    fn only_what_the_stack_needs() {
        let ids = |d: &BuilderDef| needs(d);
        let py = ids(&def("python", &[]));
        assert_eq!(py, vec![Need::Known("git"), Need::Known("python")]);

        let rust = ids(&def("rust", &[]));
        if cfg!(windows) {
            assert_eq!(rust, vec![Need::Known("git"), Need::Known("msvc"), Need::Known("rust")]);
        } else {
            assert_eq!(rust, vec![Need::Known("git"), Need::Known("rust")]);
        }

        let custom = ids(&def("custom", &["npm", "cargo", "cargo", "my-tool"]));
        assert_eq!(
            custom,
            vec![Need::Known("git"), Need::Known("rust"), Need::Known("node"), Need::Program("my-tool".into())]
        );
    }

    #[test]
    fn check_reports_unknown_programs_as_not_installable() {
        let st = check(&def("custom", &["surely-not-a-real-program-xyz"]));
        let t = st.iter().find(|t| t.id == "surely-not-a-real-program-xyz").unwrap();
        assert!(!t.installed && !t.installable);
    }

    #[test]
    fn winget_setup_script_is_embedded_and_sane() {
        assert!(INSTALL_WINGET_SCRIPT.contains("Add-AppxPackage"));
        assert!(INSTALL_WINGET_SCRIPT.contains("RegisterByFamilyName"));
        // Run with `-File` after a BOM: no double quotes to escape, no tabs.
        assert!(!INSTALL_WINGET_SCRIPT.contains('\t'));
    }

    #[cfg(not(windows))]
    #[test]
    fn winget_only_exists_on_windows() {
        assert!(winget().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn tells_a_live_process_from_a_dead_one() {
        assert!(process::is_alive(std::process::id(), "anything"));
        assert!(!process::is_alive(0, "x"));
        assert!(!process::is_alive(4_194_303, "x"));
    }
}
