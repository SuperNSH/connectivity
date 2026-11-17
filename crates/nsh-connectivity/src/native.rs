//! The platform seam as TSX sees it: `NativeConnectivity`, whose async members reject with a
//! `code`. The TS package runs debouncing and the probe itself over these three calls.

use std::sync::Arc;

use nsh_core::prelude::*;
use tokio::sync::{mpsc, watch};

use crate::{
    backend::{Backend, PlatformEvent},
    error::ConnectivityError,
    platform,
    types::{InterfaceInfo, PlatformSnapshot},
};

/// The interface the platform reports, from TSX.
#[derive(JsObject, Clone, Debug)]
pub struct NativeInterface {
    /// The OS's name for it.
    pub name: String,
    /// Its connection type, as the TS string.
    pub kind: String,
    /// Its address.
    pub ip_address: Option<String>,
    /// Its subnet mask.
    pub subnet: Option<String>,
    /// The Wi-Fi network's name.
    pub ssid: Option<String>,
}

/// One snapshot of the platform, from TSX.
#[derive(JsObject, Clone, Debug)]
pub struct NativeSnapshot {
    /// The live types, strongest first, as TS strings; empty when nothing is live.
    pub types: Vec<String>,
    /// The current interface.
    pub interface: Option<NativeInterface>,
    /// The path is metered.
    pub is_expensive: bool,
}

/// The platform's connectivity backend. Create one and keep it.
#[class]
pub struct NativeConnectivity {
    backend: Arc<dyn Backend>,
    events: Option<mpsc::UnboundedReceiver<PlatformEvent>>,
    stopped: watch::Sender<u64>,
}

#[class]
impl NativeConnectivity {
    /// The platform's backend.
    pub fn new() -> NativeConnectivity {
        NativeConnectivity {
            backend: platform::default_backend(),
            events: None,
            stopped: watch::channel(0).0,
        }
    }

    /// The platform's view now.
    pub async fn snapshot(this: Instance<Self>) -> Result<NativeSnapshot, JsError> {
        let backend = this.with(|native| native.backend.clone());
        let snapshot = backend.snapshot().await.map_err(into_js)?;
        Ok(to_native(snapshot))
    }

    /// Starts the platform's change stream; `nextEvent` hands out its events.
    pub async fn watch(this: Instance<Self>) -> Result<(), JsError> {
        let backend = this.with(|native| native.backend.clone());
        let (sink, events) = mpsc::unbounded_channel();
        backend.watch(sink).map_err(into_js)?;
        this.with_mut(|native| native.events = Some(events));
        Ok(())
    }

    /// Resolves `true` at the next platform event, `false` once watching stopped.
    pub async fn next_event(this: Instance<Self>) -> bool {
        let (events, mut stopped) =
            this.with_mut(|native| (native.events.take(), native.stopped.subscribe()));
        let Some(mut events) = events else {
            return false;
        };
        let event = tokio::select! {
            biased;
            _ = stopped.changed() => None,
            event = events.recv() => event,
        };
        if event.is_some() {
            this.with_mut(|native| {
                if native.events.is_none() {
                    native.events = Some(events);
                }
            });
        }
        event.is_some()
    }

    /// Stops the change stream; a pending `nextEvent` resolves `false`. Idempotent.
    pub fn stop_watching(&mut self) {
        self.backend.stop_watching();
        self.events = None;
        self.stopped
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }
}

impl Default for NativeConnectivity {
    fn default() -> Self {
        Self::new()
    }
}

/// The error a rejection carries: its message, and the `code` apps match on.
fn into_js(error: ConnectivityError) -> JsError {
    JsError::Error(error.message().to_owned()).with_code(error.code())
}

fn to_native(snapshot: PlatformSnapshot) -> NativeSnapshot {
    NativeSnapshot {
        types: snapshot
            .types
            .iter()
            .map(|t| t.as_str().to_owned())
            .collect(),
        interface: snapshot.interface.map(|i: InterfaceInfo| NativeInterface {
            name: i.name,
            kind: i.r#type.as_str().to_owned(),
            ip_address: i.ip_address,
            subnet: i.subnet,
            ssid: i.ssid,
        }),
        is_expensive: snapshot.is_expensive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::ErrorKind, types::ConnectionType};

    #[test]
    fn a_snapshot_crosses_as_ts_strings() {
        let native = to_native(PlatformSnapshot {
            types: vec![ConnectionType::Wifi, ConnectionType::Vpn],
            interface: Some(InterfaceInfo {
                name: "en0".to_owned(),
                r#type: ConnectionType::Wifi,
                ip_address: None,
                subnet: None,
                ssid: None,
            }),
            is_expensive: true,
        });
        assert_eq!(native.types, vec!["wifi", "vpn"]);
        let interface = native.interface.expect("interface crosses");
        assert_eq!(interface.kind, "wifi");
        assert_eq!(interface.name, "en0");
        assert!(native.is_expensive);
    }

    #[test]
    fn a_rejection_carries_the_code() {
        let error = into_js(ConnectivityError::new(ErrorKind::NotAvailable));
        assert_eq!(
            error,
            JsError::Error(ErrorKind::NotAvailable.message().to_owned())
                .with_code("ERR_CONNECTIVITY_NOT_AVAILABLE")
        );
    }
}
