//! `cirrus qs-manager` — start a cirrus-qs server.

use std::sync::Arc;

use cirrus_backend_soft::{SoftDetector, SoftMotor};
use cirrus_core::msg::{MovableObj, ReadableObj};
use cirrus_qs::{Registry, Server};
use clap::Args;
use tokio::sync::Mutex as TMutex;

use crate::manager_lua::ManagerLuaState;

/// Arguments for `cirrus qs-manager`.
#[derive(Args, Debug)]
pub struct ManagerArgs {
    /// Control REP socket address. Plans and engine commands flow here.
    #[arg(long, default_value = "tcp://*:60615")]
    control: String,

    /// Document PUB socket address. Bluesky `RemoteDispatcher` connects
    /// here to receive `RunStart` / `EventDescriptor` / `Event` /
    /// `RunStop` documents.
    #[arg(long, default_value = "tcp://*:60625")]
    documents: String,

    /// Register `n` `SoftDetector` instances named `det1`, `det2`, …
    /// Useful for trying out the queueserver workflow without bringing
    /// a real registry. Set to 0 to register none.
    #[arg(long, default_value_t = 1)]
    soft_detectors: usize,

    /// Register `n` `SoftMotor` instances named `m1`, `m2`, … Set to 0
    /// to skip.
    #[arg(long, default_value_t = 1)]
    soft_motors: usize,

    /// Optional Prometheus `/metrics` HTTP listener address (e.g.
    /// `127.0.0.1:9090`). Requires cirrus-qs built with the
    /// `metrics` feature; otherwise this flag logs a warning and
    /// is ignored.
    #[arg(long)]
    metrics: Option<String>,

    /// Optional path to a permissions.toml file that gates JSON-RPC
    /// methods by user group. Without this flag, the server runs
    /// permissive — every method is allowed for every caller.
    /// Callers identify themselves by `params.api_key`.
    #[arg(long)]
    permissions: Option<std::path::PathBuf>,
}

/// Entry point — returns a process exit code.
pub async fn run(args: ManagerArgs) -> i32 {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    let mut reg = Registry::new();
    for i in 1..=args.soft_detectors {
        let name = format!("det{i}");
        let det = SoftDetector::new(&name);
        reg.register_readable(&name, det as Arc<dyn ReadableObj>);
    }
    for i in 1..=args.soft_motors {
        let name = format!("m{i}");
        let motor = Arc::new(SoftMotor::new(&name, Some(0.0)));
        reg.register_readable(&name, motor.clone() as Arc<dyn ReadableObj>);
        reg.register_movable(&name, motor as Arc<dyn MovableObj>);
    }
    reg.register_plan_count("count");

    // Share the engine slot + registry between Server and the
    // daemon-side Lua bridge so `lua_eval` resolves the same `RE`
    // and the same registered devices the queue worker sees.
    let engine_slot = Arc::new(TMutex::new(None));
    let registry_for_lua = Arc::new(reg.clone());
    let evaluator: Arc<dyn cirrus_qs::LuaEvaluator> =
        Arc::new(ManagerLuaState::new(engine_slot.clone(), registry_for_lua));

    let mut sb = Server::builder()
        .control_address(&args.control)
        .document_address(&args.documents)
        .registry(reg)
        .engine_slot(engine_slot)
        .lua_evaluator(evaluator);
    if let Some(addr) = &args.metrics {
        sb = sb.metrics_address(addr);
    }
    if let Some(path) = &args.permissions {
        sb = sb.permissions_path(path.clone());
    }
    let server = match sb.build() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cirrus qs-manager: bind failed: {e}");
            return 2;
        }
    };
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\ncirrus qs-manager: shutting down");
            shutdown.shutdown();
        }
    });

    println!(
        "cirrus qs-manager listening:\n  control:   {}\n  documents: {}",
        args.control, args.documents
    );
    match server.run_async().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("cirrus qs-manager: {e}");
            1
        }
    }
}
