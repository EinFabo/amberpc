//! Updates from GitHub releases, checked at start and every six hours. With automatic
//! updates on, a new version is downloaded in the background and installed once no
//! dictation runs; the installer restarts Amber.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::state::Core;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

pub fn spawn(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Not in the first seconds after login, when everything starts at once.
        tokio_sleep(Duration::from_secs(45)).await;
        loop {
            if app.state::<Core>().settings().auto_update {
                if let Err(e) = install(&app, true).await {
                    crate::diag::warn(format!("update check failed: {e}"));
                }
            }
            tokio_sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone() }))
}

/// Downloads and installs a newer version if there is one. [quiet]: wait until no
/// dictation runs, and announce the restart.
pub async fn install(app: &AppHandle, quiet: bool) -> Result<bool, String> {
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    crate::diag::info(format!("update {} found, downloading", update.version));
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    if quiet {
        // Never in the middle of a dictation, and only after a quiet minute.
        let mut idle = 0;
        while idle < 12 {
            idle = if app.state::<Core>().dictation.is_active() { 0 } else { idle + 1 };
            tokio_sleep(Duration::from_secs(5)).await;
        }
        crate::notify(app, "Amber is updating", &format!("Version {} - Amber restarts in a moment.", update.version));
        tokio_sleep(Duration::from_secs(3)).await;
    }
    crate::diag::info(format!("installing update {}", update.version));
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
