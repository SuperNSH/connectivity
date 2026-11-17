//! Apple (macOS, iOS): NWPathMonitor through Network.framework's C API, declared by hand
//! because no objc2 crate binds it (docs/decisions.md). A snapshot runs a one-shot monitor
//! and takes its first path; the watch keeps one monitor and nudges on every update.
//!
//! A path names neither the SSID nor addresses, so those stay `None`. Satellite
//! (`nw_path_is_ultra_constrained`) needs macOS/iOS 26 and is not read.

use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use block2::{Block, RcBlock, StackBlock};
use dispatch2::{DispatchQueue, DispatchRetained, DispatchTime};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    backend::{Backend, BackendFuture, PlatformEvent},
    error::{ConnectivityError, ErrorKind},
    types::{ConnectionType, InterfaceInfo, PlatformSnapshot},
};

/// `nw_path_monitor_t`, `nw_path_t` and `nw_interface_t`: opaque os_objects.
type NwObject = *mut c_void;

// <Network/path.h>: nw_path_status_t.
const NW_PATH_STATUS_SATISFIED: u32 = 1;
// <Network/interface.h>: nw_interface_type_t.
const NW_INTERFACE_TYPE_OTHER: u32 = 0;
const NW_INTERFACE_TYPE_WIFI: u32 = 1;
const NW_INTERFACE_TYPE_CELLULAR: u32 = 2;
const NW_INTERFACE_TYPE_WIRED: u32 = 3;
const NW_INTERFACE_TYPE_LOOPBACK: u32 = 4;

/// The first path arrives in milliseconds; this only bounds a monitor that never answers.
const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(5);

// Signatures from the SDK's Network.framework headers (path_monitor.h, path.h, interface.h,
// nw_object.h); all available from macOS 10.14 and iOS 12.
#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_path_monitor_create() -> NwObject;
    fn nw_path_monitor_set_queue(monitor: NwObject, queue: &DispatchQueue);
    fn nw_path_monitor_set_update_handler(monitor: NwObject, handler: &Block<dyn Fn(NwObject)>);
    fn nw_path_monitor_start(monitor: NwObject);
    fn nw_path_monitor_cancel(monitor: NwObject);
    fn nw_path_get_status(path: NwObject) -> u32;
    fn nw_path_is_expensive(path: NwObject) -> bool;
    // The block returns C `bool`; block2 has no encoding for Rust `bool`, so it is a `u8`.
    fn nw_path_enumerate_interfaces<'a>(path: NwObject, block: &Block<dyn Fn(NwObject) -> u8 + 'a>);
    fn nw_interface_get_type(interface: NwObject) -> u32;
    fn nw_interface_get_name(interface: NwObject) -> *const c_char;
    fn nw_release(object: NwObject);
}

pub(crate) struct NativeBackend;

/// A running monitor. Network.framework objects are thread-safe os_objects.
struct Monitor(NwObject);

// SAFETY: os_objects are reference counted and safe to use and release from any thread.
unsafe impl Send for Monitor {}

impl Monitor {
    /// A monitor delivering every path to `on_path` on `queue`, started.
    fn start(
        queue: &DispatchQueue,
        on_path: impl Fn(PlatformSnapshot) + 'static,
    ) -> Result<Monitor, ConnectivityError> {
        // SAFETY: a plain constructor; a null result is checked.
        let raw = unsafe { nw_path_monitor_create() };
        if raw.is_null() {
            return Err(ErrorKind::Backend.into());
        }
        let handler = RcBlock::new(move |path: NwObject| on_path(read_path(path)));
        // SAFETY: `raw` is a live monitor; Network copies the block and retains the queue.
        unsafe {
            nw_path_monitor_set_queue(raw, queue);
            nw_path_monitor_set_update_handler(raw, &handler);
            nw_path_monitor_start(raw);
        }
        Ok(Monitor(raw))
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: cancel stops further updates; the release balances the create.
        unsafe {
            nw_path_monitor_cancel(self.0);
            nw_release(self.0);
        }
    }
}

/// The watch's monitor and the sink it nudges. One per process.
struct Watch {
    _monitor: Monitor,
    sink: Arc<Mutex<Option<UnboundedSender<PlatformEvent>>>>,
}

static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

/// One interface on a path: its name and `nw_interface_type_t`.
type PathInterface = (String, u32);

/// Reads a path inside the update handler, where it is valid.
fn read_path(path: NwObject) -> PlatformSnapshot {
    // SAFETY: Network hands the handler a live path for the duration of the call.
    let satisfied = unsafe { nw_path_get_status(path) } == NW_PATH_STATUS_SATISFIED;
    if !satisfied {
        return PlatformSnapshot::default();
    }
    let interfaces = RefCell::new(Vec::<PathInterface>::new());
    let block = StackBlock::new(|interface: NwObject| {
        // SAFETY: the interface is live during the enumeration; the name is a C string
        // owned by it, copied out here.
        let (name, kind) = unsafe {
            let name = nw_interface_get_name(interface);
            let name = if name.is_null() {
                String::new()
            } else {
                CStr::from_ptr(name).to_string_lossy().into_owned()
            };
            (name, nw_interface_get_type(interface))
        };
        interfaces.borrow_mut().push((name, kind));
        1u8
    });
    // SAFETY: the path is live; the block outlives the synchronous enumeration.
    let expensive = unsafe {
        nw_path_enumerate_interfaces(path, &block);
        nw_path_is_expensive(path)
    };
    snapshot_from(&interfaces.into_inner(), expensive)
}

/// What a satisfied path's interfaces mean, in the path's own preference order.
fn connection_type(name: &str, kind: u32) -> Option<ConnectionType> {
    match kind {
        NW_INTERFACE_TYPE_WIFI => Some(ConnectionType::Wifi),
        NW_INTERFACE_TYPE_CELLULAR => Some(ConnectionType::Cellular),
        NW_INTERFACE_TYPE_WIRED => Some(ConnectionType::Ethernet),
        NW_INTERFACE_TYPE_LOOPBACK => None,
        // Network reports a VPN's tunnel as `other`; its name is the only tell.
        NW_INTERFACE_TYPE_OTHER
            if ["utun", "ipsec", "ppp", "tun", "tap"]
                .iter()
                .any(|prefix| name.starts_with(prefix)) =>
        {
            Some(ConnectionType::Vpn)
        }
        _ => Some(ConnectionType::Other),
    }
}

/// The path's types in its order, with a VPN appended after the path it runs over.
fn snapshot_from(interfaces: &[PathInterface], is_expensive: bool) -> PlatformSnapshot {
    let mut types: Vec<ConnectionType> = Vec::new();
    let mut vpn = false;
    let mut primary = None;
    for (name, kind) in interfaces {
        let Some(connection) = connection_type(name, *kind) else {
            continue;
        };
        if primary.is_none() {
            primary = Some(InterfaceInfo {
                name: name.clone(),
                r#type: connection,
                ip_address: None,
                subnet: None,
                ssid: None,
            });
        }
        if connection == ConnectionType::Vpn {
            vpn = true;
        } else if !types.contains(&connection) {
            types.push(connection);
        }
    }
    if vpn {
        types.push(ConnectionType::Vpn);
    }
    PlatformSnapshot {
        types,
        interface: primary,
        is_expensive,
    }
}

fn lock_watch() -> std::sync::MutexGuard<'static, Option<Watch>> {
    WATCH.lock().unwrap_or_else(PoisonError::into_inner)
}

type Reply = Arc<Mutex<Option<oneshot::Sender<Result<PlatformSnapshot, ConnectivityError>>>>>;

/// Answers the waiting snapshot once; later answers find the slot empty.
fn reply(slot: &Reply, answer: Result<PlatformSnapshot, ConnectivityError>) {
    let sender = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(sender) = sender {
        let _ = sender.send(answer);
    }
}

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        Box::pin(async move {
            let (sender, answer) = oneshot::channel();
            let slot: Reply = Arc::new(Mutex::new(Some(sender)));
            let queue = DispatchQueue::new("dev.supernsh.connectivity.snapshot", None);
            let on_path = slot.clone();
            let monitor = Monitor::start(&queue, move |snapshot| reply(&on_path, Ok(snapshot)))?;
            // The deadline runs on the same serial queue, so exactly one of the two answers.
            let on_deadline = slot.clone();
            let deadline =
                DispatchTime::try_from(SNAPSHOT_DEADLINE).unwrap_or(DispatchTime::FOREVER);
            let _ = queue.after(deadline, move || {
                reply(&on_deadline, Err(ErrorKind::Timeout.into()));
            });
            let result = answer
                .await
                .unwrap_or_else(|_| Err(ErrorKind::Backend.into()));
            drop(monitor);
            drop(queue);
            result
        })
    }

    fn watch(&self, sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        let mut guard = lock_watch();
        // A second watch replaces the first: one monitor per process.
        if let Some(previous) = guard.take() {
            previous
                .sink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
        }
        let shared: Arc<Mutex<Option<UnboundedSender<PlatformEvent>>>> =
            Arc::new(Mutex::new(Some(sink)));
        let on_path = shared.clone();
        let queue: DispatchRetained<DispatchQueue> =
            DispatchQueue::new("dev.supernsh.connectivity.watch", None);
        let monitor = Monitor::start(&queue, move |_snapshot| {
            let sink = on_path.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(sink) = sink.as_ref() {
                let _ = sink.send(PlatformEvent);
            }
        })?;
        *guard = Some(Watch {
            _monitor: monitor,
            sink: shared,
        });
        Ok(())
    }

    fn stop_watching(&self) {
        let watch = lock_watch().take();
        if let Some(watch) = watch {
            // Dropping the sink first means a handler already queued sends nothing.
            watch
                .sink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, kind: u32) -> PathInterface {
        (name.to_owned(), kind)
    }

    #[test]
    fn interface_types_map_to_connection_types() {
        assert_eq!(
            connection_type("en0", NW_INTERFACE_TYPE_WIFI),
            Some(ConnectionType::Wifi)
        );
        assert_eq!(
            connection_type("pdp_ip0", NW_INTERFACE_TYPE_CELLULAR),
            Some(ConnectionType::Cellular)
        );
        assert_eq!(
            connection_type("en5", NW_INTERFACE_TYPE_WIRED),
            Some(ConnectionType::Ethernet)
        );
        assert_eq!(connection_type("lo0", NW_INTERFACE_TYPE_LOOPBACK), None);
        assert_eq!(
            connection_type("utun3", NW_INTERFACE_TYPE_OTHER),
            Some(ConnectionType::Vpn)
        );
        assert_eq!(
            connection_type("bridge0", NW_INTERFACE_TYPE_OTHER),
            Some(ConnectionType::Other)
        );
    }

    #[test]
    fn a_vpn_is_appended_after_its_path() {
        let snapshot = snapshot_from(
            &[
                iface("utun3", NW_INTERFACE_TYPE_OTHER),
                iface("en0", NW_INTERFACE_TYPE_WIFI),
            ],
            false,
        );
        assert_eq!(
            snapshot.types,
            vec![ConnectionType::Wifi, ConnectionType::Vpn]
        );
        assert_eq!(snapshot.interface.expect("primary").name, "utun3");
    }

    #[test]
    fn nothing_but_loopback_is_nothing_live() {
        let snapshot = snapshot_from(&[iface("lo0", NW_INTERFACE_TYPE_LOOPBACK)], true);
        assert!(snapshot.types.is_empty());
        assert!(snapshot.interface.is_none());
        assert!(snapshot.is_expensive);
    }

    #[test]
    fn types_keep_path_order_without_duplicates() {
        let snapshot = snapshot_from(
            &[
                iface("en0", NW_INTERFACE_TYPE_WIFI),
                iface("pdp_ip0", NW_INTERFACE_TYPE_CELLULAR),
                iface("en1", NW_INTERFACE_TYPE_WIFI),
            ],
            true,
        );
        assert_eq!(
            snapshot.types,
            vec![ConnectionType::Wifi, ConnectionType::Cellular]
        );
        assert!(snapshot.is_expensive);
    }

    // Runs the real NWPathMonitor: local only, it sends nothing over the network.
    #[tokio::test]
    async fn snapshot_resolves_once_within_its_deadline() {
        let result = tokio::time::timeout(Duration::from_secs(10), NativeBackend.snapshot())
            .await
            .expect("snapshot resolves");
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn watch_nudges_and_stop_watching_closes_the_stream() {
        let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
        NativeBackend.watch(sink).expect("the monitor starts");
        // The monitor delivers the current path once it starts.
        let first = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;
        assert!(matches!(first, Ok(Some(_))), "no initial update");
        NativeBackend.stop_watching();
        NativeBackend.stop_watching();
        let end = tokio::time::timeout(Duration::from_secs(5), async {
            while events.recv().await.is_some() {}
        })
        .await;
        assert!(end.is_ok(), "the stream did not close");
    }
}
