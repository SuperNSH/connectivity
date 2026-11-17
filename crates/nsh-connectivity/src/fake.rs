//! A scripted backend and probe for tests. Clones share one state.

use std::{
    future::{pending, ready},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::sync::mpsc::UnboundedSender;

use crate::{
    backend::{Backend, BackendFuture, PlatformEvent},
    error::ConnectivityError,
    probe::Probe,
    types::{ConnectionType, InterfaceInfo, PlatformSnapshot, ProbeOutcome},
};

/// A device the test scripts: what `snapshot` answers, whether `watch` fails, and when the
/// platform has news.
#[derive(Clone)]
pub struct FakeBackend {
    state: Arc<Mutex<FakeState>>,
}

struct FakeState {
    snapshot: Result<PlatformSnapshot, ConnectivityError>,
    watch_error: Option<ConnectivityError>,
    sink: Option<UnboundedSender<PlatformEvent>>,
    watchers: usize,
    stopped: usize,
    snapshots: usize,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    /// A device on Wi-Fi (`wlan0`), connected.
    pub fn new() -> Self {
        FakeBackend {
            state: Arc::new(Mutex::new(FakeState {
                snapshot: Ok(PlatformSnapshot {
                    types: vec![ConnectionType::Wifi],
                    interface: Some(InterfaceInfo {
                        name: "wlan0".to_owned(),
                        r#type: ConnectionType::Wifi,
                        ip_address: Some("192.168.1.20".to_owned()),
                        subnet: Some("255.255.255.0".to_owned()),
                        ssid: Some("Home".to_owned()),
                    }),
                    is_expensive: false,
                }),
                watch_error: None,
                sink: None,
                watchers: 0,
                stopped: 0,
                snapshots: 0,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the next snapshots answer.
    pub fn set_snapshot(&self, snapshot: Result<PlatformSnapshot, ConnectivityError>) {
        self.lock().snapshot = snapshot;
    }

    /// Makes the next `watch` fail with `error`; `None` lets it succeed.
    pub fn set_watch_error(&self, error: Option<ConnectivityError>) {
        self.lock().watch_error = error;
    }

    /// The platform has news: one event to the watcher. True when a watcher existed.
    pub fn emit_event(&self) -> bool {
        let state = self.lock();
        state
            .sink
            .as_ref()
            .is_some_and(|sink| sink.send(PlatformEvent).is_ok())
    }

    /// How many `watch` calls succeeded.
    pub fn watchers(&self) -> usize {
        self.lock().watchers
    }

    /// How many times `stop_watching` ran.
    pub fn stopped(&self) -> usize {
        self.lock().stopped
    }

    /// How many snapshots were read.
    pub fn snapshots(&self) -> usize {
        self.lock().snapshots
    }
}

impl Backend for FakeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        let mut state = self.lock();
        state.snapshots += 1;
        Box::pin(ready(state.snapshot.clone()))
    }

    fn watch(&self, sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        let mut state = self.lock();
        if let Some(error) = state.watch_error.take() {
            return Err(error);
        }
        state.sink = Some(sink);
        state.watchers += 1;
        Ok(())
    }

    fn stop_watching(&self) {
        let mut state = self.lock();
        state.sink = None;
        state.stopped += 1;
    }
}

/// A probe the test scripts: an outcome, or no answer at all. Clones share one state.
#[derive(Clone)]
pub struct FakeProbe {
    state: Arc<Mutex<ProbeState>>,
}

struct ProbeState {
    outcome: Option<ProbeOutcome>,
    urls: Vec<String>,
}

impl FakeProbe {
    /// Answers `outcome` at once.
    pub fn answering(outcome: ProbeOutcome) -> Self {
        Self::with(Some(outcome))
    }

    /// Never answers, as a stalled server.
    pub fn stalled() -> Self {
        Self::with(None)
    }

    fn with(outcome: Option<ProbeOutcome>) -> Self {
        FakeProbe {
            state: Arc::new(Mutex::new(ProbeState {
                outcome,
                urls: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ProbeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What later probes answer; `None` never answers.
    pub fn set_outcome(&self, outcome: Option<ProbeOutcome>) {
        self.lock().outcome = outcome;
    }

    /// How many probes ran.
    pub fn probes(&self) -> usize {
        self.lock().urls.len()
    }

    /// The URLs probed, in order.
    pub fn urls(&self) -> Vec<String> {
        self.lock().urls.clone()
    }
}

impl Probe for FakeProbe {
    fn probe(&self, url: &str, _timeout: Duration) -> BackendFuture<'_, ProbeOutcome> {
        let mut state = self.lock();
        state.urls.push(url.to_owned());
        match state.outcome {
            Some(outcome) => Box::pin(ready(outcome)),
            None => Box::pin(pending()),
        }
    }
}
