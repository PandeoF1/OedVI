//! Preparing, building and running a builder on this machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, anyhow};
use oedvi_common::BuilderDef;
use oedvi_common::builder::{resolve_vars, substitute, substitute_argv};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::git;
use crate::process::{self, Cmd, LogSink, Running, Stream};
use crate::settings::{Layout, Settings};
use crate::state::{BuildRecord, StateFiles, now_secs};

/// A builder with every placeholder replaced for this machine.
#[derive(Debug, Clone, Serialize)]
pub struct Prepared {
    pub def: BuilderDef,
    pub checkout_dir: PathBuf,
    pub work_dir: PathBuf,
    pub build_steps: Vec<CmdSpec>,
    pub run: CmdSpec,
    /// Changes when the build commands or their environment change.
    pub recipe: String,
}

/// Serializable form of a command, for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct CmdSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl CmdSpec {
    fn to_cmd(&self) -> anyhow::Result<Cmd> {
        let mut c = Cmd::new(&self.argv, &self.cwd)?;
        c.env = self.env.clone();
        Ok(c)
    }
}

pub fn checkout_dir(layout: &Layout, def: &BuilderDef) -> PathBuf {
    layout.repos.join(def.checkout_key())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn prepare(def: &BuilderDef, settings: &Settings) -> anyhow::Result<Prepared> {
    let layout = settings.layout();
    let checkout = checkout_dir(&layout, def);
    let work_dir = match &def.workdir {
        Some(w) => oedvi_common::paths::join_safe(&checkout, w).context("workdir invalide")?,
        None => checkout.clone(),
    };
    let cache_dir = layout.cache.join(def.checkout_key());
    let builtins: BTreeMap<String, String> = [
        ("game_dir", path_str(&layout.game)),
        ("cache_dir", path_str(&cache_dir)),
        ("data_dir", path_str(&layout.data)),
        ("root_dir", path_str(&layout.root)),
        ("repo_dir", path_str(&checkout)),
        ("work_dir", path_str(&work_dir)),
        ("builder_id", def.id.clone()),
        ("exe_suffix", std::env::consts::EXE_SUFFIX.to_string()),
        ("os", std::env::consts::OS.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let vars = resolve_vars(&builtins, &def.vars).map_err(|e| anyhow!(e))?;
    let sub_env = |m: &BTreeMap<String, String>| -> anyhow::Result<BTreeMap<String, String>> {
        m.iter().map(|(k, v)| Ok((k.clone(), substitute(v, &vars).map_err(|e| anyhow!(e))?))).collect()
    };
    let common = sub_env(&def.env)?;
    let mut build_env = common.clone();
    build_env.extend(sub_env(&def.build_env)?);
    let mut run_env = common;
    run_env.extend(sub_env(&def.run_env)?);
    if let Some(local) = settings.builder_env.get(&def.id) {
        run_env.extend(local.clone());
    }

    let build_steps = def
        .build
        .iter()
        .map(|step| {
            Ok(CmdSpec {
                argv: substitute_argv(step, &vars).map_err(|e| anyhow!(e))?,
                cwd: work_dir.clone(),
                env: build_env.clone(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let run_cwd = match &def.run_workdir {
        Some(w) => PathBuf::from(substitute(w, &vars).map_err(|e| anyhow!(e))?),
        None => work_dir.clone(),
    };
    let run = CmdSpec { argv: substitute_argv(&def.run, &vars).map_err(|e| anyhow!(e))?, cwd: run_cwd, env: run_env };

    let mut h = blake3::Hasher::new();
    for s in &build_steps {
        h.update(serde_json::to_string(&(&s.argv, &s.cwd, &s.env))?.as_bytes());
    }
    let recipe = h.finalize().to_hex()[..16].to_string();

    std::fs::create_dir_all(&cache_dir)?;
    Ok(Prepared { def: def.clone(), checkout_dir: checkout, work_dir, build_steps, run, recipe })
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildState {
    pub head: Option<String>,
    pub last: Option<BuildRecord>,
    /// HEAD was built successfully with the current recipe and the program
    /// to run exists.
    pub up_to_date: bool,
}

pub async fn build_state(p: &Prepared, state: &StateFiles) -> BuildState {
    let head = if git::is_cloned(&p.checkout_dir) { git::head(&p.checkout_dir).await.ok() } else { None };
    let last = state.builds().get(p.def.checkout_key()).and_then(|m| m.get(&p.recipe)).cloned();
    // The program, and any argument naming a file inside the clone (a script
    // the build writes, say), must still exist: a resync or a manual clean
    // can remove build output.
    let checkout = path_str(&p.checkout_dir);
    let program_exists = p.run.argv.iter().enumerate().all(|(i, arg)| {
        let looks_like_path = arg.contains(['/', '\\']);
        if i == 0 && looks_like_path {
            let path = PathBuf::from(arg);
            return if path.is_relative() { p.run.cwd.join(path).exists() } else { path.exists() };
        }
        !(looks_like_path && arg.starts_with(&checkout)) || PathBuf::from(arg).exists()
    });
    let up_to_date = matches!((&head, &last), (Some(h), Some(r)) if r.ok && &r.commit == h) && program_exists;
    BuildState { head, last, up_to_date }
}

/// Clones when needed, installs the missing tools this builder needs (when
/// `install_tools`), and runs the build steps.
pub async fn build(
    p: &Prepared,
    state: &StateFiles,
    install_tools: bool,
    sink: LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    ensure_tools(&p.def, install_tools, &sink, cancel).await?;
    if !git::is_cloned(&p.checkout_dir) {
        sink(Stream::Info, &format!("Clonage de {} ({})", p.def.repo, p.def.git_ref));
        git::clone(&p.def.repo, &p.def.git_ref, &p.checkout_dir, &sink, cancel).await?;
    }
    let head = git::head(&p.checkout_dir).await?;
    let started = Instant::now();
    let mut result = Ok(());
    for (i, step) in p.build_steps.iter().enumerate() {
        sink(Stream::Info, &format!("Step {}/{}", i + 1, p.build_steps.len()));
        if let Err(e) = process::run(&step.to_cmd()?, sink.clone(), cancel).await {
            result = Err(e);
            break;
        }
    }
    if cancel.is_cancelled() {
        return result;
    }
    let mut builds = state.builds();
    builds.entry(p.def.checkout_key().to_string()).or_default().insert(
        p.recipe.clone(),
        BuildRecord {
            commit: head,
            ok: result.is_ok(),
            finished_at: now_secs(),
            secs: started.elapsed().as_secs_f64(),
        },
    );
    state.save_builds(&builds)?;
    match &result {
        Ok(()) => sink(Stream::Info, &format!("Build finished in {:.0} s", started.elapsed().as_secs_f64())),
        Err(e) => sink(Stream::Info, &format!("Build failed: {e:#}")),
    }
    result
}

/// Files the run command names inside the game directory that are not there
/// (the game is not downloaded yet, or not entirely).
pub fn missing_game_files(p: &Prepared, game_dir: &std::path::Path) -> Vec<PathBuf> {
    let game = path_str(game_dir);
    p.run.argv.iter().filter(|a| a.starts_with(&game)).map(PathBuf::from).filter(|f| !f.exists()).collect()
}

/// Starts the game.
pub fn launch(p: &Prepared, sink: LogSink) -> anyhow::Result<Running> {
    process::spawn(&p.run.to_cmd()?, sink)
}

/// Makes sure the tools `def` needs are there: installs the missing ones when
/// allowed, fails with their list otherwise. Tools already present are never
/// touched.
pub async fn ensure_tools(
    def: &BuilderDef,
    install: bool,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let missing = crate::prereq::missing(def);
    if missing.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = missing.iter().map(|t| t.label.as_str()).collect();
    if !install {
        anyhow::bail!(
            "missing tools: {} (use the “Install the tools” button, or install them and retry)",
            names.join(", ")
        );
    }
    sink(Stream::Info, &format!("Outils manquants pour {} : {}", def.name, names.join(", ")));
    crate::prereq::install_missing(def, sink, cancel).await?;
    Ok(())
}

/// For git actions: only git is needed.
pub async fn ensure_git(
    def: &BuilderDef,
    install: bool,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let mut only_git = def.clone();
    only_git.stack = "custom".into();
    only_git.requires.clear();
    ensure_tools(&only_git, install, sink, cancel).await
}
