//! Performance runs reported by the launchers, kept in
//! `<data_dir>/stats/sessions.jsonl` (one JSON per line, append only).

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use oedvi_common::stats::{Session, StatsReport, aggregate, aggregate_phases};
use serde_json::json;

use crate::api::Shared;

/// Runs shown in `recent`.
const RECENT: usize = 60;
/// Runs kept in memory and answered from; older ones stay in the file.
const MAX_LOADED: usize = 20_000;

pub struct Stats {
    path: PathBuf,
    sessions: Mutex<Vec<Session>>,
}

impl Stats {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("stats").join("sessions.jsonl");
        let mut sessions: Vec<Session> = std::fs::read_to_string(&path)
            .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default();
        let skip = sessions.len().saturating_sub(MAX_LOADED);
        sessions.drain(..skip);
        if !sessions.is_empty() {
            tracing::info!("stats: {} performance runs loaded from {}", sessions.len(), path.display());
        }
        Self { path, sessions: Mutex::new(sessions) }
    }

    /// `false` when this run was already stored.
    fn add(&self, s: Session) -> std::io::Result<bool> {
        let mut all = self.sessions.lock().unwrap();
        if all.iter().any(|o| o.id == s.id) {
            return Ok(false);
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(&s).map_err(std::io::Error::other)?)?;
        all.push(s);
        Ok(true)
    }

    fn report(&self) -> StatsReport {
        let all = self.sessions.lock().unwrap();
        StatsReport {
            rows: aggregate(&all),
            phases: aggregate_phases(&all),
            recent: all.iter().rev().take(RECENT).cloned().collect(),
        }
    }
}

pub async fn report(State(state): State<Shared>) -> Json<StatsReport> {
    Json(state.stats.report())
}

pub async fn submit(State(state): State<Shared>, Json(s): Json<Session>) -> Response {
    if let Err(e) = s.validate() {
        tracing::warn!("stats: run refused: {e}");
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e }))).into_response();
    }
    let line = format!(
        "{} @ {} on {}: {:.1} fps avg, {:.1} low 1%, {} samples over {:.0} s",
        s.builder, s.commit, s.machine, s.avg, s.low1, s.samples, s.duration_secs
    );
    match state.stats.add(s) {
        Ok(true) => {
            tracing::info!("stats: {line}");
            StatusCode::CREATED.into_response()
        }
        Ok(false) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("stats: cannot store the run: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response()
        }
    }
}
