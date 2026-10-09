#[path = "../tests/common/mod.rs"]
mod common;

use common::Harness;
use serde_json::json;
use sqlx::Row;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;

fn write_private_new(path: &Path, bytes: &[u8]) -> Result<(), ()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| ())?;
    file.write_all(bytes).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn run() -> Result<(), ()> {
    let session_path = std::env::var("SERVICE_STATUS_SESSION_HANDOFF").map_err(|_| ())?;
    let audit_path = std::env::var("SERVICE_STATUS_AUDIT_PROOF").map_err(|_| ())?;
    if session_path != "/qa-handoff/owner-session.json"
        || audit_path != "/qa-handoff/audit-proof.json"
    {
        return Err(());
    }

    let mut harness = Harness::new().await.ok_or(())?;
    // The Owner route gate is required to render the independent panel; this
    // helper leaves paper, price, signals, and market-stream product modes off.
    harness.restart_api_with_owner_beta_access().await;
    let owner_id = harness.owner.user_id;
    let session = json!({
        "cookie_name": "__Host-lagrange_session",
        "cookie_value": harness.owner.cookie_value.clone(),
        "expires_at_secs": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ())?
            .as_secs() as i64 + 3_600,
    });
    write_private_new(Path::new(&session_path), format!("{session}\n").as_bytes())?;

    let listener = TcpListener::bind("0.0.0.0:8080").await.map_err(|_| ())?;
    let app = harness.app.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|_| ())?;

    let audit_reads: i64 = sqlx::query(
        "SELECT COUNT(*)::bigint AS read_count \
         FROM public.audit_logs \
         WHERE actor_user_id = $1 AND action = 'admin.service_status.read'",
    )
    .bind(owner_id)
    .fetch_one(&harness.admin_pool)
    .await
    .map_err(|_| ())?
    .try_get("read_count")
    .map_err(|_| ())?;
    write_private_new(
        Path::new(&audit_path),
        format!("{}\n", json!({ "owner_status_reads": audit_reads })).as_bytes(),
    )?;
    harness.teardown().await;
    Ok(())
}

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        std::process::exit(1);
    }
}
