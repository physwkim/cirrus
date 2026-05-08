//! EPICS PV Access backend for cirrus.
//!
//! Stubs only — wiring to `epics-pva-rs` is M5 work. See `cirrus-backend-epics-ca`
//! for the same pattern.

#![deny(missing_docs)]

use async_trait::async_trait;
use cirrus_core::error::{CirrusError, Result};
use cirrus_core::reading::ReadingValue;
use cirrus_core::status::{Status, StatusError, SubToken};
use cirrus_event_model::DataKey;
use cirrus_protocols_async::{ReadingValueCallback, SignalBackend};
use serde::Serialize;
use std::time::Duration;

/// Stub PVA backend.
pub struct EpicsPvaBackend<T: Clone + Send + Sync + 'static> {
    pv: String,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Clone + Send + Sync + 'static> EpicsPvaBackend<T> {
    /// Build with a PV name.
    pub fn new(pv: impl Into<String>) -> Self {
        Self {
            pv: pv.into(),
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + Serialize + 'static> SignalBackend<T> for EpicsPvaBackend<T> {
    async fn connect(&self, _timeout: Duration) -> Result<()> {
        Err(CirrusError::Backend("epics-pva not yet wired".into()))
    }
    async fn put(&self, _value: T, _wait: bool, _timeout: Option<Duration>) -> Status {
        Status::fail(StatusError::Failed("epics-pva not yet wired".into()))
    }
    async fn get_datakey(&self, _source: &str) -> Result<DataKey> {
        Err(CirrusError::Backend("epics-pva not yet wired".into()))
    }
    async fn get_reading(&self) -> Result<ReadingValue> {
        Err(CirrusError::Backend("epics-pva not yet wired".into()))
    }
    async fn get_value(&self) -> Result<T> {
        Err(CirrusError::Backend("epics-pva not yet wired".into()))
    }
    async fn get_setpoint(&self) -> Result<T> {
        Err(CirrusError::Backend("epics-pva not yet wired".into()))
    }
    fn set_callback(&self, _cb: Option<ReadingValueCallback<T>>) -> SubToken {
        SubToken::noop()
    }
    fn source(&self, _name: &str) -> String {
        format!("pva://{}", self.pv)
    }
}
