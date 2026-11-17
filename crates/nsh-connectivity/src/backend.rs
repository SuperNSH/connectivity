//! The seam between [`crate::Connectivity`] and a platform.

use std::{future::Future, pin::Pin};

use crate::{error::ConnectivityError, types::PlatformSnapshot};

/// A future a backend or a probe returns. Native targets run the event loop on tokio's
/// threads, so it is `Send` there; the web is single-threaded and drops the bound.
#[cfg(not(target_arch = "wasm32"))]
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// A future a backend or a probe returns.
#[cfg(target_arch = "wasm32")]
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A nudge to re-read the snapshot; it carries no data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformEvent;

/// What a platform provides. The core adds debouncing, probing and the never-empty list.
pub trait Backend: Send + Sync {
    /// The platform's view now. Resolves or errors; never hangs.
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>>;

    /// Starts the change stream: one event per native notification into `sink`. An error
    /// means there is no change stream; snapshots still work.
    fn watch(
        &self,
        sink: tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
    ) -> Result<(), ConnectivityError>;

    /// Stops the change stream and drops the sink. Idempotent; no event follows it.
    fn stop_watching(&self);
}
