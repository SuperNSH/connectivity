//! Network status, change events and Internet reachability for Native Shell apps.
//!
//! [`Connectivity`] follows Flutter's `connectivity_plus` ([`Connectivity::check_connectivity`],
//! [`Connectivity::on_connectivity_changed`]) and React Native's NetInfo ([`Connectivity::fetch`],
//! [`Connectivity::refresh`], [`Connectivity::subscribe`]). Reachability is probed with an HTTP
//! GET whose 204 means the Internet answered, never inferred from an interface being up.
//! Every call resolves exactly once: a value, a coded [`ConnectivityError`], or a cancel.

mod backend;
mod error;
mod fake;
mod native;
mod platform;
mod probe;
mod rt;
mod types;

use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use nsh_core::prelude::*;
use tokio::sync::{mpsc, watch};

pub use backend::{Backend, BackendFuture, PlatformEvent};
pub use error::{ConnectivityError, ErrorKind};
pub use fake::{FakeBackend, FakeProbe};
pub use native::{NativeConnectivity, NativeInterface, NativeSnapshot};
pub use probe::{HttpProbe, Probe, classify};
pub use types::{
    CellularGeneration, Configuration, ConnectionType, InterfaceInfo, NetworkDetails, NetworkState,
    PlatformSnapshot, ProbeOutcome, Reachability,
};

use types::{listed, state_of};

/// The crate's UI set: what TSX imports from `nsh-connectivity/native`.
#[native_module("nsh-connectivity/native")]
fn native(registry: &mut Registry) {
    registry.class::<NativeConnectivity>();
}

/// The connectivity of this device. Cheap to clone; clones share the state, the
/// subscriptions and the in-flight probe.
#[derive(Clone)]
pub struct Connectivity {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Arc<dyn Backend>,
    probe: Arc<dyn Probe>,
    config: Mutex<Configuration>,
    shared: Mutex<Shared>,
    /// Bumped by `stop`: every operation started before it resolves `Cancelled`.
    stop: watch::Sender<u64>,
}

type Answer = Option<Result<NetworkState, ConnectivityError>>;

#[derive(Default)]
struct Shared {
    current: Option<NetworkState>,
    /// The types `current` came from, and the reachability known for them.
    types: Option<Vec<ConnectionType>>,
    reach: (Reachability, bool),
    hub: Option<Arc<Hub>>,
    inflight: Option<watch::Receiver<Answer>>,
}

/// What subscribers read, and whether the platform's change stream runs for them.
struct Hub {
    states: watch::Sender<NetworkState>,
    types: watch::Sender<Vec<ConnectionType>>,
    watching: AtomicBool,
}

/// connectivity_plus's change stream: distinct, never-empty type lists.
#[derive(Debug)]
pub struct ConnectivityStream {
    rx: watch::Receiver<Vec<ConnectionType>>,
    last: Option<Vec<ConnectionType>>,
}

impl ConnectivityStream {
    /// The next list that differs from the last one yielded, starting with the current one.
    /// `None` once the stream ended (`stop`).
    pub async fn next(&mut self) -> Option<Vec<ConnectionType>> {
        loop {
            let types = self.rx.borrow_and_update().clone();
            if self.last.as_ref() != Some(&types) {
                self.last = Some(types.clone());
                return Some(types);
            }
            if self.rx.changed().await.is_err() {
                return None;
            }
        }
    }
}

impl Default for Connectivity {
    fn default() -> Self {
        Self::new()
    }
}

impl Connectivity {
    /// The platform's backend and the HTTP probe.
    pub fn new() -> Self {
        Self::build(platform::default_backend(), Arc::new(HttpProbe))
    }

    /// Over `backend`, for tests and fakes, with the HTTP probe.
    pub fn with_backend(backend: impl Backend + 'static) -> Self {
        Self::build(Arc::new(backend), Arc::new(HttpProbe))
    }

    /// The same backend with `probe` answering reachability. Call it before using the handle.
    #[must_use]
    pub fn with_probe(self, probe: impl Probe + 'static) -> Self {
        let config = self.config();
        let built = Self::build(self.inner.backend.clone(), Arc::new(probe));
        built.configure(config);
        built
    }

    fn build(backend: Arc<dyn Backend>, probe: Arc<dyn Probe>) -> Self {
        Connectivity {
            inner: Arc::new(Inner {
                backend,
                probe,
                config: Mutex::new(Configuration::default()),
                shared: Mutex::new(Shared::default()),
                stop: watch::channel(0).0,
            }),
        }
    }

    /// The live types, strongest first: never empty, and exactly `[None]` when nothing is live.
    pub async fn check_connectivity(&self) -> Result<Vec<ConnectionType>, ConnectivityError> {
        let snapshot = self.guarded(self.inner.backend.snapshot()).await?;
        Ok(listed(&snapshot.types))
    }

    /// The state now, from a fresh snapshot and the last probe answer for the same path. With
    /// `Some(name)`, the state of that interface: a name the platform does not report is
    /// `UnknownInterface`, an empty one `InvalidArgument`.
    pub async fn fetch(&self, interface: Option<&str>) -> Result<NetworkState, ConnectivityError> {
        if interface == Some("") {
            return Err(ErrorKind::InvalidArgument.into());
        }
        let snapshot = self.guarded(self.inner.backend.snapshot()).await?;
        let state = self.absorb(&snapshot, None);
        let Some(name) = interface else {
            return Ok(state);
        };
        match snapshot.interface.as_ref().filter(|i| i.name == name) {
            Some(found) => Ok(NetworkState {
                connection_type: found.r#type,
                ..state
            }),
            None => Err(ErrorKind::UnknownInterface.into()),
        }
    }

    /// A fresh snapshot and a fresh probe. Concurrent callers share one cycle.
    pub async fn refresh(&self) -> Result<NetworkState, ConnectivityError> {
        let (mut answer, leader) = {
            let mut shared = self.shared();
            match &shared.inflight {
                Some(rx) => (rx.clone(), None),
                None => {
                    let (tx, rx) = watch::channel(None);
                    shared.inflight = Some(rx.clone());
                    (rx, Some(tx))
                }
            }
        };
        match leader {
            Some(tx) => {
                let clear = ClearInflight(&self.inner);
                let result = self.guarded(self.cycle()).await;
                drop(clear);
                tx.send_replace(Some(result.clone()));
                result
            }
            None => {
                let wait = async move {
                    match answer.wait_for(Option::is_some).await {
                        Ok(value) => value.clone().unwrap_or(Err(ErrorKind::Cancelled.into())),
                        // The leading caller was dropped mid-cycle.
                        Err(_) => Err(ErrorKind::Cancelled.into()),
                    }
                };
                self.guarded(wait).await
            }
        }
    }

    /// The last state this handle knew, without a probe; a fresh snapshot when it knows none.
    pub async fn state(&self) -> Result<NetworkState, ConnectivityError> {
        if let Some(current) = self.shared().current.clone() {
            return Ok(current);
        }
        self.fetch(None).await
    }

    /// The configuration the next read uses. Work already in flight keeps its own.
    pub fn configure(&self, config: Configuration) {
        *self
            .inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = config;
    }

    /// The configuration now.
    pub fn config(&self) -> Configuration {
        self.inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The debounced state, starting with the current one. Starting the platform's change
    /// stream needs a tokio runtime (NSH enters one in its tasks) or the web's event loop;
    /// without one this is `NotAvailable`. A failed `watch` still subscribes, with no events.
    pub async fn subscribe(&self) -> Result<watch::Receiver<NetworkState>, ConnectivityError> {
        self.hub(|hub| hub.states.subscribe()).await
    }

    /// connectivity_plus's stream: distinct, never-empty lists, starting with the current one.
    pub async fn on_connectivity_changed(&self) -> Result<ConnectivityStream, ConnectivityError> {
        let rx = self.hub(|hub| hub.types.subscribe()).await?;
        Ok(ConnectivityStream { rx, last: None })
    }

    /// Cancels in-flight work, the platform watch and the event loop; subscriptions end.
    /// Idempotent. Later calls start afresh.
    pub fn stop(&self) {
        self.inner
            .stop
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        let hub = self.shared().hub.take();
        if hub.is_some_and(|hub| hub.watching.load(Ordering::SeqCst)) {
            self.inner.backend.stop_watching();
        }
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.inner.shared()
    }

    /// Runs `work` unless `stop` lands first, which resolves `Cancelled`.
    async fn guarded<T>(
        &self,
        work: impl Future<Output = Result<T, ConnectivityError>>,
    ) -> Result<T, ConnectivityError> {
        let mut stop = self.inner.stop.subscribe();
        tokio::select! {
            biased;
            _ = stop.changed() => Err(ErrorKind::Cancelled.into()),
            result = work => result,
        }
    }

    /// One snapshot and, when something is live, one probe.
    async fn cycle(&self) -> Result<NetworkState, ConnectivityError> {
        let snapshot = self.inner.backend.snapshot().await?;
        if snapshot.types.is_empty() {
            return Ok(self.absorb(&snapshot, None));
        }
        let outcome = self.run_probe().await;
        Ok(self.absorb(&snapshot, Some(outcome)))
    }

    /// The probe, bounded by the configured timeout whatever the probe does.
    async fn run_probe(&self) -> ProbeOutcome {
        let config = self.config();
        let probe = self
            .inner
            .probe
            .probe(&config.reachability_url, config.reachability_timeout);
        tokio::select! {
            outcome = probe => outcome,
            _ = rt::sleep(config.reachability_timeout) => ProbeOutcome::Unreachable,
        }
    }

    /// Records the state a snapshot gives and publishes it when it changed.
    fn absorb(&self, snapshot: &PlatformSnapshot, outcome: Option<ProbeOutcome>) -> NetworkState {
        let mut shared = self.shared();
        let same_path = shared.types.as_deref() == Some(snapshot.types.as_slice());
        let reach = match outcome {
            Some(ProbeOutcome::Reachable) => (Reachability::Reachable, false),
            Some(ProbeOutcome::CaptivePortal) => (Reachability::Unreachable, true),
            Some(ProbeOutcome::Unreachable) => (Reachability::Unreachable, false),
            None if same_path => shared.reach,
            None => (Reachability::Unknown, false),
        };
        let state = state_of(snapshot, reach.0, reach.1);
        shared.reach = reach;
        shared.types = Some(snapshot.types.clone());
        shared.current = Some(state.clone());
        if let Some(hub) = &shared.hub {
            hub.publish(&state, listed(&snapshot.types));
        }
        state
    }

    /// The hub, made with the current state and the platform's change stream on first use.
    /// `take` runs under the lock, so its receiver exists before the loop can look.
    async fn hub<R>(&self, take: impl FnOnce(&Hub) -> R) -> Result<R, ConnectivityError> {
        if let Some(hub) = &self.shared().hub {
            return Ok(take(hub));
        }
        let current = self.state().await?;
        let mut shared = self.shared();
        if let Some(hub) = &shared.hub {
            return Ok(take(hub));
        }
        let (sink, events) = mpsc::unbounded_channel();
        let watching = self.inner.backend.watch(sink).is_ok();
        let types = listed(shared.types.as_deref().unwrap_or_default());
        let hub = Arc::new(Hub {
            states: watch::channel(current).0,
            types: watch::channel(types).0,
            watching: AtomicBool::new(watching),
        });
        let taken = take(&hub);
        if watching {
            let task = event_loop(
                Connectivity {
                    inner: self.inner.clone(),
                },
                hub.clone(),
                events,
                self.inner.stop.subscribe(),
            );
            if rt::spawn(task).is_err() {
                self.inner.backend.stop_watching();
                hub.watching.store(false, Ordering::SeqCst);
            }
        }
        shared.hub = Some(hub);
        Ok(taken)
    }

    /// The loop's hub lost its last subscriber: it retires, unless one arrived meanwhile.
    fn retire(&self, hub: &Arc<Hub>) -> bool {
        let mut shared = self.shared();
        if hub.states.receiver_count() > 0 || hub.types.receiver_count() > 0 {
            return false;
        }
        if shared.hub.as_ref().is_some_and(|h| Arc::ptr_eq(h, hub)) {
            shared.hub = None;
            self.inner.backend.stop_watching();
        }
        true
    }

    /// One debounced change: a fresh snapshot, a probe when asked and something is live, and
    /// one published state.
    async fn on_change(&self) -> Result<(), ConnectivityError> {
        let snapshot = self.inner.backend.snapshot().await?;
        let outcome = if self.config().probe_on_events && !snapshot.types.is_empty() {
            Some(self.run_probe().await)
        } else {
            None
        };
        self.absorb(&snapshot, outcome);
        Ok(())
    }
}

impl Inner {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Hub {
    fn publish(&self, state: &NetworkState, types: Vec<ConnectionType>) {
        self.states.send_if_modified(|now| {
            let changed = now != state;
            if changed {
                *now = state.clone();
            }
            changed
        });
        self.types.send_if_modified(|now| {
            let changed = *now != types;
            if changed {
                *now = types;
            }
            changed
        });
    }

    async fn unsubscribed(&self) {
        self.states.closed().await;
        self.types.closed().await;
    }
}

/// Clears the in-flight refresh when its leader finishes or is dropped.
struct ClearInflight<'a>(&'a Inner);

impl Drop for ClearInflight<'_> {
    fn drop(&mut self) {
        self.0.shared().inflight = None;
    }
}

/// Debounces platform events: each event restarts the window, and when it closes one snapshot
/// (and probe) makes one published state. Ends on `stop`, when the platform closes the stream,
/// or when the last subscriber goes.
async fn event_loop(
    connectivity: Connectivity,
    hub: Arc<Hub>,
    mut events: mpsc::UnboundedReceiver<PlatformEvent>,
    mut stop: watch::Receiver<u64>,
) {
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => return,
            _ = hub.unsubscribed() => {
                if connectivity.retire(&hub) {
                    return;
                }
                continue;
            }
            event = events.recv() => if event.is_none() {
                return;
            },
        }
        let mut open = true;
        loop {
            let debounce = connectivity.config().debounce;
            tokio::select! {
                biased;
                _ = stop.changed() => return,
                event = events.recv() => match event {
                    Some(_) => continue,
                    None => {
                        open = false;
                        break;
                    }
                },
                _ = rt::sleep(debounce) => break,
            }
        }
        tokio::select! {
            biased;
            _ = stop.changed() => return,
            // A failed snapshot publishes nothing; the next event tries again.
            _ = connectivity.on_change() => {}
        }
        if !open {
            return;
        }
    }
}
