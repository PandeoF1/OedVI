//! Self-update of the launcher.
//!
//! Put the Windows installer built by `tauri build` and its signature in
//! `launcher_dir` (by hand, or `PUT /api/launcher/files/<name>` from CI):
//!
//! ```text
//! oedvi_0.2.0_x64-setup.exe
//! oedvi_0.2.0_x64-setup.exe.sig
//! oedvi_0.2.0.notes.md          (optional: release notes)
//! ```
//!
//! `GET /api/launcher/latest.json` answers in the format of Tauri's updater
//! with the newest signed installer; the launcher checks the signature with
//! the public key built into it before installing.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::api::Shared;

/// `(version, installer file name)` of each signed installer.
fn installers(dir: &Path) -> Vec<(Vec<u64>, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix("_x64-setup.exe") else { continue };
        if !dir.join(format!("{name}.sig")).exists() {
            continue;
        }
        // `<product>_<version>`
        let Some((_, version)) = stem.rsplit_once('_') else { continue };
        let parts: Option<Vec<u64>> = version.split(['.', '-']).take(3).map(|p| p.parse().ok()).collect();
        if let Some(v) = parts.filter(|v| v.len() == 3) {
            out.push((v, name));
        }
    }
    out.sort();
    out
}

/// `0.1.2 (oedvi_0.1.2_x64-setup.exe)` for the startup log.
pub fn describe_newest(dir: &Path) -> Option<String> {
    let (version, name) = installers(dir).pop()?;
    Some(format!("{} ({name})", version.iter().map(u64::to_string).collect::<Vec<_>>().join(".")))
}

fn base_url(state: &Shared, headers: &HeaderMap) -> String {
    if let Some(u) = &state.cfg.public_url {
        return u.trim_end_matches('/').to_string();
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    let proto = headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()).unwrap_or("http");
    format!("{proto}://{host}")
}

pub async fn latest(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let dir = state.cfg.launcher_dir();
    let Some((version, name)) = installers(&dir).pop() else {
        // The updater reads 204 as "no update".
        return StatusCode::NO_CONTENT.into_response();
    };
    let version = version.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
    let signature = std::fs::read_to_string(dir.join(format!("{name}.sig"))).unwrap_or_default();
    let notes = std::fs::read_to_string(dir.join(format!("{}.notes.md", name.trim_end_matches("_x64-setup.exe"))))
        .unwrap_or_default();
    let pub_date = std::fs::metadata(dir.join(&name))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let url = format!("{}/launcher/{}", base_url(&state, &headers), name);
    Json(json!({
        "version": version,
        "notes": notes,
        "pub_date": rfc3339(pub_date),
        "platforms": { "windows-x86_64": { "signature": signature.trim(), "url": url } }
    }))
    .into_response()
}

/// `PUT /api/launcher/files/<name>`: upload from CI (admin token).
pub async fn upload(State(state): State<Shared>, UrlPath(name): UrlPath<String>, body: Body) -> Response {
    let ok_name = !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        && (name.ends_with(".exe") || name.ends_with(".sig") || name.ends_with(".md"));
    if !ok_name {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "file name refused" }))).into_response();
    }
    let dir: PathBuf = state.cfg.launcher_dir();
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response();
    }
    let tmp = dir.join(format!(".{name}.upload"));
    let result: anyhow::Result<u64> = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut stream = body.into_data_stream();
        let mut n = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            n += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.sync_all().await?;
        tokio::fs::rename(&tmp, dir.join(&name)).await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(n) => {
            tracing::info!("launcher file {name} uploaded ({n} bytes)");
            (StatusCode::CREATED, Json(json!({ "name": name, "size": n }))).into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response()
        }
    }
}

fn rfc3339(secs: u64) -> String {
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_newest_signed_installer() {
        let dir = tempfile::tempdir().unwrap();
        for f in [
            "oedvi_0.1.0_x64-setup.exe",
            "oedvi_0.1.0_x64-setup.exe.sig",
            "oedvi_0.10.0_x64-setup.exe",
            "oedvi_0.10.0_x64-setup.exe.sig",
            "oedvi_0.9.0_x64-setup.exe",
            "oedvi_1.0.0_x64-setup.exe", // unsigned: ignored
        ] {
            std::fs::write(dir.path().join(f), "x").unwrap();
        }
        assert_eq!(installers(dir.path()).pop().unwrap().1, "oedvi_0.10.0_x64-setup.exe");
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_795_612_775), "2026-11-25T13:19:35Z");
    }
}
