use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;

use crate::config::Config;
use crate::scanner::Scanner;
use crate::store::{Kind, Store};
use crate::telemetry::{Activity, Role};

pub struct AppState {
    pub cfg: Arc<Config>,
    pub scanner: Arc<Scanner>,
    pub store: Store,
    pub activity: Arc<Activity>,
    pub stats: crate::stats::Stats,
    /// What the builder list looked like when it was last logged.
    builders_logged: Mutex<String>,
}

impl AppState {
    pub fn new(cfg: Arc<Config>, scanner: Arc<Scanner>, store: Store) -> Self {
        let stats = crate::stats::Stats::load(&cfg.data_dir);
        Self { cfg, scanner, store, activity: Arc::default(), stats, builders_logged: Mutex::default() }
    }

    /// Logs the builder list when it differs from the last one logged, with
    /// the reason for each builder that failed to load.
    pub fn note_builders(&self, list: &oedvi_common::builder::BuilderList) {
        let ids: Vec<&str> = list.builders.iter().map(|b| b.id.as_str()).collect();
        let mut summary = ids.join(", ");
        for e in &list.errors {
            summary.push_str(&format!("\n{}: {}", e.id, e.message));
        }
        let mut last = self.builders_logged.lock().unwrap();
        if *last == summary {
            return;
        }
        *last = summary;
        tracing::info!("builders: {} loaded ({})", ids.len(), ids.join(", "));
        for e in &list.errors {
            tracing::warn!("builder {} is broken and hidden: {}", e.id, e.message);
        }
    }
}

pub type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    let read = Router::new()
        .route("/api/status", get(status))
        .route("/api/manifest", get(manifest))
        .route("/api/builders", get(builders))
        // `/api/builders` is the resolved list, so the file names of a
        // collection (broken ones included) have their own route.
        .route("/api/ids/{kind}", get(list_ids))
        .route("/api/{kind}", get(list_ids))
        .route("/api/{kind}/{id}", get(read_file).put(write_file).delete(delete_file))
        .route("/api/{kind}/{id}/check", post(check_file))
        .route("/api/rescan", post(rescan))
        .route("/api/stats", get(crate::stats::report).post(crate::stats::submit))
        .route("/api/launcher/latest.json", get(crate::launcher::latest))
        .layer(CompressionLayer::new())
        .route(
            "/api/launcher/files/{name}",
            axum::routing::put(crate::launcher::upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .nest_service("/launcher", ServeDir::new(state.cfg.launcher_dir()))
        .nest_service("/files", ServeDir::new(&state.cfg.game_dir).append_index_html_on_directories(false))
        .layer(middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/api/health", get(health))
        .merge(read)
        .layer(middleware::from_fn_with_state(state.clone(), crate::telemetry::access))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "name": "oedvi", "api": oedvi_common::API_VERSION, "version": env!("CARGO_PKG_VERSION") }))
}

/// Bearer token check. Writes (anything but GET/HEAD) need the admin token
/// when one is set. The role that was granted (or why not) travels on the
/// response for the access log.
async fn auth(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let cfg = &state.cfg;
    // Players report their FPS with the read token.
    let reporting = *req.method() == axum::http::Method::POST && req.uri().path() == "/api/stats";
    let writing = !reporting && !matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let admin = if cfg.admin_token.is_empty() { &cfg.token } else { &cfg.admin_token };
    let is_admin = !admin.is_empty() && ct_eq(given, admin);
    let is_reader = !cfg.token.is_empty() && ct_eq(given, &cfg.token);
    let role = if writing {
        if admin.is_empty() {
            Ok("open")
        } else if is_admin {
            Ok("admin")
        } else if is_reader {
            Err("denied:read token used for a write")
        } else {
            Err(if given.is_empty() { "denied:no token" } else { "denied:wrong token" })
        }
    } else if cfg.token.is_empty() {
        Ok("open")
    } else if is_reader {
        Ok("read")
    } else if is_admin {
        Ok("admin")
    } else {
        Err(if given.is_empty() { "denied:no token" } else { "denied:wrong token" })
    };
    let mut res = match role {
        Ok(_) => next.run(req).await,
        Err(_) => (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid or missing token" }))).into_response(),
    };
    res.extensions_mut().insert(Role(role.unwrap_or_else(|denied| denied)));
    res
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn error(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

async fn status(State(state): State<Shared>) -> Json<oedvi_common::ScanStatus> {
    Json(state.scanner.status())
}

async fn manifest(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let Some(published) = state.scanner.published() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "first scan still running, see /api/status");
    };
    let etag = format!("\"m{}\"", published.manifest.version);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ETAG, HeaderValue::from_str(&etag).unwrap())
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(published.json.clone()))
        .unwrap()
}

async fn rescan(State(state): State<Shared>) -> StatusCode {
    tracing::info!("rescan requested");
    state.scanner.request_rescan();
    StatusCode::ACCEPTED
}

async fn builders(State(state): State<Shared>) -> Response {
    match state.store.resolved() {
        Ok(list) => {
            state.note_builders(&list);
            Json(list).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[allow(clippy::result_large_err)]
fn kind(s: &str) -> Result<Kind, Response> {
    Kind::parse(s).ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown collection"))
}

async fn list_ids(State(state): State<Shared>, Path(k): Path<String>) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match state.store.read_all(k) {
        Ok(all) => Json(all.into_keys().collect::<Vec<_>>()).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn read_file(State(state): State<Shared>, Path((k, id)): Path<(String, String)>) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match state.store.read(k, &id) {
        Ok(Some(text)) => ([(header::CONTENT_TYPE, "application/toml; charset=utf-8")], text).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn write_file(State(state): State<Shared>, Path((k, id)): Path<(String, String)>, body: String) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match state.store.write(k, &id, &body) {
        Ok(list) => {
            tracing::info!("{} {id:?} saved ({} bytes)", k.as_str(), body.len());
            state.note_builders(&list);
            Json(list).into_response()
        }
        Err(e) => {
            tracing::warn!("{} {id:?} refused: {e}", k.as_str());
            error(StatusCode::UNPROCESSABLE_ENTITY, e)
        }
    }
}

async fn check_file(State(state): State<Shared>, Path((k, id)): Path<(String, String)>, body: String) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match state.store.check(k, &id, &body) {
        Ok(list) => Json(list).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

async fn delete_file(State(state): State<Shared>, Path((k, id)): Path<(String, String)>) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match state.store.delete(k, &id) {
        Ok(true) => {
            tracing::info!("{} {id:?} deleted (kept in .trash)", k.as_str());
            if let Ok(list) = state.store.resolved() {
                state.note_builders(&list);
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}
