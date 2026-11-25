//! `oedvi`: the launcher without its window. Same settings and state as the
//! desktop app, handy over SSH, in scripts, or to debug a builder.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use oedvi_client::process::{LogSink, Stream};
use oedvi_client::settings::Mode;
use oedvi_client::state::StateFiles;
use oedvi_client::sync::{self, FileState, Progress, SyncOptions};
use oedvi_client::{Api, Settings, builder, git};
use oedvi_common::BuilderDef;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "oedvi",
    version,
    about = "OedVI: build and run the game from source, optionally synced with an OedVI server"
)]
struct Cli {
    /// Settings file (shared with the launcher).
    #[arg(long, env = "OEDVI_SETTINGS", global = true)]
    settings: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Shows or changes the settings.
    Config {
        /// `standalone` (everything on this PC) or `remote` (connected to a server).
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        game_dir: Option<PathBuf>,
        #[arg(long)]
        jobs: Option<usize>,
        /// true/false: delete local files the server no longer publishes.
        #[arg(long)]
        delete_extra: Option<bool>,
        /// true/false : installer les outils manquants avant un build.
        #[arg(long)]
        auto_install_tools: Option<bool>,
    },
    /// State of the local files compared to the server (remote mode).
    Status {
        /// Also lists the files that are not up to date.
        #[arg(long)]
        files: bool,
    },
    /// Downloads, updates and deletes to match the server (remote mode).
    Sync {
        /// Re-reads every file (full verification).
        #[arg(long)]
        verify: bool,
        /// Does not delete local files missing from the manifest.
        #[arg(long)]
        keep_extra: bool,
        #[arg(long)]
        no_space_check: bool,
    },
    /// Which files to keep locally (remote mode).
    Select {
        #[command(subcommand)]
        action: SelectAction,
    },
    /// Liste les builders.
    Builders,
    /// Git on the clone of a builder.
    Git {
        id: String,
        #[arg(value_parser = ["status", "fetch", "pull", "resync"], default_value = "status")]
        action: String,
    },
    /// Tools a builder needs (git, Rust, Build Tools…).
    Tools {
        id: String,
        /// Installe ceux qui manquent (winget, Windows).
        #[arg(long)]
        install: bool,
    },
    /// Compile un builder.
    Build {
        id: String,
        /// Rebuilds even if nothing changed.
        #[arg(long)]
        force: bool,
    },
    /// Compile si besoin puis lance le jeu.
    Run {
        id: String,
        #[arg(long)]
        no_build: bool,
    },
    /// Server side (remote mode).
    Server {
        #[arg(value_parser = ["status", "rescan"])]
        action: String,
    },
    /// Reads, writes or deletes a builder or template (on the server in remote mode, on this PC otherwise).
    Edit {
        #[arg(value_parser = ["builders", "templates"])]
        kind: String,
        id: String,
        /// TOML file to save; without it, prints the current content.
        #[arg(long, conflicts_with = "delete")]
        put: Option<PathBuf>,
        #[arg(long)]
        delete: bool,
    },
}

#[derive(Subcommand)]
enum SelectAction {
    /// Shows the groups and the rules.
    Show,
    All,
    None,
    Group {
        name: String,
        #[arg(action = clap::ArgAction::Set, value_parser = clap::builder::BoolishValueParser::new())]
        on: bool,
    },
    Path {
        prefix: String,
        #[arg(action = clap::ArgAction::Set, value_parser = clap::builder::BoolishValueParser::new())]
        on: bool,
    },
}

fn gb(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.2} Go", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.1} Mo", b as f64 / (1u64 << 20) as f64)
    }
}

fn sink() -> LogSink {
    Arc::new(|stream, line| match stream {
        Stream::Stderr => eprintln!("{line}"),
        Stream::Info => println!("\x1b[36m» {line}\x1b[0m"),
        Stream::Stdout => println!("{line}"),
    })
}

fn ctrl_c_token() -> CancellationToken {
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("\nstopping…");
        t.cancel();
    });
    token
}

struct Ctx {
    settings: Settings,
    path: PathBuf,
}

impl Ctx {
    fn remote(&self) -> bool {
        self.settings.mode == Mode::Remote && !self.settings.server_url.trim().is_empty()
    }
    fn api(&self) -> anyhow::Result<Api> {
        if !self.remote() {
            bail!("standalone mode: there is no server. Connect to one with `oedvi config --mode remote --server URL`");
        }
        Api::new(&self.settings.server_url, &self.settings.token)
    }
    /// The builders and templates of this PC (standalone mode).
    fn local_store(&self) -> anyhow::Result<oedvi_common::store::Store> {
        Ok(oedvi_client::local::store(&self.settings.layout())?)
    }
    /// The builders: the server's in remote mode, the local files otherwise.
    async fn builder_list(&self) -> anyhow::Result<oedvi_common::BuilderList> {
        if !self.remote() {
            return Ok(self.local_store()?.resolved()?);
        }
        let state = self.state()?;
        match self.api()?.builders().await {
            Ok(l) => {
                state.save_builders(&l)?;
                Ok(l)
            }
            Err(e) => state.builders().with_context(|| format!("{e:#}")),
        }
    }
    fn state(&self) -> anyhow::Result<StateFiles> {
        let layout = self.settings.layout();
        layout.create_all()?;
        Ok(StateFiles::new(&layout))
    }
    async fn manifest(&self) -> anyhow::Result<oedvi_common::Manifest> {
        let state = self.state()?;
        match self.api()?.manifest().await {
            Ok(m) => {
                state.save_manifest(&m)?;
                Ok(m)
            }
            Err(e) => {
                let m = state.manifest().with_context(|| format!("{e:#}"))?;
                eprintln!("server unavailable ({e:#}), using the cached manifest v{}", m.version);
                Ok(m)
            }
        }
    }
    async fn builder(&self, id: &str) -> anyhow::Result<BuilderDef> {
        let list = self.builder_list().await?;
        if let Some(err) = list.errors.iter().find(|e| e.id == id) {
            bail!("builder {id} is invalid: {}", err.message);
        }
        list.builders.into_iter().find(|b| b.id == id).with_context(|| format!("unknown builder {id:?}"))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("OEDVI_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    oedvi_client::prereq::refresh_path();
    let path = cli.settings.unwrap_or_else(Settings::default_path);
    let settings = Settings::load(&path)?;
    let mut ctx = Ctx { settings, path };

    match cli.cmd {
        Command::Config { mode, server, token, root, game_dir, jobs, delete_extra, auto_install_tools } => {
            let s = &mut ctx.settings;
            let changed = mode.is_some()
                || server.is_some()
                || token.is_some()
                || root.is_some()
                || game_dir.is_some()
                || jobs.is_some()
                || delete_extra.is_some()
                || auto_install_tools.is_some();
            if let Some(v) = mode {
                s.mode = match v.as_str() {
                    "standalone" => Mode::Standalone,
                    "remote" => Mode::Remote,
                    other => bail!("mode must be `standalone` or `remote`, not {other:?}"),
                };
            }
            if let Some(v) = server {
                s.server_url = v
            }
            if let Some(v) = token {
                s.token = v
            }
            if let Some(v) = root {
                s.root_dir = v
            }
            if let Some(v) = game_dir {
                s.game_dir = Some(v)
            }
            if let Some(v) = jobs {
                s.download_jobs = v.max(1)
            }
            if let Some(v) = delete_extra {
                s.delete_extra = v
            }
            if let Some(v) = auto_install_tools {
                s.auto_install_tools = v
            }
            if changed {
                s.save(&ctx.path)?;
            }
            let mut shown = serde_json::to_value(&*s)?;
            if !s.token.is_empty() {
                shown["token"] = "********".into();
            }
            println!("{}\n{}", ctx.path.display(), serde_json::to_string_pretty(&shown)?);
            println!("folders: {}", serde_json::to_string_pretty(&s.layout())?);
        }

        Command::Status { files } => {
            let m = ctx.manifest().await?;
            let state = ctx.state()?;
            let st = sync::local_status(
                &m,
                &state.selection(),
                &state.index(),
                &ctx.settings.layout().game,
                ctx.settings.delete_extra,
            )?;
            let s = &st.summary;
            println!("manifest v{} — {} files, {}", s.manifest_version, s.files_total, gb(s.bytes_total));
            println!("selected: {} files, {}", s.selected_files, gb(s.selected_bytes));
            println!("up to date: {} files, {}", s.ok_files, gb(s.ok_bytes));
            println!(
                "missing {} · partial {} · outdated {} · to verify {} · to delete {} · extra {}",
                s.missing, s.partial, s.outdated, s.unverified, s.to_remove, s.extra
            );
            if let Some(free) = s.free_space {
                println!("at most {} to download, {} free", gb(s.download_max_bytes), gb(free));
            }
            println!("{}", if s.up_to_date { "✔ everything is up to date" } else { "→ run `oedvi sync`" });
            if files {
                for f in st.files.iter().filter(|f| !matches!(f.state, FileState::Ok | FileState::Unselected)) {
                    println!("  {:<11} {:>10}  {}", format!("{:?}", f.state), gb(f.size), f.path);
                }
            }
        }

        Command::Sync { verify, keep_extra, no_space_check } => {
            let m = ctx.manifest().await?;
            let state = ctx.state()?;
            let opts = SyncOptions {
                verify_all: verify,
                delete_extra: ctx.settings.delete_extra && !keep_extra,
                jobs: ctx.settings.download_jobs,
                check_space: !no_space_check,
            };
            let progress = Arc::new(Progress::default());
            let p = progress.clone();
            let ticker = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let s = p.snapshot();
                    let speed = s.downloaded_bytes as f64 / s.elapsed_secs.max(0.1);
                    eprint!(
                        "\r\x1b[2K[{}] files {}/{} · verified {} · downloaded {}/{} · {}/s",
                        s.phase,
                        s.files_done,
                        s.files_total,
                        gb(s.verified_bytes),
                        gb(s.downloaded_bytes),
                        gb(s.download_bytes_needed),
                        gb(speed as u64)
                    );
                }
            });
            let report = sync::sync(
                &ctx.api()?,
                &m,
                &state.selection(),
                &state,
                &ctx.settings.layout().game,
                &opts,
                progress,
                ctrl_c_token(),
            )
            .await;
            ticker.abort();
            eprintln!();
            let report = report?;
            println!(
                "{} files synced, {} downloaded, {} deleted{}",
                report.files_synced,
                gb(report.bytes_downloaded),
                report.removed,
                if report.cancelled { " (interrompu)" } else { "" }
            );
            for e in &report.errors {
                eprintln!("✘ {} : {}", e.path, e.message);
            }
            if !report.errors.is_empty() {
                bail!("{} files failed", report.errors.len());
            }
        }

        Command::Select { action } => {
            let state = ctx.state()?;
            let mut sel = state.selection();
            match action {
                SelectAction::Show => {
                    let m = ctx.manifest().await?;
                    for g in &m.groups {
                        let on = m.files.iter().filter(|f| f.group == g.name).all(|f| sel.is_selected(f));
                        let some = m.files.iter().filter(|f| f.group == g.name).any(|f| sel.is_selected(f));
                        let mark = if on {
                            "[x]"
                        } else if some {
                            "[~]"
                        } else {
                            "[ ]"
                        };
                        println!(
                            "{mark} {:<16} {:>10}  {}{}",
                            g.name,
                            gb(g.size),
                            g.label.clone().unwrap_or_default(),
                            if g.required { " (obligatoire)" } else { "" }
                        );
                    }
                    for (p, v) in &sel.paths {
                        println!("rule: {p} → {}", if *v { "keep" } else { "exclude" });
                    }
                    let (n, size) = sel.selected_size(&m);
                    println!("total: {n} files, {}", gb(size));
                    return Ok(());
                }
                SelectAction::All => sel.set_all(true),
                SelectAction::None => sel.set_all(false),
                SelectAction::Group { name, on } => {
                    let m = ctx.manifest().await?;
                    if !m.groups.iter().any(|g| g.name == name) {
                        bail!("unknown group {name:?}");
                    }
                    sel.set_group(&m, &name, on);
                }
                SelectAction::Path { prefix, on } => sel.set_path(&prefix, on),
            }
            state.save_selection(&sel)?;
            println!("selection saved; run `oedvi sync` to apply it");
        }

        Command::Builders => {
            let list = ctx.builder_list().await?;
            let layout = ctx.settings.layout();
            for b in &list.builders {
                let cloned = git::is_cloned(&builder::checkout_dir(&layout, b));
                println!(
                    "{:<24} {:<32} {:<7} {}@{} {}",
                    b.id,
                    b.name,
                    b.stack,
                    b.repo.rsplit('/').next().unwrap_or(&b.repo),
                    b.git_ref,
                    if cloned { "" } else { "(not cloned)" }
                );
            }
            for e in &list.errors {
                eprintln!("✘ {} : {}", e.id, e.message);
            }
        }

        Command::Git { id, action } => {
            let def = ctx.builder(&id).await?;
            let dir = builder::checkout_dir(&ctx.settings.layout(), &def);
            let cancel = ctrl_c_token();
            if action != "status" {
                builder::ensure_git(&def, ctx.settings.auto_install_tools, &sink(), &cancel).await?;
            }
            match action.as_str() {
                "fetch" | "pull" | "resync" if !git::is_cloned(&dir) => {
                    git::clone(&def.repo, &def.git_ref, &dir, &sink(), &cancel).await?;
                }
                "fetch" => git::fetch(&dir, &sink(), &cancel).await?,
                "pull" => {
                    git::ensure_remote(&dir, &def.repo, &sink(), &cancel).await?;
                    git::pull(&dir, &def.git_ref, &sink(), &cancel).await?
                }
                "resync" => {
                    git::ensure_remote(&dir, &def.repo, &sink(), &cancel).await?;
                    git::resync(&dir, &def.git_ref, &sink(), &cancel).await?
                }
                _ => {}
            }
            let st = git::status(&dir, &def.git_ref).await?;
            println!("{}", serde_json::to_string_pretty(&st)?);
        }

        Command::Tools { id, install } => {
            let def = ctx.builder(&id).await?;
            if install {
                let done = oedvi_client::prereq::install_missing(&def, &sink(), &ctrl_c_token()).await?;
                if done.is_empty() {
                    println!("nothing to install");
                }
            }
            for t in oedvi_client::prereq::check(&def) {
                println!(
                    "{} {:<34} {}",
                    if t.installed { "✔" } else { "✘" },
                    t.label,
                    t.found.unwrap_or_else(|| if t.installable {
                        "missing (--install)".into()
                    } else {
                        "manquant".into()
                    })
                );
            }
        }

        Command::Build { id, force } => {
            let def = ctx.builder(&id).await?;
            let p = builder::prepare(&def, &ctx.settings)?;
            let state = ctx.state()?;
            if !force && builder::build_state(&p, &state).await.up_to_date {
                println!("already built for this commit (--force to rebuild)");
                return Ok(());
            }
            builder::build(&p, &state, ctx.settings.auto_install_tools, sink(), &ctrl_c_token()).await?;
        }

        Command::Run { id, no_build } => {
            let def = ctx.builder(&id).await?;
            let p = builder::prepare(&def, &ctx.settings)?;
            let state = ctx.state()?;
            if !no_build && !builder::build_state(&p, &state).await.up_to_date {
                builder::build(&p, &state, ctx.settings.auto_install_tools, sink(), &ctrl_c_token()).await?;
            }
            let running = builder::launch(&p, sink())?;
            let pid = running.pid;
            let cancel = ctrl_c_token();
            let wait = running.wait();
            tokio::pin!(wait);
            tokio::select! {
                status = &mut wait => println!("the game stopped ({})", oedvi_client::process::describe(status?)),
                _ = cancel.cancelled() => {
                    oedvi_client::process::kill_tree(pid);
                    let _ = wait.await;
                }
            }
        }

        Command::Server { action } => {
            let api = ctx.api()?;
            if action == "rescan" {
                api.rescan().await?;
                println!("rescan requested");
            }
            println!("{}", serde_json::to_string_pretty(&api.status().await?)?);
        }

        Command::Edit { kind, id, put, delete } if !ctx.remote() => {
            let kind = oedvi_common::store::Kind::parse(&kind).context("kind must be `builders` or `templates`")?;
            let store = ctx.local_store()?;
            if delete {
                if !store.delete(kind, &id).map_err(anyhow::Error::msg)? {
                    bail!("{id} does not exist");
                }
                println!("deleted (a copy is kept in .trash)");
            } else if let Some(file) = put {
                let text = std::fs::read_to_string(&file)?;
                let list = store.write(kind, &id, &text).map_err(anyhow::Error::msg)?;
                println!("saved; {} valid builders", list.builders.len());
                for e in &list.errors {
                    eprintln!("✘ {}: {}", e.id, e.message);
                }
            } else {
                match store.read(kind, &id).map_err(anyhow::Error::msg)? {
                    Some(t) => print!("{t}"),
                    None => bail!("{id} does not exist"),
                }
            }
        }

        Command::Edit { kind, id, put, delete } => {
            let api = ctx.api()?;
            if delete {
                api.delete_raw(&kind, &id).await?;
                println!("deleted (a copy is kept in .trash on the server)");
            } else if let Some(file) = put {
                let text = std::fs::read_to_string(&file)?;
                let list = api.write_raw(&kind, &id, &text).await?;
                println!("saved; {} valid builders", list.builders.len());
                for e in &list.errors {
                    eprintln!("✘ {} : {}", e.id, e.message);
                }
            } else {
                match api.read_raw(&kind, &id).await? {
                    Some(t) => print!("{t}"),
                    None => bail!("{kind}/{id} does not exist"),
                }
            }
        }
    }
    Ok(())
}
