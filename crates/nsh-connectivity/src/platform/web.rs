//! Web: `navigator.onLine` answers the snapshot, `online`/`offline` events nudge a re-read.
//!
//! The browser cannot name the radio behind `navigator.onLine`, so an online snapshot reports
//! Wi-Fi and reachability stays with the HTTP probe — the parity of react-native-netinfo's web
//! module. The web has no metered signal, so `is_expensive` is always false.

use std::cell::RefCell;
use std::future::ready;

use tokio::sync::mpsc::UnboundedSender;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::backend::{Backend, BackendFuture, PlatformEvent};
use crate::error::{ConnectivityError, ErrorKind};
use crate::types::{ConnectionType, PlatformSnapshot};

pub(crate) struct NativeBackend;

/// What `navigator.onLine` can answer, including that no navigator exists at all.
enum Online {
    Yes,
    No,
    NoNavigator,
}

struct WatchState {
    target: web_sys::EventTarget,
    online: Closure<dyn FnMut()>,
    offline: Closure<dyn FnMut()>,
}

// The JS global these listeners install on is process-wide, and `Closure` is not `Send` under
// threaded wasm, so the state lives in a thread-local instead of the backend struct; the
// single-threaded JS event loop keeps every access race-free.
thread_local! {
    static WATCH: RefCell<Option<WatchState>> = RefCell::new(None);
}

/// The platform has no connectivity API here.
fn not_available() -> ConnectivityError {
    ConnectivityError::new(ErrorKind::NotAvailable)
}

/// The browser's own machinery refused; a JS exception carries no numeric status.
fn backend() -> ConnectivityError {
    ConnectivityError::new(ErrorKind::Backend)
}

/// The snapshot for one moment: online is Wi-Fi (the browser cannot name the radio), offline is
/// nothing live (the core then reports `[None]`), a missing navigator is Unknown. Reachability
/// is the probe's to answer, never this snapshot's.
fn snapshot_for(online: Online) -> PlatformSnapshot {
    let types = match online {
        Online::Yes => vec![ConnectionType::Wifi],
        Online::No => Vec::new(),
        Online::NoNavigator => vec![ConnectionType::Unknown],
    };
    PlatformSnapshot {
        types,
        interface: None, // the browser cannot name an interface
        is_expensive: false,
    }
}

/// Reads `navigator.onLine` off the window, or off the JS global in a worker.
fn read_online() -> Online {
    if let Some(window) = web_sys::window() {
        return if window.navigator().on_line() {
            Online::Yes
        } else {
            Online::No
        };
    }
    let global = js_sys::global();
    let navigator = match js_sys::Reflect::get(&global, &JsValue::from_str("navigator")) {
        Ok(value) if !value.is_null_or_undefined() => value,
        _ => return Online::NoNavigator,
    };
    if let Some(worker_navigator) = navigator.dyn_ref::<web_sys::WorkerNavigator>() {
        return if worker_navigator.on_line() {
            Online::Yes
        } else {
            Online::No
        };
    }
    // A host with a navigator that is neither shape: read the boolean property itself.
    match js_sys::Reflect::get(&navigator, &JsValue::from_str("onLine"))
        .ok()
        .and_then(|value| value.as_bool())
    {
        Some(true) => Online::Yes,
        Some(false) => Online::No,
        None => Online::NoNavigator,
    }
}

/// The window, or in a worker the global scope: the EventTarget `online`/`offline` fire on.
fn event_target() -> Option<web_sys::EventTarget> {
    if let Some(window) = web_sys::window() {
        return Some(window.into());
    }
    js_sys::global().dyn_into::<web_sys::EventTarget>().ok()
}

/// A listener that nudges the core with one `PlatformEvent` per browser event.
fn listener(sink: UnboundedSender<PlatformEvent>) -> Closure<dyn FnMut()> {
    Closure::<dyn FnMut()>::new(move || {
        // A failed send means the core dropped the channel: a cancel, not an error to surface.
        let _ = sink.send(PlatformEvent);
    })
}

fn detach(state: &WatchState) {
    let _ = state
        .target
        .remove_event_listener_with_callback("online", state.online.as_ref().unchecked_ref());
    let _ = state
        .target
        .remove_event_listener_with_callback("offline", state.offline.as_ref().unchecked_ref());
}

/// Removes the installed listeners and drops them; repeating this is a no-op, and no event is
/// delivered after it returns.
fn stop_listening() {
    if let Some(state) = WATCH.with(|watch| watch.borrow_mut().take()) {
        detach(&state);
    }
}

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        // Reading a JS property resolves immediately; it cannot block, so it cannot hang.
        Box::pin(ready(Ok(snapshot_for(read_online()))))
    }

    fn watch(&self, sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        // A second watch replaces the first: the browser offers one global event source.
        stop_listening();
        let target = event_target().ok_or_else(not_available)?;
        let online = listener(sink.clone());
        let offline = listener(sink);

        if target
            .add_event_listener_with_callback("online", online.as_ref().unchecked_ref())
            .is_err()
        {
            return Err(backend());
        }
        if target
            .add_event_listener_with_callback("offline", offline.as_ref().unchecked_ref())
            .is_err()
        {
            // Undo the half-installed watch so a failure leaves nothing registered.
            let _ = target
                .remove_event_listener_with_callback("online", online.as_ref().unchecked_ref());
            return Err(backend());
        }

        WATCH.with(|watch| *watch.borrow_mut() = Some(WatchState { target, online, offline }));
        Ok(())
    }

    fn stop_watching(&self) {
        stop_listening();
    }
}

#[cfg(test)]
mod tests {
    use super::{backend, not_available, snapshot_for, NativeBackend, Online};
    use crate::error::ErrorKind;
    use crate::types::ConnectionType;

    #[test]
    fn online_maps_to_wifi() {
        let snapshot = snapshot_for(Online::Yes);
        assert_eq!(snapshot.types, vec![ConnectionType::Wifi]);
    }

    #[test]
    fn offline_maps_to_nothing_live() {
        assert!(snapshot_for(Online::No).types.is_empty());
    }

    #[test]
    fn missing_navigator_maps_to_unknown() {
        let snapshot = snapshot_for(Online::NoNavigator);
        assert_eq!(snapshot.types, vec![ConnectionType::Unknown]);
    }

    #[test]
    fn snapshot_names_no_interface_and_is_never_expensive() {
        for online in [Online::Yes, Online::No, Online::NoNavigator] {
            let snapshot = snapshot_for(online);
            assert!(snapshot.interface.is_none());
            assert!(!snapshot.is_expensive);
        }
    }

    #[test]
    fn a_missing_event_target_is_not_available() {
        let error = not_available();
        assert_eq!(error.kind(), ErrorKind::NotAvailable);
        assert_eq!(error.status(), None);
        assert_eq!(error.code(), "ERR_CONNECTIVITY_NOT_AVAILABLE");
    }

    #[test]
    fn a_listener_refusal_is_backend_without_status() {
        let error = backend();
        assert_eq!(error.kind(), ErrorKind::Backend);
        assert_eq!(error.status(), None);
        assert_eq!(error.code(), "ERR_CONNECTIVITY_BACKEND");
    }

    #[test]
    fn native_backend_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NativeBackend>();
    }
}
