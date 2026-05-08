//! EPICS Channel Access backend for cirrus.
//!
//! Wires up the local `epics-ca-rs` crate. The actual integration (channel
//! lookup, sharded registry per K3, in-flight dedup per K4) is not yet
//! implemented — this crate currently exposes a stub that compiles and returns
//! `Backend("epics-ca not yet wired")` from every async method. The trait
//! shape is final; only the bodies need filling in.

#![deny(missing_docs)]

use async_trait::async_trait;
use cirrus_core::error::{CirrusError, Result};
use cirrus_core::reading::ReadingValue;
use cirrus_core::status::{Status, StatusError, SubToken};
use cirrus_event_model::DataKey;
use cirrus_protocols_async::{ReadingValueCallback, SignalBackend};
use serde::Serialize;
use std::time::Duration;

/// Stub `SignalBackend` that always errors. Replace with a real impl once
/// the `epics-ca-rs` API is mapped (TODO: M2).
pub struct EpicsCaBackend<T: Clone + Send + Sync + 'static> {
    pv: String,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Clone + Send + Sync + 'static> EpicsCaBackend<T> {
    /// Build with a PV name.
    pub fn new(pv: impl Into<String>) -> Self {
        Self {
            pv: pv.into(),
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + Serialize + 'static> SignalBackend<T> for EpicsCaBackend<T> {
    async fn connect(&self, _timeout: Duration) -> Result<()> {
        Err(CirrusError::Backend("epics-ca not yet wired".into()))
    }
    async fn put(&self, _value: T, _wait: bool, _timeout: Option<Duration>) -> Status {
        Status::fail(StatusError::Failed("epics-ca not yet wired".into()))
    }
    async fn get_datakey(&self, _source: &str) -> Result<DataKey> {
        Err(CirrusError::Backend("epics-ca not yet wired".into()))
    }
    async fn get_reading(&self) -> Result<ReadingValue> {
        Err(CirrusError::Backend("epics-ca not yet wired".into()))
    }
    async fn get_value(&self) -> Result<T> {
        Err(CirrusError::Backend("epics-ca not yet wired".into()))
    }
    async fn get_setpoint(&self) -> Result<T> {
        Err(CirrusError::Backend("epics-ca not yet wired".into()))
    }
    fn set_callback(&self, _cb: Option<ReadingValueCallback<T>>) -> SubToken {
        SubToken::noop()
    }
    fn source(&self, _name: &str) -> String {
        format!("ca://{}", self.pv)
    }
}
