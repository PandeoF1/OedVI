//! oedvi-server: publishes the game files, their manifest and the builders.

mod api;
mod config;
mod launcher;
mod scanner;
mod stats;
mod store;
mod telemetry;
mod watch;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Publishes game files and builder definitions to the OedVI launcher")]
struct Args {
    /// Configuration file.
    #[arg(short, long, env = "OEDVI_CONFIG", default_value = "oedvi-server.toml")]
    config: PathBuf,
    /// Hash everything, publish the manifest and exit (no HTTP server).
    #[arg(long)]
    scan_only: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into()),
        )
        .with_target(false)
        // `docker logs` and files do not interpret terminal colors.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let args = Args::parse();
    let cfg = Arc::new(config::Config::load(&args.config)?);
    tracing::info!("OedVI server {} starting", env!("CARGO_PKG_VERSION"));
    tracing::info!("game files: {}", cfg.game_dir.display());
    tracing::info!(
        "data: {} (builders {}, templates {})",
        cfg.data_dir.display(),
        cfg.builders_dir().display(),
        cfg.templates_dir().display()
    );
    tracing::info!(
        "hashing: {} MiB chunks, {} job(s); rescan: watch={} (quiet {}s), periodic={}",
        cfg.chunk_size_mib,
        cfg.hash_jobs,
        cfg.watch,
        cfg.watch_debounce_secs,
        if cfg.rescan_interval_mins == 0 {
            "off".to_string()
        } else {
            format!("every {} min", cfg.rescan_interval_mins)
        }
    );
    let groups: Vec<String> =
        cfg.groups.iter().map(|g| format!("{}{}", g.name, if g.required { "*" } else { "" })).collect();
    tracing::info!("groups ({} default, * = required): {}", cfg.default_group, groups.join(", "));
    match (cfg.token.is_empty(), cfg.admin_token.is_empty()) {
        (true, _) => {
            tracing::warn!("no token set: anyone who reaches this server can download the files and edit builders")
        }
        (false, true) => tracing::warn!("read token set but no admin token: the read token also allows edits"),
        (false, false) => tracing::info!("auth: read token and admin token set"),
    }
    match launcher::describe_newest(&cfg.launcher_dir()) {
        Some(v) => tracing::info!("launcher update served: {v}"),
        None => tracing::info!("launcher update: no signed installer in {}", cfg.launcher_dir().display()),
    }

    let scanner = scanner::Scanner::new(cfg.clone());
    if args.scan_only {
        let s = scanner.clone();
        let task = tokio::spawn(s.run());
        // `run` scans once at start; wait for that scan to finish.
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let st = scanner.status();
            if !st.scanning && st.last_scan_at.is_some() {
                scanner.stop();
                let _ = task.await;
                return match st.last_error {
                    Some(e) => Err(anyhow::anyhow!(e)),
                    None => Ok(()),
                };
            }
        }
    }

    let store = store::Store::new(cfg.builders_dir(), cfg.templates_dir())?;
    tokio::spawn(scanner.clone().run());
    watch::spawn(cfg.clone(), scanner.clone());

    let state = Arc::new(api::AppState::new(cfg.clone(), scanner.clone(), store));
    match state.store.resolved() {
        Ok(list) => {
            if list.builders.is_empty() && list.errors.is_empty() {
                tracing::info!("builders: none yet (add .toml files to {})", cfg.builders_dir().display());
            }
            state.note_builders(&list);
        }
        Err(e) => tracing::warn!("cannot read the builders: {e:#}"),
    }
    telemetry::spawn_download_summaries(state.activity.clone());
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await.with_context(|| format!("binding {}", cfg.bind))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("shutting down");
            scanner.stop();
        })
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
