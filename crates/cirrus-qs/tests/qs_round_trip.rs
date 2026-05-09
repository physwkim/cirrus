//! End-to-end test: REQ → cirrus-qs REP → engine → response.

use std::sync::Arc;
use std::time::Duration;

use cirrus_backend_soft::SoftDetector;
use cirrus_core::msg::ReadableObj;
use cirrus_qs::{Registry, Server, ServerShutdown};
use serde_json::{json, Value};

fn rand_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let bump = NEXT.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32;
    // Use 14 bits of entropy (16384 slots) instead of 10 (1024) so
    // parallel test runs collide on the IPC socket path far less
    // often. The "port" here is a stringification ingredient only —
    // collisions land two tests on the same ipc:// path, so the
    // second `bind` fails with "Address already in use".
    let base = 32_768u16;
    let offset = ((nanos.wrapping_add(bump as u32 * 16_777_213)) & 0x3FFF) as u16;
    base.saturating_add(offset)
}

fn endpoint(port: u16) -> String {
    format!(
        "ipc:///tmp/cirrus-qs-test-{}-{}.sock",
        std::process::id(),
        port
    )
}

fn rpc(socket: &zmq::Socket, method: &str, params: Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    socket.send(serde_json::to_vec(&req).unwrap(), 0).unwrap();
    let resp = socket.recv_bytes(0).unwrap();
    serde_json::from_slice(&resp).unwrap()
}

fn spawn_server(reg: Registry, port: u16) -> ServerShutdown {
    spawn_server_inner(reg, port, None)
}

fn spawn_server_with_perms(
    reg: Registry,
    port: u16,
    perms_path: std::path::PathBuf,
) -> ServerShutdown {
    spawn_server_inner(reg, port, Some(perms_path))
}

fn spawn_server_inner(
    reg: Registry,
    port: u16,
    perms_path: Option<std::path::PathBuf>,
) -> ServerShutdown {
    let ep = endpoint(port);
    let mut builder = Server::builder()
        .control_address(ep)
        .document_address(format!(
            "ipc:///tmp/cirrus-qs-doc-{}-{}.sock",
            std::process::id(),
            port
        ))
        .registry(reg);
    if let Some(p) = perms_path {
        builder = builder.permissions_path(p);
    }
    let server = builder.build().expect("server build");
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        let _ = server.run_async().await;
    });
    shutdown
}

fn req_socket(port: u16) -> zmq::Socket {
    let ctx = zmq::Context::new();
    let req = ctx.socket(zmq::REQ).unwrap();
    req.set_rcvtimeo(3_000).unwrap();
    req.set_sndtimeo(3_000).unwrap();
    req.connect(&endpoint(port)).unwrap();
    req
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ping_works() {
    let port = rand_port();
    let mut reg = Registry::new();
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let req = req_socket(port);
    let r = rpc(&req, "ping", json!({}));
    assert_eq!(r["result"]["msg"], "pong");

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_to_end_count_through_qs() {
    let port = rand_port();
    let det = SoftDetector::new("det1");
    let mut reg = Registry::new();
    reg.register_readable("det1", det as Arc<dyn ReadableObj>);
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let req = req_socket(port);

    let r = rpc(&req, "environment_open", json!({}));
    assert_eq!(r["result"]["success"], true);

    let r = rpc(&req, "plans_allowed", json!({}));
    let plans = r["result"]["plans_allowed"].as_array().unwrap();
    assert!(plans.iter().any(|v| v == "count"));

    let r = rpc(&req, "devices_allowed", json!({}));
    let devs = r["result"]["devices_allowed"].as_array().unwrap();
    assert!(devs.iter().any(|v| v == "det1"));

    let r = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 3]}}),
    );
    assert_eq!(r["result"]["success"], true);
    assert_eq!(r["result"]["qsize"], 1);

    let r = rpc(&req, "status", json!({}));
    assert_eq!(r["result"]["items_in_queue"], 1);
    assert_eq!(r["result"]["manager_state"], "idle");

    let r = rpc(&req, "queue_start", json!({}));
    assert_eq!(r["result"]["success"], true);

    let mut done = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let r = rpc(&req, "status", json!({}));
        if r["result"]["plans_run"].as_u64().unwrap_or(0) >= 1
            && r["result"]["items_in_queue"] == 0
            && r["result"]["manager_state"] == "idle"
        {
            done = true;
            break;
        }
    }
    assert!(done, "queue did not finish");

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_plan_rejected() {
    let port = rand_port();
    let mut reg = Registry::new();
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let req = req_socket(port);

    let r = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "no_such_plan", "args": []}}),
    );
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown plan"));

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_aborts_running_queue_task() {
    use cirrus_core::msg::Msg;
    use cirrus_core::plan::plan_box;
    use std::sync::atomic::{AtomicU64, Ordering};

    let port = rand_port();
    let counter = Arc::new(AtomicU64::new(0));
    let counter_for_factory = counter.clone();
    let mut reg = Registry::new();
    let factory: cirrus_qs::PlanFactory = Arc::new(move |_reg, _args| {
        let c = counter_for_factory.clone();
        Ok(plan_box(async_stream::stream! {
            loop {
                yield Msg::Sleep(Duration::from_millis(50));
                c.fetch_add(1, Ordering::SeqCst);
            }
        }))
    });
    reg.register_plan("long_loop", factory);
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let req = req_socket(port);
    rpc(&req, "environment_open", json!({}));
    rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "long_loop", "args": []}}),
    );
    rpc(&req, "queue_start", json!({}));
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Confirm the worker is alive and ticking.
    let mid = counter.load(Ordering::SeqCst);
    assert!(mid > 0, "queue worker did not advance pre-shutdown");

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let after = counter.load(Ordering::SeqCst);

    // Wait again. If shutdown's abort fired, the counter must NOT advance.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let later = counter.load(Ordering::SeqCst);
    assert_eq!(
        after, later,
        "queue worker continued ticking after shutdown — abort did not propagate"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_method_returns_jsonrpc_error() {
    let port = rand_port();
    let mut reg = Registry::new();
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let req = req_socket(port);

    let r = rpc(&req, "no_such_method", json!({}));
    assert_eq!(r["error"]["code"], -32601);

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_get_returns_implementation_metadata() {
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let r = rpc(&req, "config_get", json!({}));
    assert_eq!(r["result"]["config"]["implementation"], "cirrus-qs");
    assert!(r["result"]["config"]["version"].is_string());
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plans_existing_matches_plans_allowed() {
    let port = rand_port();
    let mut reg = Registry::new();
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let allowed = rpc(&req, "plans_allowed", json!({}));
    let existing = rpc(&req, "plans_existing", json!({}));
    assert_eq!(
        allowed["result"]["plans_allowed"],
        existing["result"]["plans_existing"]
    );
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_clear_empties_queue() {
    let port = rand_port();
    let det = SoftDetector::new("det1");
    let mut reg = Registry::new();
    reg.register_readable("det1", det as Arc<dyn ReadableObj>);
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 1]}}),
    );
    rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 1]}}),
    );
    let s = rpc(&req, "status", json!({}));
    assert_eq!(s["result"]["items_in_queue"], 2);
    rpc(&req, "queue_clear", json!({}));
    let s = rpc(&req, "status", json!({}));
    assert_eq!(s["result"]["items_in_queue"], 0);
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_item_move_and_get_by_uid() {
    let port = rand_port();
    let det = SoftDetector::new("det1");
    let mut reg = Registry::new();
    reg.register_readable("det1", det as Arc<dyn ReadableObj>);
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let r1 = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 1]}}),
    );
    let r2 = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 2]}}),
    );
    let uid_first = r1["result"]["item_uid"].as_str().unwrap().to_string();
    let uid_second = r2["result"]["item_uid"].as_str().unwrap().to_string();

    // Move the second item to the front.
    let mv = rpc(
        &req,
        "queue_item_move",
        json!({"uid": uid_second, "pos_dest": "front"}),
    );
    assert_eq!(mv["result"]["success"], true);

    // Verify queue order via queue_get.
    let q = rpc(&req, "queue_get", json!({}));
    let items = q["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["item_uid"], uid_second);
    assert_eq!(items[1]["item_uid"], uid_first);

    // queue_item_get by uid.
    let one = rpc(&req, "queue_item_get", json!({"uid": uid_first}));
    assert_eq!(one["result"]["item"]["item_uid"], uid_first);

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn history_populates_after_run() {
    let port = rand_port();
    let det = SoftDetector::new("det1");
    let mut reg = Registry::new();
    reg.register_readable("det1", det as Arc<dyn ReadableObj>);
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);

    rpc(&req, "environment_open", json!({}));
    rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": ["det1", 1]}}),
    );
    rpc(&req, "queue_start", json!({}));

    let mut done = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(80)).await;
        let s = rpc(&req, "status", json!({}));
        if s["result"]["plans_run"].as_u64().unwrap_or(0) >= 1 {
            done = true;
            break;
        }
    }
    assert!(done);

    let h = rpc(&req, "history_get", json!({}));
    let items = h["result"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "history should have at least one item");
    assert_eq!(items[0]["name"], "count");

    rpc(&req, "history_clear", json!({}));
    let h = rpc(&req, "history_get", json!({}));
    assert_eq!(h["result"]["items"].as_array().unwrap().len(), 0);

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lock_blocks_queue_ops_unless_keyed() {
    let port = rand_port();
    let mut reg = Registry::new();
    reg.register_plan_count("count");
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);

    let r = rpc(
        &req,
        "lock",
        json!({"lock_key": "secret", "queue": true, "user": "alice"}),
    );
    assert_eq!(r["result"]["success"], true);

    // Without lock_key — must be rejected.
    let r = rpc(&req, "queue_clear", json!({}));
    assert!(r["error"]["message"].as_str().unwrap().contains("locked"));

    // With wrong key — also rejected.
    let r = rpc(&req, "queue_clear", json!({"lock_key": "wrong"}));
    assert!(r["error"]["message"].as_str().unwrap().contains("locked"));

    // With correct key — allowed.
    let r = rpc(&req, "queue_clear", json!({"lock_key": "secret"}));
    assert_eq!(r["result"]["success"], true);

    // Unlock.
    let r = rpc(&req, "unlock", json!({"lock_key": "secret"}));
    assert_eq!(r["result"]["success"], true);

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn re_metadata_round_trip() {
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    rpc(&req, "environment_open", json!({}));
    rpc(
        &req,
        "re_metadata",
        json!({"metadata": {"operator": "alice", "beamline": "BL-7"}}),
    );
    let r = rpc(&req, "re_metadata", json!({}));
    assert_eq!(r["result"]["metadata"]["operator"], "alice");
    assert_eq!(r["result"]["metadata"]["beamline"], "BL-7");
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn not_implemented_methods_return_defined_error() {
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    // Methods that remain registered-but-stub-only (NOT_IMPLEMENTED).
    // permissions_reload moved out — it's now actually implemented and
    // gated by RBAC (admin-only), tested separately below. Likewise
    // permissions_get / manager_test / task_status / task_result return
    // success (819bf6e wire-compat).
    for m in [
        "permissions_set",
        "script_upload",
        "function_execute",
        "kernel_interrupt",
        "manager_stop",
        "manager_kill",
    ] {
        let r = rpc(&req, m, json!({}));
        assert_eq!(
            r["error"]["code"], -32099,
            "method {m} should report NOT_IMPLEMENTED"
        );
    }
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_includes_bluesky_fields() {
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let s = rpc(&req, "status", json!({}));
    let r = &s["result"];
    for k in [
        "manager_state",
        "items_in_queue",
        "items_in_history",
        "plans_run",
        "plans_failed",
        "re_state",
        "worker_environment_exists",
        "queue_stop_pending",
        "queue_autostart_enabled",
        "plan_queue_uid",
        "plan_history_uid",
        "lock_info_uid",
    ] {
        assert!(!r[k].is_null(), "status missing field: {k} (got {s})");
    }
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_status_returns_completed_for_any_uid() {
    // bluesky-queueserver clients always poll task_status after
    // queue_item_execute. cirrus-qs runs synchronously, so we don't
    // track tasks — but returning a clean "completed" shape keeps
    // those clients happy.
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let r = rpc(&req, "task_status", json!({"task_uid": "anything"}));
    assert_eq!(r["result"]["status"], "completed");
    let r = rpc(&req, "task_result", json!({"task_uid": "anything"}));
    assert_eq!(r["result"]["status"], "completed");
    let r = rpc(&req, "manager_test", json!({}));
    assert_eq!(r["result"]["success"], true);
    let r = rpc(&req, "permissions_get", json!({}));
    assert!(r["result"]["user_group_permissions"].is_object());
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_includes_manager_version() {
    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);
    let s = rpc(&req, "status", json!({}));
    let v = &s["result"]["manager_version"];
    assert!(v.is_string(), "manager_version should be a string, got {v}");
    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn device_inspect_returns_state_json() {
    let port = rand_port();
    let det = SoftDetector::new("det1");
    let mut reg = Registry::new();
    reg.register_readable("det1", det as Arc<dyn ReadableObj>);
    let shutdown = spawn_server(reg, port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);

    let r = rpc(&req, "device_inspect", json!({"name": "det1"}));
    assert!(
        r["result"]["success"].as_bool().unwrap_or(false),
        "device_inspect should succeed: {r}"
    );
    assert_eq!(r["result"]["name"], "det1");
    assert_eq!(r["result"]["state"]["type"], "SoftDetector");
    assert_eq!(r["result"]["state"]["name"], "det1");
    assert!(r["result"]["state"]["counts"].is_number());

    // Unknown device.
    let r = rpc(&req, "device_inspect", json!({"name": "nope"}));
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("no device"),
        "expected 'no device' message, got {r}"
    );

    // Missing name param.
    let r = rpc(&req, "device_inspect", json!({}));
    assert_eq!(
        r["error"]["code"], -32602,
        "missing name → INVALID_PARAMS: {r}"
    );

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rbac_denies_mutation_for_read_only_group() {
    // Permissions config: anonymous callers (no api_key) land in
    // `viewer`, which is read-only. `admin-key` → admin group with full
    // access. Read-only callers must get NOT_AUTHORIZED on mutating
    // RPCs and success on info RPCs; admin must succeed on both.
    let toml = r#"
        default_group = "viewer"

        [user_groups.viewer]
        read_only = true
        allowed_plans = []
        allowed_devices = []

        [user_groups.admin]
        admin = true
        allowed_plans = [".*"]
        allowed_devices = [".*"]

        [api_keys]
        "admin-key" = "admin"
    "#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("permissions.toml");
    std::fs::write(&path, toml).unwrap();

    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server_with_perms(reg, port, path);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);

    // Info RPC always succeeds — even for read_only.
    let r = rpc(&req, "ping", json!({}));
    assert!(r["result"]["success"].as_bool().unwrap_or(false));

    // Mutation RPC: anonymous → denied (NOT_AUTHORIZED).
    let r = rpc(&req, "queue_clear", json!({}));
    assert_eq!(r["error"]["code"], -32001, "viewer should be denied: {r}");

    // Mutation RPC: admin-key → succeeds.
    let r = rpc(&req, "queue_clear", json!({"api_key": "admin-key"}));
    assert!(
        r["result"]["success"].as_bool().unwrap_or(false),
        "admin should succeed: {r}"
    );

    // permissions_reload (Admin class): viewer denied, admin OK.
    let r = rpc(&req, "permissions_reload", json!({}));
    assert_eq!(r["error"]["code"], -32001, "viewer permissions_reload: {r}");
    let r = rpc(&req, "permissions_reload", json!({"api_key": "admin-key"}));
    assert!(
        r["result"]["success"].as_bool().unwrap_or(false),
        "admin permissions_reload: {r}"
    );

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rbac_filters_plan_name_on_queue_add() {
    // viewer is read-only by default; primary allows only "count"
    // and "scan_*". queue_item_add with plan name "fly" must be
    // denied; "count" and "scan_grid" must pass plan-name check
    // (they may still fail because the plan is not registered, but
    // the *RBAC* check passes — assert by error code).
    let toml = r#"
        default_group = "primary"
        [user_groups.primary]
        allowed_plans = ["count", "scan_.*"]
        allowed_devices = [".*"]
    "#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("permissions.toml");
    std::fs::write(&path, toml).unwrap();

    let port = rand_port();
    let reg = Registry::new();
    let shutdown = spawn_server_with_perms(reg, port, path);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let req = req_socket(port);

    // "fly" is not in allowed_plans → NOT_AUTHORIZED.
    let r = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "fly", "args": []}}),
    );
    assert_eq!(
        r["error"]["code"], -32001,
        "fly should be RBAC-denied for primary: {r}"
    );

    // "count" passes RBAC (may then fail because it's not registered
    // in this test's empty Registry, but that's a different code).
    let r = rpc(
        &req,
        "queue_item_add",
        json!({"item": {"name": "count", "args": []}}),
    );
    assert_ne!(
        r["error"]["code"], -32001,
        "count should not be RBAC-denied: {r}"
    );

    shutdown.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
}
