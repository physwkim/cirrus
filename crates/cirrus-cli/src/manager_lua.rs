//! Daemon-side Lua bridge — pre-populates an mlua state with the
//! registry's devices and a `RE` global pointing at the daemon's
//! engine, then implements [`cirrus_qs::LuaEvaluator`] so the
//! `lua_eval` RPC can drive it.
//!
//! ## Concurrency
//!
//! One mlua state, shared across all RPC calls (single beamline =
//! single operator pattern). Acquired through a `tokio::sync::Mutex`
//! so concurrent `lua_eval` requests serialize. Plan execution
//! (`RE:run`) runs the engine on the same tokio runtime; the REP
//! socket loop is *not* blocked because `lua_eval` itself returns a
//! task_uid immediately.
//!
//! ## Lifecycle
//!
//! The Lua state is built lazily on the first `eval()` call. We
//! consult `engine_slot` then; if no engine is open, the eval fails
//! with a clear message. Once built, the Lua state persists for the
//! lifetime of the daemon process — globals set by previous evals
//! survive across calls.

use std::sync::Arc;

use async_trait::async_trait;
use cirrus_engine::RunEngine;
use cirrus_qs::{EvalResult, LuaEvaluator, Registry};
use mlua::Lua;
use tokio::sync::Mutex as TMutex;

use crate::lua_env::{build_lua, LuaDevice};

/// Daemon-side Lua bridge.
pub struct ManagerLuaState {
    /// Built lazily on first eval (needs an open engine).
    lua: TMutex<Option<Lua>>,
    /// Engine slot — populated by `environment_open`. We snapshot
    /// the Arc on lazy build.
    engine_slot: Arc<TMutex<Option<Arc<RunEngine>>>>,
    /// Read-only registry view; devices published as Lua globals.
    registry: Arc<Registry>,
}

impl ManagerLuaState {
    /// Build a state bound to the given engine slot + registry.
    /// Building does no I/O; the actual mlua state is constructed on
    /// the first `eval()` call (when the engine is guaranteed open).
    pub fn new(engine_slot: Arc<TMutex<Option<Arc<RunEngine>>>>, registry: Arc<Registry>) -> Self {
        Self {
            lua: TMutex::new(None),
            engine_slot,
            registry,
        }
    }

    fn build_state(re: Arc<RunEngine>, registry: &Registry) -> mlua::Result<Lua> {
        let lua = build_lua(re)?;
        // Publish each registered device as a Lua global with its
        // declared name. Walk the role tables; a device that appears
        // under multiple roles (motor: readable + movable) carries
        // both. Roles the registry doesn't currently track
        // (locatable, stoppable, monitorable, ...) are left None —
        // those calls error from Lua.
        for name in registry.device_names() {
            let dev = LuaDevice {
                name: name.clone(),
                readable: registry.readable(&name).cloned(),
                movable: registry.movable(&name).cloned(),
                locatable: None,
                stoppable: None,
                triggerable: registry.triggerable(&name).cloned(),
                stageable: registry.stageable(&name).cloned(),
                monitorable: None,
                flyable: registry.flyable(&name).cloned(),
                preparable: None,
                configurable: None,
                collectable: registry.collectable(&name).cloned(),
                pausable: None,
            };
            lua.globals().set(name.as_str(), dev)?;
        }
        Ok(lua)
    }
}

#[async_trait]
impl LuaEvaluator for ManagerLuaState {
    async fn eval(&self, source: &str) -> EvalResult {
        let mut lua_lock = self.lua.lock().await;

        if lua_lock.is_none() {
            let re = match self.engine_slot.lock().await.as_ref() {
                Some(e) => e.clone(),
                None => {
                    return EvalResult {
                        stdout: String::new(),
                        return_value: None,
                        error: Some(
                            "lua_eval: environment not open. Call \
                             environment_open first."
                                .into(),
                        ),
                    };
                }
            };
            match Self::build_state(re, &self.registry) {
                Ok(l) => *lua_lock = Some(l),
                Err(e) => {
                    return EvalResult {
                        stdout: String::new(),
                        return_value: None,
                        error: Some(format!("lua_eval: build state: {e}")),
                    };
                }
            }
        }

        let lua = lua_lock.as_mut().unwrap();
        eval_in(lua, source)
    }
}

/// Capture stdout + return value of evaluating `source` in `lua`.
/// Tries `return <source>` first (so bare expressions surface a
/// return_value); falls back to `source` as a statement chunk.
fn eval_in(lua: &Lua, source: &str) -> EvalResult {
    use mlua::Function;
    use std::sync::Mutex as StdMutex;

    let captured: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
    let cap_for_fn = captured.clone();
    let saved_print = lua.globals().get::<Function>("print").ok();
    let new_print = match lua.create_function(move |_, args: mlua::Variadic<mlua::Value>| {
        let mut parts = Vec::with_capacity(args.len());
        for v in args.iter() {
            parts.push(value_to_string(v));
        }
        cap_for_fn.lock().unwrap().push(parts.join("\t"));
        Ok(())
    }) {
        Ok(f) => f,
        Err(e) => {
            return EvalResult {
                stdout: String::new(),
                return_value: None,
                error: Some(format!("create capture print: {e}")),
            };
        }
    };
    if let Err(e) = lua.globals().set("print", new_print) {
        return EvalResult {
            stdout: String::new(),
            return_value: None,
            error: Some(format!("install capture print: {e}")),
        };
    }

    let outcome = run_chunk(lua, source);

    if let Some(p) = saved_print {
        let _ = lua.globals().set("print", p);
    }

    let stdout = captured.lock().unwrap().join("\n");
    match outcome {
        Ok(rv) => EvalResult {
            stdout,
            return_value: rv,
            error: None,
        },
        Err(e) => EvalResult {
            stdout,
            return_value: None,
            error: Some(e),
        },
    }
}

fn run_chunk(lua: &Lua, source: &str) -> Result<Option<String>, String> {
    // Try as expression first.
    let as_expr = format!("return {source}");
    match lua.load(&as_expr).eval::<mlua::Value>() {
        Ok(v) => Ok(Some(value_to_string(&v))),
        Err(_) => match lua.load(source).exec() {
            Ok(()) => Ok(None),
            Err(e) => Err(format!("{e}")),
        },
    }
}

fn value_to_string(v: &mlua::Value) -> String {
    match v {
        mlua::Value::Nil => "nil".to_string(),
        mlua::Value::Boolean(b) => b.to_string(),
        mlua::Value::Integer(i) => i.to_string(),
        mlua::Value::Number(n) => n.to_string(),
        mlua::Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cirrus_engine::RunEngine;

    fn fresh_state() -> (Arc<TMutex<Option<Arc<RunEngine>>>>, ManagerLuaState) {
        let engine_slot = Arc::new(TMutex::new(None));
        let registry = Arc::new(Registry::new());
        let state = ManagerLuaState::new(engine_slot.clone(), registry);
        (engine_slot, state)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_without_environment_errors() {
        let (_slot, state) = fresh_state();
        let r = state.eval("1 + 1").await;
        assert!(r.error.is_some());
        assert!(r.error.as_deref().unwrap().contains("environment not open"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_expression_returns_value() {
        let (slot, state) = fresh_state();
        *slot.lock().await = Some(Arc::new(RunEngine::new(vec![])));
        let r = state.eval("1 + 2").await;
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(r.return_value.as_deref(), Some("3"));
        assert_eq!(r.stdout, "");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_print_captures_stdout() {
        let (slot, state) = fresh_state();
        *slot.lock().await = Some(Arc::new(RunEngine::new(vec![])));
        let r = state.eval("print(\"hello\"); print(42)").await;
        assert!(r.error.is_none(), "{r:?}");
        assert!(r.return_value.is_none());
        assert_eq!(r.stdout, "hello\n42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_globals_persist_between_calls() {
        let (slot, state) = fresh_state();
        *slot.lock().await = Some(Arc::new(RunEngine::new(vec![])));
        let _ = state.eval("x = 41").await;
        let r = state.eval("x + 1").await;
        assert_eq!(r.return_value.as_deref(), Some("42"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_lua_error_surfaces() {
        let (slot, state) = fresh_state();
        *slot.lock().await = Some(Arc::new(RunEngine::new(vec![])));
        let r = state.eval("error('boom')").await;
        assert!(r.error.is_some());
        assert!(r.error.as_deref().unwrap().contains("boom"));
    }
}
