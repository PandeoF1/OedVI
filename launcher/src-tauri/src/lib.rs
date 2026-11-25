//! Tauri side of the launcher: commands the UI calls, and events it listens
//! to (`sync-progress`, `sync-done`, `task-log`, `task-end`).

mod tasks;
mod update;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use oedvi_client::state::StateFiles;
use oedvi_client::sync::{self, Progress, ProgressSnapshot, Status, SyncOptions, SyncReport};
use oedvi_client::{Api, Layout, Settings, builder, git};
use oedvi_common::{BuilderDef, BuilderList, Manifest, ScanStatus};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

use tasks::{TaskKind, Tasks};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn anyerr(e: anyhow::Error) -> String {
    format!("{e:#}")
}

pub struct AppState {
    settings_path: PathBuf,
    settings: RwLock<Settings>,
    manifest: RwLock<Option<Manifest>>,
    builders: RwLock<Option<BuilderList>>,
    sync: Mutex<Option<SyncJob>>,
    tasks: Tasks,
    /// Last run's measures per builder, one point a second, for the live charts.
    live: Mutex<std::collections::HashMap<String, Vec<LivePoint>>>,
    specs: tokio::sync::OnceCell<oedvi_common::stats::Specs>,
    /// Recorder of each running game, to mark phases by hand.
    recorders: Mutex<std::collections::HashMap<String, Arc<oedvi_client::metrics::Recorder>>>,
}

/// What the game and the system measured at one second of a run.
#[derive(Clone, serde::Serialize)]
struct LivePoint {
    /// Seconds since the game started.
    t: f64,
    values: std::collections::BTreeMap<String, f64>,
    /// The phase of the game at that second.
    phase: Option<String>,
}

/// Points kept per builder: 15 minutes.
const LIVE_POINTS: usize = 900;

/// Once a second, sends the latest measures to every window.
fn live_ticker(app: &AppHandle, id: &str, recorder: Arc<oedvi_client::metrics::Recorder>, stop: CancellationToken) {
    let (app, id) = (app.clone(), id.to_string());
    app.state::<AppState>().live.lock().unwrap().insert(id.clone(), Vec::new());
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tick.tick() => {}
            }
            let values = recorder.latest();
            if values.is_empty() {
                continue;
            }
            let point = LivePoint { t: recorder.elapsed(), values, phase: recorder.phase() };
            {
                let state = app.state::<AppState>();
                let mut live = state.live.lock().unwrap();
                let buf = live.entry(id.clone()).or_default();
                if buf.len() >= LIVE_POINTS {
                    buf.remove(0);
                }
                buf.push(point.clone());
            }
            let _ = app.emit("metrics", serde_json::json!({ "id": id, "point": point }));
        }
    });
}

/// What is in the game folder (standalone mode: the user's own copy).
#[derive(Serialize)]
struct FolderInfo {
    path: PathBuf,
    exists: bool,
    /// Entries directly inside it.
    entries: usize,
}

#[tauri::command]
fn game_folder_info(state: State<AppState>) -> FolderInfo {
    let path = state.settings().layout().game;
    let entries = std::fs::read_dir(&path).map(|d| d.count()).unwrap_or(0);
    FolderInfo { exists: path.is_dir(), entries, path }
}

/// Marks the start of a phase of the running game (“menu”, “intro”…).
#[tauri::command]
fn mark_phase(state: State<AppState>, id: String, name: String) -> CmdResult<()> {
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err("phase name empty or too long".into());
    }
    let recorders = state.recorders.lock().unwrap();
    let recorder = recorders.get(&id).ok_or("the game is not running")?;
    recorder.mark(name);
    Ok(())
}

/// The points of the last run of a builder, for a window opened late.
#[tauri::command]
fn live_history(state: State<AppState>, id: String) -> Vec<LivePoint> {
    state.live.lock().unwrap().get(&id).cloned().unwrap_or_default()
}

/// This PC: CPU, memory, system, graphics cards and driver versions.
#[tauri::command]
async fn machine_specs(state: State<'_, AppState>) -> CmdResult<oedvi_common::stats::Specs> {
    Ok(state.specs.get_or_init(oedvi_client::monitor::specs).await.clone())
}

/// A small window, always on top, with the live charts of a builder: to keep
/// next to the game.
#[tauri::command]
async fn open_overlay(app: AppHandle, id: String) -> CmdResult<()> {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = app.emit_to("overlay", "overlay-select", &id);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let url = tauri::WebviewUrl::App(format!("index.html?overlay={id}").into());
    tauri::WebviewWindowBuilder::new(&app, "overlay", url)
        .title("OedVI · live stats")
        .inner_size(400.0, 620.0)
        .min_inner_size(300.0, 220.0)
        .always_on_top(true)
        .build()
        .map_err(err)?;
    Ok(())
}

struct SyncJob {
    progress: Arc<Progress>,
    cancel: CancellationToken,
}

impl AppState {
    fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    /// Connected to a server (remote mode with an address).
    fn remote(&self) -> bool {
        let s = self.settings.read().unwrap();
        s.mode == oedvi_client::settings::Mode::Remote && !s.server_url.trim().is_empty()
    }

    fn api(&self) -> CmdResult<Api> {
        if !self.remote() {
            return Err("standalone mode: there is no server".into());
        }
        let s = self.settings();
        Api::new(&s.server_url, &s.token).map_err(anyerr)
    }

    /// The builder and template files of this PC (standalone mode).
    fn local_store(&self) -> CmdResult<oedvi_common::store::Store> {
        oedvi_client::local::store(&self.settings().layout()).map_err(err)
    }

    fn state_files(&self) -> CmdResult<(Layout, StateFiles)> {
        let layout = self.settings().layout();
        layout.create_all().map_err(err)?;
        let files = StateFiles::new(&layout);
        Ok((layout, files))
    }

    fn manifest(&self) -> CmdResult<Manifest> {
        if let Some(m) = self.manifest.read().unwrap().clone() {
            return Ok(m);
        }
        let (_, files) = self.state_files()?;
        let m = files.manifest().ok_or("no manifest: connect to the server")?;
        *self.manifest.write().unwrap() = Some(m.clone());
        Ok(m)
    }

    fn builder(&self, id: &str) -> CmdResult<BuilderDef> {
        let list = self.builders.read().unwrap().clone().or_else(|| self.state_files().ok()?.1.builders());
        list.and_then(|l| l.builders.into_iter().find(|b| b.id == id))
            .ok_or_else(|| format!("builder {id:?} is unknown or invalid"))
    }
}

// ---------------------------------------------------------------- settings

#[derive(Serialize)]
struct SettingsView {
    settings: Settings,
    layout: Layout,
    path: PathBuf,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsView {
    let settings = state.settings();
    SettingsView { layout: settings.layout(), settings, path: state.settings_path.clone() }
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> CmdResult<SettingsView> {
    if state.sync.lock().unwrap().is_some() {
        return Err("a sync is in progress".into());
    }
    let changed_server = {
        let old = state.settings.read().unwrap();
        old.server_url != settings.server_url || old.token != settings.token
    };
    settings.save(&state.settings_path).map_err(anyerr)?;
    *state.settings.write().unwrap() = settings;
    if changed_server {
        *state.manifest.write().unwrap() = None;
        *state.builders.write().unwrap() = None;
    }
    Ok(get_settings(state))
}

#[tauri::command]
fn open_path(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    std::fs::create_dir_all(&path).ok();
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
}

// ------------------------------------------------------------------ server

#[derive(Serialize)]
struct ServerInfo {
    online: bool,
    error: Option<String>,
    version: Option<String>,
    scan: Option<ScanStatus>,
}

#[tauri::command]
async fn server_info(state: State<'_, AppState>) -> CmdResult<ServerInfo> {
    if !state.remote() {
        return Ok(ServerInfo { online: false, error: None, version: None, scan: None });
    }
    let api = state.api()?;
    match api.health().await {
        Ok(h) => {
            let scan = api.status().await;
            Ok(ServerInfo {
                online: true,
                error: scan.as_ref().err().map(|e| format!("{e:#}")),
                version: h.get("version").and_then(|v| v.as_str()).map(String::from),
                scan: scan.ok(),
            })
        }
        Err(e) => Ok(ServerInfo { online: false, error: Some(format!("{e:#}")), version: None, scan: None }),
    }
}

#[tauri::command]
async fn server_rescan(state: State<'_, AppState>) -> CmdResult<()> {
    state.api()?.rescan().await.map_err(anyerr)
}

// ------------------------------------------------------------- game files

#[derive(Serialize)]
struct ManifestInfo {
    version: u64,
    generated_at: u64,
    total_size: u64,
    file_count: usize,
    groups: Vec<oedvi_common::GroupInfo>,
    /// Loaded from the local copy because the server did not answer.
    offline: bool,
    error: Option<String>,
}

#[tauri::command]
async fn refresh_manifest(state: State<'_, AppState>) -> CmdResult<ManifestInfo> {
    let api = state.api()?;
    let (offline, error, m) = match api.manifest().await {
        Ok(m) => {
            let (_, files) = state.state_files()?;
            files.save_manifest(&m).map_err(anyerr)?;
            (false, None, m)
        }
        Err(e) => {
            let (_, files) = state.state_files()?;
            match files.manifest() {
                Some(m) => (true, Some(format!("{e:#}")), m),
                None => return Err(format!("{e:#}")),
            }
        }
    };
    let info = ManifestInfo {
        version: m.version,
        generated_at: m.generated_at,
        total_size: m.total_size,
        file_count: m.files.len(),
        groups: m.groups.clone(),
        offline,
        error,
    };
    *state.manifest.write().unwrap() = Some(m);
    Ok(info)
}

#[tauri::command]
async fn local_status(state: State<'_, AppState>) -> CmdResult<Status> {
    let m = state.manifest()?;
    let (layout, files) = state.state_files()?;
    let delete_extra = state.settings().delete_extra;
    tokio::task::spawn_blocking(move || {
        sync::local_status(&m, &files.selection(), &files.index(), &layout.game, delete_extra).map_err(anyerr)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn set_selection(
    state: State<'_, AppState>,
    kind: String,
    target: Option<String>,
    selected: bool,
) -> CmdResult<Status> {
    let m = state.manifest()?;
    let (_, files) = state.state_files()?;
    let mut sel = files.selection();
    match (kind.as_str(), target) {
        ("all", _) => sel.set_all(selected),
        ("group", Some(g)) => sel.set_group(&m, &g, selected),
        ("path", Some(p)) => sel.set_path(&p, selected),
        _ => return Err("invalid selection".into()),
    }
    sel.simplify(&m);
    files.save_selection(&sel).map_err(anyerr)?;
    local_status(state).await
}

#[tauri::command]
async fn start_sync(app: AppHandle, state: State<'_, AppState>, verify: bool) -> CmdResult<()> {
    if state.tasks.any_running(TaskKind::Run) {
        return Err("close the game before syncing (its files are open)".into());
    }
    let progress = Arc::new(Progress::default());
    let cancel = CancellationToken::new();
    {
        let mut slot = state.sync.lock().unwrap();
        if slot.is_some() {
            return Err("a sync is already running".into());
        }
        *slot = Some(SyncJob { progress: progress.clone(), cancel: cancel.clone() });
    }
    let api = state.api()?;
    let m = state.manifest()?;
    let (layout, files) = state.state_files()?;
    let s = state.settings();
    let opts =
        SyncOptions { verify_all: verify, delete_extra: s.delete_extra, jobs: s.download_jobs, check_space: true };

    let ticker = {
        let app = app.clone();
        let progress = progress.clone();
        tokio::spawn(async move {
            loop {
                let _ = app.emit("sync-progress", progress.snapshot());
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
    };
    tauri::async_runtime::spawn(async move {
        let result =
            sync::sync(&api, &m, &files.selection(), &files, &layout.game, &opts, progress.clone(), cancel).await;
        ticker.abort();
        let _ = app.emit("sync-progress", progress.snapshot());
        let payload: Result<SyncReport, String> = result.map_err(anyerr);
        let _ = app.emit("sync-done", payload);
        *app.state::<AppState>().sync.lock().unwrap() = None;
    });
    Ok(())
}

#[tauri::command]
fn cancel_sync(state: State<AppState>) {
    if let Some(job) = state.sync.lock().unwrap().as_ref() {
        job.cancel.cancel();
    }
}

#[tauri::command]
fn sync_progress(state: State<AppState>) -> Option<ProgressSnapshot> {
    state.sync.lock().unwrap().as_ref().map(|j| j.progress.snapshot())
}

// ---------------------------------------------------------------- builders

#[tauri::command]
async fn list_builders(state: State<'_, AppState>, refresh: bool) -> CmdResult<BuilderListView> {
    let mut offline = None;
    if !state.remote() {
        // Standalone: the files of this PC are the truth, read each time.
        *state.builders.write().unwrap() = Some(state.local_store()?.resolved().map_err(err)?);
    } else if refresh || state.builders.read().unwrap().is_none() {
        match state.api()?.builders().await {
            Ok(list) => {
                let (_, files) = state.state_files()?;
                files.save_builders(&list).map_err(anyerr)?;
                *state.builders.write().unwrap() = Some(list);
            }
            Err(e) => {
                offline = Some(format!("{e:#}"));
                if state.builders.read().unwrap().is_none() {
                    let (_, files) = state.state_files()?;
                    *state.builders.write().unwrap() = files.builders();
                }
            }
        }
    }
    let list = state.builders.read().unwrap().clone().unwrap_or_default();
    let layout = state.settings().layout();
    let builders = list
        .builders
        .into_iter()
        .map(|b| {
            let cloned = git::is_cloned(&builder::checkout_dir(&layout, &b));
            let task = state.tasks.current(&b.id);
            BuilderRow { def: b, cloned, task }
        })
        .collect();
    Ok(BuilderListView { builders, errors: list.errors, offline })
}

#[derive(Serialize)]
struct BuilderListView {
    builders: Vec<BuilderRow>,
    errors: Vec<oedvi_common::BuilderLoadError>,
    offline: Option<String>,
}

#[derive(Serialize)]
struct BuilderRow {
    def: BuilderDef,
    cloned: bool,
    task: Option<tasks::TaskInfo>,
}

#[derive(Serialize)]
struct BuilderDetail {
    def: BuilderDef,
    prepared: Option<builder::Prepared>,
    prepare_error: Option<String>,
    git: git::GitStatus,
    build: Option<builder::BuildState>,
    tools: Vec<oedvi_client::prereq::ToolStatus>,
    task: Option<tasks::TaskInfo>,
    shared_with: Vec<String>,
}

#[tauri::command]
async fn builder_detail(state: State<'_, AppState>, id: String) -> CmdResult<BuilderDetail> {
    let def = state.builder(&id)?;
    let settings = state.settings();
    let (_, files) = state.state_files()?;
    let dir = builder::checkout_dir(&settings.layout(), &def);
    let git = git::status(&dir, &def.git_ref).await.map_err(anyerr)?;
    let (prepared, prepare_error) = match builder::prepare(&def, &settings) {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    let build = match &prepared {
        Some(p) => Some(builder::build_state(p, &files).await),
        None => None,
    };
    let shared_with = state
        .builders
        .read()
        .unwrap()
        .as_ref()
        .map(|l| {
            l.builders
                .iter()
                .filter(|b| b.id != def.id && b.checkout_key() == def.checkout_key())
                .map(|b| b.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let tools = {
        let def = def.clone();
        tokio::task::spawn_blocking(move || oedvi_client::prereq::check(&def)).await.map_err(err)?
    };
    Ok(BuilderDetail { tools, task: state.tasks.current(&id), def, prepared, prepare_error, git, build, shared_with })
}

/// Saves the run, and sends it to the server unless the player opted out.
async fn record_session(
    app: &AppHandle,
    builder: &str,
    commit: &str,
    recorder: &oedvi_client::metrics::Recorder,
    sink: &oedvi_client::process::LogSink,
) {
    use oedvi_client::process::Stream::Info;
    let state = app.state::<AppState>();
    let settings = state.settings();
    let id = format!("{:016x}{:016x}", rand_u64(), rand_u64());
    let Some(session) = recorder.finish(id, builder, commit, &settings.machine_name) else {
        sink(Info, "Nothing could be measured: no session recorded.");
        return;
    };
    let mut session = session;
    session.specs = Some(state.specs.get_or_init(oedvi_client::monitor::specs).await.clone());
    if session.samples == 0 {
        sink(
            Info,
            "No FPS found in the game output (see the builder's fps_patterns): only the system measures are kept.",
        );
    }
    sink(
        Info,
        &format!(
            "Performance: {:.1} FPS on average, {:.1} at the lowest (1 %), {} metrics over {:.0} s",
            session.avg,
            session.low1,
            session.metrics.len(),
            session.duration_secs
        ),
    );
    if let Ok((_, files)) = state.state_files()
        && let Err(e) = files.add_session(&session)
    {
        sink(Info, &format!("Cannot save the result locally: {e:#}"));
    }
    if settings.share_stats && state.remote() {
        match state.api().map(|a| async move { a.post_session(&session).await }) {
            Ok(fut) => match fut.await {
                Ok(()) => sink(Info, "Result sent to the server for the comparison."),
                Err(e) => sink(Info, &format!("Cannot send the result to the server: {e:#}")),
            },
            Err(e) => sink(Info, &format!("Cannot send the result to the server: {e}")),
        }
    }
    let _ = app.emit("stats-changed", ());
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// Runs recorded on this PC, newest first.
#[tauri::command]
fn stats_local(state: State<AppState>) -> CmdResult<Vec<oedvi_common::stats::Session>> {
    let (_, files) = state.state_files()?;
    let mut all = files.sessions();
    all.reverse();
    Ok(all)
}

/// Every builder and version compared, from the server.
#[tauri::command]
async fn stats_remote(state: State<'_, AppState>) -> CmdResult<oedvi_common::stats::StatsReport> {
    state.api()?.stats().await.map_err(anyerr)
}

/// `tools`, `fetch`, `pull`, `resync`, `build`, `rebuild` or `play`.
#[tauri::command]
async fn builder_action(app: AppHandle, state: State<'_, AppState>, id: String, action: String) -> CmdResult<()> {
    let def = state.builder(&id)?;
    let settings = state.settings();
    let (_, files) = state.state_files()?;
    let kind = match action.as_str() {
        "tools" => TaskKind::Setup,
        "fetch" | "pull" | "resync" => TaskKind::Git,
        "build" | "rebuild" => TaskKind::Build,
        "play" => TaskKind::Run,
        _ => return Err(format!("unknown action {action:?}")),
    };
    if kind == TaskKind::Run && state.sync.lock().unwrap().is_some() {
        return Err("wait for the sync to finish".into());
    }
    let prepared = builder::prepare(&def, &settings).map_err(anyerr)?;
    let (cancel, sink) = state.tasks.start(&app, &def, kind, &action)?;
    let dir = prepared.checkout_dir.clone();
    let auto = settings.auto_install_tools;
    let game_dir = settings.layout().game;

    tauri::async_runtime::spawn(async move {
        let result: anyhow::Result<String> = async {
            if matches!(action.as_str(), "fetch" | "pull" | "resync") {
                builder::ensure_git(&def, auto, &sink, &cancel).await?;
            }
            match action.as_str() {
                "tools" => {
                    let done = oedvi_client::prereq::install_missing(&def, &sink, &cancel).await?;
                    Ok(if done.is_empty() {
                        "all the tools are already there".into()
                    } else {
                        format!("installed: {}", done.join(", "))
                    })
                }
                "fetch" | "pull" | "resync" if !git::is_cloned(&dir) => {
                    git::clone(&def.repo, &def.git_ref, &dir, &sink, &cancel).await?;
                    Ok("cloned".into())
                }
                "fetch" => {
                    git::fetch(&dir, &sink, &cancel).await?;
                    Ok("up to date with the remote repository".into())
                }
                "pull" => {
                    git::ensure_remote(&dir, &def.repo, &sink, &cancel).await?;
                    git::pull(&dir, &def.git_ref, &sink, &cancel).await?;
                    Ok("pull finished".into())
                }
                "resync" => {
                    git::ensure_remote(&dir, &def.repo, &sink, &cancel).await?;
                    git::resync(&dir, &def.git_ref, &sink, &cancel).await?;
                    Ok("resynced with the repository".into())
                }
                "build" | "rebuild" => {
                    if action == "build" && builder::build_state(&prepared, &files).await.up_to_date {
                        sink(oedvi_client::process::Stream::Info, "Already built for this commit.");
                        return Ok("already up to date".into());
                    }
                    builder::build(&prepared, &files, auto, sink.clone(), &cancel).await?;
                    Ok("build finished".into())
                }
                "play" => {
                    // Better to know now than after a long build.
                    if let Some(absent) = builder::missing_game_files(&prepared, &game_dir).first() {
                        anyhow::bail!(
                            "the game is not there ({} not found): set the game folder in the Game tab (or sync it, in remote mode)",
                            absent.display()
                        );
                    }
                    if !builder::build_state(&prepared, &files).await.up_to_date {
                        builder::build(&prepared, &files, auto, sink.clone(), &cancel).await?;
                    }
                    // What the game prints and what the system does are recorded
                    // for the live charts and the comparison.
                    let (recorder, bad) =
                        oedvi_client::metrics::Recorder::new(&def.fps_patterns, &def.metrics, &def.phases);
                    for p in bad {
                        sink(
                            oedvi_client::process::Stream::Info,
                            &format!("ignored invalid regex: {p}"),
                        );
                    }
                    let recorder = std::sync::Arc::new(recorder);
                    let game_sink: oedvi_client::process::LogSink = {
                        let (recorder, sink) = (recorder.clone(), sink.clone());
                        std::sync::Arc::new(move |stream, line: &str| {
                            recorder.feed(line);
                            sink(stream, line)
                        })
                    };
                    let commit = git::head(&dir).await.unwrap_or_default();
                    let running = builder::launch(&prepared, game_sink)?;
                    let pid = running.pid;
                    app.state::<AppState>().tasks.set_pid(&def.id, pid);
                    let monitor_stop = CancellationToken::new();
                    oedvi_client::monitor::spawn(pid, recorder.clone(), monitor_stop.clone());
                    live_ticker(&app, &def.id, recorder.clone(), monitor_stop.clone());
                    app.state::<AppState>().recorders.lock().unwrap().insert(def.id.clone(), recorder.clone());
                    let wait = running.wait();
                    tokio::pin!(wait);
                    let outcome: anyhow::Result<String> = tokio::select! {
                        s = &mut wait => {
                            let s = s?;
                            if s.success() {
                                Ok("the game closed".into())
                            } else {
                                Err(anyhow::anyhow!("the game stopped ({})", oedvi_client::process::describe(s)))
                            }
                        }
                        _ = cancel.cancelled() => {
                            // Off the async threads (taskkill can take a second), and
                            // without waiting for pipes a surviving child keeps open.
                            let _ = tokio::task::spawn_blocking(move || oedvi_client::process::kill_tree(pid)).await;
                            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), wait).await;
                            Ok("game stopped".into())
                        }
                    };
                    monitor_stop.cancel();
                    app.state::<AppState>().recorders.lock().unwrap().remove(&def.id);
                    record_session(&app, &def.id, &commit, &recorder, &sink).await;
                    outcome
                }
                _ => unreachable!(),
            }
        }
        .await;
        app.state::<AppState>().tasks.finish(&app, &def.id, result);
    });
    Ok(())
}

#[tauri::command]
fn stop_task(state: State<AppState>, id: String) {
    state.tasks.cancel(&id);
}

#[tauri::command]
fn task_logs(state: State<AppState>, id: String) -> Vec<tasks::LogLine> {
    state.tasks.logs(&id)
}

#[tauri::command]
fn clear_logs(state: State<AppState>, id: String) {
    state.tasks.clear_logs(&id)
}

#[tauri::command]
fn set_builder_env(
    state: State<AppState>,
    id: String,
    env: std::collections::BTreeMap<String, String>,
) -> CmdResult<()> {
    let mut s = state.settings();
    if env.is_empty() {
        s.builder_env.remove(&id);
    } else {
        s.builder_env.insert(id, env);
    }
    s.save(&state.settings_path).map_err(anyerr)?;
    *state.settings.write().unwrap() = s;
    Ok(())
}

// ------------------------------------------------------- editing on server

#[tauri::command]
async fn list_ids(state: State<'_, AppState>, kind: String) -> CmdResult<Vec<String>> {
    if !state.remote() {
        let kind = oedvi_common::store::Kind::parse(&kind).ok_or("unknown collection")?;
        return Ok(state.local_store()?.read_all(kind).map_err(err)?.into_keys().collect());
    }
    state.api()?.list_ids(&kind).await.map_err(anyerr)
}

#[tauri::command]
async fn read_raw(state: State<'_, AppState>, kind: String, id: String) -> CmdResult<Option<String>> {
    if !state.remote() {
        let kind = oedvi_common::store::Kind::parse(&kind).ok_or("unknown collection")?;
        return state.local_store()?.read(kind, &id);
    }
    state.api()?.read_raw(&kind, &id).await.map_err(anyerr)
}

#[tauri::command]
async fn check_raw(state: State<'_, AppState>, kind: String, id: String, text: String) -> CmdResult<BuilderList> {
    if !state.remote() {
        let kind = oedvi_common::store::Kind::parse(&kind).ok_or("unknown collection")?;
        return state.local_store()?.check(kind, &id, &text);
    }
    state.api()?.check_raw(&kind, &id, &text).await.map_err(anyerr)
}

#[tauri::command]
async fn write_raw(state: State<'_, AppState>, kind: String, id: String, text: String) -> CmdResult<BuilderList> {
    if !state.remote() {
        let kind = oedvi_common::store::Kind::parse(&kind).ok_or("unknown collection")?;
        let list = state.local_store()?.write(kind, &id, &text)?;
        *state.builders.write().unwrap() = Some(list.clone());
        return Ok(list);
    }
    let list = state.api()?.write_raw(&kind, &id, &text).await.map_err(anyerr)?;
    *state.builders.write().unwrap() = Some(list.clone());
    Ok(list)
}

#[tauri::command]
async fn delete_raw(state: State<'_, AppState>, kind: String, id: String) -> CmdResult<()> {
    if !state.remote() {
        let kind = oedvi_common::store::Kind::parse(&kind).ok_or("unknown collection")?;
        state.local_store()?.delete(kind, &id)?;
    } else {
        state.api()?.delete_raw(&kind, &id).await.map_err(anyerr)?;
    }
    *state.builders.write().unwrap() = None;
    Ok(())
}

// -------------------------------------------------------------------- app

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("OEDVI_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    // Tools installed since Windows started this process are on PATH too.
    oedvi_client::prereq::refresh_path();
    let settings_path = std::env::var_os("OEDVI_SETTINGS").map(PathBuf::from).unwrap_or_else(Settings::default_path);
    let settings = Settings::load(&settings_path).unwrap_or_else(|e| {
        tracing::error!("{e:#}; using defaults");
        Settings::default()
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(update::Pending::default())
        .manage(AppState {
            settings_path,
            settings: RwLock::new(settings),
            manifest: RwLock::new(None),
            builders: RwLock::new(None),
            sync: Mutex::new(None),
            tasks: Tasks::default(),
            live: Mutex::default(),
            recorders: Mutex::default(),
            specs: tokio::sync::OnceCell::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            open_path,
            server_info,
            server_rescan,
            refresh_manifest,
            local_status,
            stats_local,
            live_history,
            game_folder_info,
            mark_phase,
            machine_specs,
            open_overlay,
            stats_remote,
            set_selection,
            start_sync,
            cancel_sync,
            sync_progress,
            list_builders,
            builder_detail,
            builder_action,
            stop_task,
            task_logs,
            clear_logs,
            set_builder_env,
            list_ids,
            read_raw,
            check_raw,
            write_raw,
            delete_raw,
            update::check_update,
            update::install_update,
        ])
        .on_window_event(|window, event| {
            // Closing the launcher stops builds; a running game keeps running.
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                state.tasks.cancel_all(&[TaskKind::Git, TaskKind::Build]);
                if let Some(job) = state.sync.lock().unwrap().as_ref() {
                    job.cancel.cancel();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
