//! Rescans when the game directory changes, once it has been quiet for a while
//! (copying 250 GB produces events for a long time), and on a timer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::config::Config;
use crate::scanner::Scanner;

pub fn spawn(cfg: Arc<Config>, scanner: Arc<Scanner>) {
    if cfg.rescan_interval_mins > 0 {
        let scanner = scanner.clone();
        let every = Duration::from_secs(cfg.rescan_interval_mins * 60);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.tick().await;
            loop {
                tick.tick().await;
                tracing::debug!("periodic rescan");
                scanner.request_rescan();
            }
        });
    }

    if !cfg.watch {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(ev) if !ev.kind.is_access() => {
            let _ = tx.send(());
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("file watcher: {e}"),
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("cannot watch the game directory ({e}); set rescan_interval_mins or POST /api/rescan");
            return;
        }
    };
    if let Err(e) = watcher.watch(&cfg.game_dir, RecursiveMode::Recursive) {
        tracing::warn!("cannot watch {} ({e}); set rescan_interval_mins or POST /api/rescan", cfg.game_dir.display());
        return;
    }
    tracing::info!("watching {} for changes", cfg.game_dir.display());

    let debounce = Duration::from_secs(cfg.watch_debounce_secs.max(1));
    tokio::spawn(async move {
        let _watcher = watcher; // lives as long as this task
        let mut last_event: Option<Instant> = None;
        loop {
            let wait = match last_event {
                Some(t) => debounce.saturating_sub(t.elapsed()),
                None => Duration::from_secs(3600),
            };
            tokio::select! {
                ev = rx.recv() => {
                    if ev.is_none() { return; }
                    last_event = Some(Instant::now());
                }
                _ = tokio::time::sleep(wait) => {
                    if last_event.take().is_some() {
                        tracing::info!("game directory changed; rescanning");
                        scanner.request_rescan();
                    }
                }
            }
        }
    });
}
