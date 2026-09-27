//! `sen-turbo-fieldfare` — TurboFieldfare LLM runtime for SenClaw.
//!
//! `sen-turbo-fieldfare serve --host {host} --port {port} --model {model_path}`
//! loads one completed `.gturbo` directory and serves it as an OpenAI-compatible
//! provider. The Metal engine is `TurboFieldfareServer` (Swift), spawned beside
//! this binary. This process adds the runtime protocol: `/health` is 503 while
//! the engine maps weights, bearer auth on every other route, and
//! `/runtime/shutdown`.
//!
//! Contract: `senclaw/docs/runtime-protocol.md` §4.2.

mod engine;
mod proxy;

use std::sync::Arc;

use sen_runtime_sdk::env::LaunchEnv;
use sen_runtime_sdk::manifest::{Capability, RunMode};
use sen_runtime_sdk::server::{serve, Readiness, ServeArgs, ServeOptions};

use engine::{engine_binary, max_context_from_rest, snap_max_context, EngineHandle};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sen_runtime_sdk::server::init_tracing();

    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("serve") => {
            argv.remove(0);
        }
        Some(other) if !other.starts_with('-') => {
            anyhow::bail!("unknown subcommand `{other}` (sen-turbo-fieldfare has only `serve`)");
        }
        _ => {}
    }
    let parsed = ServeArgs::parse(argv).map_err(|e| anyhow::anyhow!(e))?;

    let env = LaunchEnv::from_env("sen-turbo-fieldfare", env!("CARGO_PKG_VERSION"));
    let model_path = parsed
        .model
        .clone()
        .or_else(|| env.model_path.clone())
        .ok_or_else(|| anyhow::anyhow!("no model: pass --model <path> or set SENCLAW_MODEL_PATH"))?;
    if !model_path.is_dir() {
        anyhow::bail!("--model {} is not a directory", model_path.display());
    }
    let model_id = env
        .model_id
        .clone()
        .or_else(|| model_path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .ok_or_else(|| anyhow::anyhow!("could not derive a model id from {}", model_path.display()))?;
    let engine_bin = engine_binary().ok_or_else(|| {
        anyhow::anyhow!(
            "TurboFieldfareServer was not found next to this binary. \
             Set SEN_TURBO_FIELDARE_ENGINE to the Swift executable, or install a packaged build."
        )
    })?;
    let max_context = snap_max_context(max_context_from_rest(&parsed.rest).unwrap_or(32_768));

    let readiness = Readiness::loading();
    let handle = EngineHandle::spawn(&engine_bin, &model_path, &model_id, max_context, readiness.clone())?;
    let info_base = handle.base.clone();
    let info_model = model_id.clone();
    let routes = proxy::router(handle, readiness.clone());

    let result = serve(
        routes,
        ServeOptions {
            env,
            mode: RunMode::Model,
            capabilities: vec![Capability::Chat, Capability::Vision],
            readiness,
            info_detail: Some(Arc::new(move || {
                serde_json::json!({
                    "model": info_model,
                    "engine": info_base,
                    "maxContext": max_context,
                })
            })),
            args: parsed,
        },
    )
    .await;

    // `EngineHandle` is moved into the proxy. Process exit drops the child
    // via `kill_on_drop` only if the supervise task still owns it; the task
    // exits with us. A leftover engine is reaped because it is a child of
    // this process and `kill_on_drop` is set at spawn — the supervise task
    // is aborted when the runtime ends.
    result?;
    Ok(())
}
