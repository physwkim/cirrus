//! Engine-side `Suspender` registry. The trait itself lives in
//! `cirrus-core` so plan factories can reference it without depending
//! on `cirrus-engine`. Installation hands the engine a watcher; removal
//! aborts it (rule **K1**).
//!
//! Reference: bluesky `run_engine.py:1132-1310` (`install_suspender`,
//! `request_suspend`, `_start_suspender`).

pub use cirrus_core::suspender::Suspender;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Boxed pre/post plan injection. `None` = nothing to inject.
pub type SuspendInjection = Option<crate::engine::SuspendCallback>;

/// Live registration record. Drop aborts the watcher task (rule **K1**).
pub(crate) struct SuspenderHandle {
    /// Stable id used by `RemoveSuspender` Msg.
    #[allow(dead_code)]
    pub(crate) id: u64,
    /// Underlying suspender (kept alive while the registration exists).
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn Suspender>,
    /// The watcher task — drop / abort on Drop.
    pub(crate) abort: tokio::task::AbortHandle,
}

impl SuspenderHandle {
    pub(crate) fn new(id: u64, inner: Arc<dyn Suspender>, handle: JoinHandle<()>) -> Self {
        let abort = handle.abort_handle();
        Self { id, inner, abort }
    }
}

impl Drop for SuspenderHandle {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
