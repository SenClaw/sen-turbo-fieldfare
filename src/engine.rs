//! Spawn `TurboFieldfareServer` and wait until its own `/health` answers.
//!
//! The engine binds only after the `.gturbo` weights are mapped, and it does
//! not speak the SenClaw runtime protocol (bearer token, `/runtime/shutdown`,
//! 503-while-loading). This process owns those. The engine listens on a
//! second loopback port and is reached only through the proxy.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sen_runtime_sdk::server::Readiness;
use tokio::process::{Child, Command};

/// Context lengths `TurboFieldfareServer --max-context` accepts.
const ALLOWED_CONTEXT: [u32; 5] = [4_096, 8_192, 16_384, 32_768, 65_536];

/// Snap a daemon context length onto a value the engine will start with.
/// Exact matches pass through. Anything else moves up to the next allowed
/// size, and past the top of the list stays at 65536.
pub fn snap_max_context(requested: u32) -> u32 {
    if ALLOWED_CONTEXT.contains(&requested) {
        return requested;
    }
    ALLOWED_CONTEXT.into_iter().find(|n| *n >= requested).unwrap_or(65_536)
}

/// `--max-context N` or `--max-context=N` left in [`sen_runtime_sdk::server::ServeArgs::rest`].
pub fn max_context_from_rest(rest: &[String]) -> Option<u32> {
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        if let Some(v) = arg.strip_prefix("--max-context=") {
            return v.parse().ok();
        }
        if arg == "--max-context" {
            return rest.get(i + 1).and_then(|v| v.parse().ok());
        }
        i += 1;
    }
    None
}

/// The engine binary packaged beside this one, or `SEN_TURBO_FIELFARE_ENGINE`
/// when developing against a Swift build tree.
pub fn engine_binary() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("SEN_TURBO_FIELDARE_ENGINE") {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let sibling = exe.parent()?.join("TurboFieldfareServer");
    sibling.is_file().then_some(sibling)
}

pub struct EngineHandle {
    pub base: String,
    /// Keeps the supervise task's child alive for as long as the proxy is.
    /// Dropped when the HTTP server shuts down; `kill_on_drop` then reaps
    /// the engine if the task has already stored it here.
    _child: Arc<Mutex<Option<Child>>>,
}

impl EngineHandle {
    /// Start the engine and return as soon as the process is spawned. Readiness
    /// flips to ready when `GET {base}/health` succeeds, or to failed if the
    /// process exits first.
    pub fn spawn(
        bin: &Path,
        model: &Path,
        model_id: &str,
        max_context: u32,
        readiness: Readiness,
    ) -> anyhow::Result<EngineHandle> {
        let port = free_port()?;
        let base = format!("http://127.0.0.1:{port}");
        let child = Arc::new(Mutex::new(None));
        let child_slot = Arc::clone(&child);
        let bin = bin.to_path_buf();
        let model = model.to_path_buf();
        let model_id = model_id.to_string();
        let base_for_task = base.clone();
        tokio::spawn(async move {
            supervise(bin, model, model_id, max_context, port, base_for_task, readiness, child_slot).await;
        });
        Ok(EngineHandle { _child: child, base })
    }
}

async fn supervise(
    bin: PathBuf,
    model: PathBuf,
    model_id: String,
    max_context: u32,
    port: u16,
    base: String,
    readiness: Readiness,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    let mut cmd = Command::new(&bin);
    cmd.arg("--model")
        .arg(&model)
        .arg("--port")
        .arg(port.to_string())
        .arg("--model-id")
        .arg(&model_id)
        .arg("--max-context")
        .arg(max_context.to_string())
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(dir) = bin.parent() {
        cmd.current_dir(dir);
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::error!("could not start {}: {e}", bin.display());
            readiness.set_failed();
            return;
        }
    };
    tracing::info!(
        "TurboFieldfareServer pid={} port={port} model={} context={max_context}",
        child.id().unwrap_or(0),
        model.display()
    );
    *child_slot.lock().unwrap() = Some(child);

    let client = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("health client: {e}");
            readiness.set_failed();
            return;
        }
    };
    let health = format!("{base}/health");
    loop {
        {
            let mut guard = child_slot.lock().unwrap();
            let exited = guard.as_mut().and_then(|c| c.try_wait().ok()).flatten();
            if let Some(status) = exited {
                drop(guard);
                tracing::error!("TurboFieldfareServer exited before it was ready ({status})");
                readiness.set_failed();
                return;
            }
        }
        if let Ok(resp) = client.get(&health).send().await {
            if resp.status().is_success() {
                tracing::info!("TurboFieldfareServer ready at {base}");
                readiness.set_ready();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Stay alive with the engine. If it dies after the daemon has already
    // accepted this process, exit so the supervisor's crash check respawns
    // instead of proxying to a closed port.
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut guard = child_slot.lock().unwrap();
        let exited = guard.as_mut().and_then(|c| c.try_wait().ok()).flatten();
        if let Some(status) = exited {
            tracing::error!("TurboFieldfareServer exited ({status}); stopping the runtime");
            drop(guard);
            std::process::exit(1);
        }
    }
}

fn free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_snaps_onto_the_engine_allow_list() {
        assert_eq!(snap_max_context(32_768), 32_768);
        assert_eq!(snap_max_context(2_048), 4_096);
        assert_eq!(snap_max_context(20_000), 32_768);
        assert_eq!(snap_max_context(200_000), 65_536);
    }

    #[test]
    fn max_context_is_read_from_either_flag_spelling() {
        assert_eq!(max_context_from_rest(&["--max-context".into(), "8192".into()]), Some(8192));
        assert_eq!(max_context_from_rest(&["--max-context=16384".into()]), Some(16384));
        assert_eq!(max_context_from_rest(&["--other".into()]), None);
    }
}
