//! Keeps the local items in step with the configured connectors.

use crate::{State, connectors, error::Result, items, rules};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// Sent to the interface whenever stored items may have changed.
pub const ITEMS_CHANGED: &str = "items-changed";

const PERIOD: Duration = Duration::from_secs(5 * 60);

/// A source that could not be synced, and why. The others still are.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub source: String,
    pub message: String,
}

/// Syncs every configured connector once, and drops the items of sources that are gone.
pub async fn run(app: &AppHandle) -> Result<Vec<Failure>> {
    let state = app.state::<State>();
    let connectors = connectors::configured(&state.db())?;
    let sources: Vec<&str> = connectors.iter().map(|c| c.source()).collect();
    items::retain_sources(&state.db(), &sources)?;
    rules::retain_sources(&state.db(), &sources)?;

    let mut failures = Vec::new();
    let mut notices = Vec::new();
    for connector in &connectors {
        // The database is never locked while a connector waits on the network.
        match connector.sync().await {
            Ok(fetched) => {
                let mut db = state.db();
                let fresh = items::replace_source(&mut db, connector.source(), &fetched)?;
                notices.extend(rules::arrived(&db, connector.source(), &fresh)?);
            }
            Err(message) => failures.push(Failure {
                source: connector.source().to_owned(),
                message,
            }),
        }
    }
    // Remembered so the settings can show which source is in trouble, and why.
    *state.sync_errors() = failures
        .iter()
        .map(|failure| (failure.source.clone(), failure.message.clone()))
        .collect();
    app.emit(ITEMS_CHANGED, ())?;
    for notice in notices {
        // A system that shows no notifications is no reason to fail the sync.
        let shown = app
            .notification()
            .builder()
            .title(notice.title)
            .body(notice.body)
            .show();
        if let Err(error) = shown {
            eprintln!("notification failed: {error}");
        }
    }
    Ok(failures)
}

/// Syncs now, then every few minutes for as long as the app runs.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(PERIOD);
        loop {
            ticks.tick().await;
            match run(&app).await {
                Ok(failures) => {
                    for failure in failures {
                        eprintln!("sync of {} failed: {}", failure.source, failure.message);
                    }
                }
                Err(error) => eprintln!("sync failed: {error}"),
            }
        }
    });
}
