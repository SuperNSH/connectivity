//! A target with no connectivity API: every call answers `NotAvailable`, never a panic.

use std::future::ready;

use tokio::sync::mpsc::UnboundedSender;

use crate::{
    backend::{Backend, BackendFuture, PlatformEvent},
    error::{ConnectivityError, ErrorKind},
    types::PlatformSnapshot,
};

pub(crate) struct NativeBackend;

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        Box::pin(ready(Err(ErrorKind::NotAvailable.into())))
    }

    fn watch(&self, _sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        Err(ErrorKind::NotAvailable.into())
    }

    fn stop_watching(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_call_answers_not_available() {
        let backend = NativeBackend;
        let snapshot = backend.snapshot().await.unwrap_err();
        assert_eq!(snapshot.kind(), ErrorKind::NotAvailable);
        let (sink, _events) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            backend.watch(sink).unwrap_err().code(),
            "ERR_CONNECTIVITY_NOT_AVAILABLE"
        );
        backend.stop_watching();
    }
}
