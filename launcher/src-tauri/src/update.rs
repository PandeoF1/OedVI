//! Self-update: from the GitHub releases (standalone) or from oedvi-server
//! (`GET /api/launcher/latest.json`, remote mode).
//!
//! The installer's signature is checked against the public key compiled in
//! (`OEDVI_UPDATER_PUBKEY` at build time, the output of `tauri signer
//! generate`). A build without that key cannot update itself.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::AppState;

pub const PUBKEY: Option<&str> = option_env!("OEDVI_UPDATER_PUBKEY");
/// Where a standalone launcher looks for updates: the `latest.json` of the
/// GitHub releases (set by the release workflow). In remote mode the server
/// is asked instead.
pub const UPDATE_URL: Option<&str> = option_env!("OEDVI_UPDATE_URL");

#[derive(Default)]
pub struct Pending(pub Mutex<Option<Update>>);

#[derive(Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub available: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
    /// Why updates are off, when they are.
    pub disabled: Option<String>,
    /// Why the last check failed.
    pub error: Option<String>,
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let Some(pubkey) = PUBKEY.filter(|k| !k.is_empty()) else {
        return Ok(UpdateInfo {
            current,
            available: None,
            notes: None,
            date: None,
            disabled: Some("this build has no signing key: automatic updates are off".into()),
            error: None,
        });
    };
    if !state.remote() && UPDATE_URL.is_none_or(str::is_empty) {
        return Ok(UpdateInfo {
            current,
            available: None,
            notes: None,
            date: None,
            disabled: Some("this build has no update source".into()),
            error: None,
        });
    }
    match find(&app, &state, pubkey).await {
        Ok(update) => {
            let info = UpdateInfo {
                current,
                available: update.as_ref().map(|u| u.version.clone()),
                notes: update.as_ref().and_then(|u| u.body.clone()),
                date: update.as_ref().and_then(|u| u.date.map(|d| d.to_string())),
                disabled: None,
                error: None,
            };
            *pending.0.lock().unwrap() = update;
            Ok(info)
        }
        Err(error) => {
            Ok(UpdateInfo { current, available: None, notes: None, date: None, disabled: None, error: Some(error) })
        }
    }
}

async fn find(app: &AppHandle, state: &AppState, pubkey: &str) -> Result<Option<Update>, String> {
    let s = state.settings();
    let endpoint = if state.remote() {
        format!("{}/api/launcher/latest.json", s.server_url.trim_end_matches('/'))
    } else {
        UPDATE_URL.unwrap_or_default().to_string()
    };
    let mut builder = app
        .updater_builder()
        .pubkey(pubkey)
        .endpoints(vec![endpoint.parse().map_err(|e| format!("{e}"))?])
        .map_err(|e| e.to_string())?;
    if state.remote() && !s.token.is_empty() {
        builder = builder.header("Authorization", format!("Bearer {}", s.token)).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// Downloads, checks and installs the update found by `check_update`, then
/// restarts (on Windows the installer closes the app itself).
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
) -> Result<(), String> {
    if state.sync.lock().unwrap().is_some() {
        return Err("wait for the sync to finish".into());
    }
    let update = pending.0.lock().unwrap().take().ok_or("no update is pending")?;
    let mut downloaded = 0u64;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update-progress", UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}
