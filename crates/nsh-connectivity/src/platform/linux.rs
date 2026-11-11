//! Linux: netdev reads the interface table, if-watch streams netlink address events.
//!
//! netdev marks the default-route interface during enumeration (its `gateway` feature), which
//! answers `get_default_interface` without a second pass. Linux exposes no metered hint, so
//! `is_expensive` is always false here.

use std::future::poll_fn;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

use if_watch::tokio::IfWatcher;
use netdev::interface::types::InterfaceType;
use netdev::Interface;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

use crate::backend::{Backend, BackendFuture, PlatformEvent};
use crate::error::{ConnectivityError, ErrorKind};
use crate::types::{ConnectionType, InterfaceInfo, PlatformSnapshot};

pub(crate) struct NativeBackend;

/// A netlink enumeration is local syscalls, but not free; five seconds is far beyond a healthy one.
const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(5);

/// One address watch per process: the platform has one address table, so a later `watch` replaces
/// the earlier one. Holds the loop's stop sender; the loop itself is owned by the runtime.
static WATCH: LazyLock<Mutex<Option<WatchState>>> = LazyLock::new(|| Mutex::new(None));

struct WatchState {
    stop: watch::Sender<bool>,
}

/// An io failure is the platform's own service failing; the errno rides along when there is one.
fn io_error(err: std::io::Error) -> ConnectivityError {
    match err.raw_os_error() {
        Some(errno) => ConnectivityError::new(ErrorKind::Backend).with_status(i64::from(errno)),
        None => ConnectivityError::new(ErrorKind::Backend),
    }
}

/// The connection a live interface provides. Loopback (and anything down) is not connectivity.
fn connection_type(iface: &Interface) -> Option<ConnectionType> {
    if iface.is_loopback() {
        return None;
    }
    match iface.if_type {
        InterfaceType::Ethernet
        | InterfaceType::Ethernet3Megabit
        | InterfaceType::FastEthernetT
        | InterfaceType::FastEthernetFx
        | InterfaceType::GigabitEthernet => Some(ConnectionType::Ethernet),
        InterfaceType::Wireless80211 => Some(ConnectionType::Wifi),
        InterfaceType::Wwan | InterfaceType::Wwanpp | InterfaceType::Wwanpp2 => {
            Some(ConnectionType::Cellular)
        }
        InterfaceType::Wman => Some(ConnectionType::WiMax),
        InterfaceType::Tunnel => Some(ConnectionType::Vpn),
        InterfaceType::Loopback => None,
        // TUN devices report ARPHRD_NONE; netdev's flag heuristic is the only VPN signal for them.
        InterfaceType::Unknown | InterfaceType::UnknownWithValue(_) if iface.is_tun() => {
            Some(ConnectionType::Vpn)
        }
        // DSL, PPP, serial and other odd links carry data without naming a radio; the strongest
        // conventional kind outranks them all below.
        _ => Some(ConnectionType::Other),
    }
}

/// Higher is a stronger connection; a VPN rides on top of an underlying path, so it ranks below
/// the radios that carry it.
fn type_strength(connection: ConnectionType) -> u8 {
    match connection {
        ConnectionType::Ethernet => 80,
        ConnectionType::Wifi => 70,
        ConnectionType::Cellular => 60,
        ConnectionType::Satellite => 50,
        ConnectionType::WiMax => 45,
        ConnectionType::Vpn => 40,
        ConnectionType::Bluetooth => 30,
        ConnectionType::Other => 20,
        ConnectionType::Unknown => 10,
        ConnectionType::None => 0,
    }
}

fn interface_strength(iface: &Interface) -> u8 {
    connection_type(iface).map_or(0, type_strength)
}

/// The interface's addresses. An SSID needs NetworkManager over D-Bus, which netdev does not
/// read, so it stays `None`.
fn interface_info(iface: &Interface) -> InterfaceInfo {
    let (ip_address, subnet) = match (iface.ipv4.first(), iface.ipv6.first()) {
        (Some(v4), _) => (Some(v4.addr().to_string()), Some(v4.netmask().to_string())),
        (None, Some(v6)) => (Some(v6.addr().to_string()), None),
        (None, None) => (None, None),
    };
    InterfaceInfo {
        name: iface.name.clone(),
        r#type: connection_type(iface).unwrap_or(ConnectionType::Unknown),
        ip_address,
        subnet,
        ssid: None,
    }
}

/// The snapshot for one moment: live interfaces only, strongest type first, and the default-route
/// interface (falling back to the strongest live one) as the reported interface.
fn snapshot_from_interfaces(mut interfaces: Vec<Interface>) -> PlatformSnapshot {
    interfaces.retain(|iface| iface.is_up() && iface.is_running() && !iface.is_loopback());
    interfaces.sort_by(|a, b| {
        interface_strength(b)
            .cmp(&interface_strength(a))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut types: Vec<ConnectionType> = Vec::new();
    for iface in &interfaces {
        if let Some(connection) = connection_type(iface) {
            if !types.contains(&connection) {
                types.push(connection);
            }
        }
    }

    let primary = interfaces
        .iter()
        .find(|iface| iface.default)
        .or_else(|| interfaces.first());

    PlatformSnapshot {
        types,
        interface: primary.map(interface_info),
        is_expensive: false,
    }
}

fn enumerate() -> PlatformSnapshot {
    snapshot_from_interfaces(netdev::get_interfaces())
}

fn lock_watch() -> std::sync::MutexGuard<'static, Option<WatchState>> {
    WATCH.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        Box::pin(async move {
            // netdev reads netlink and /sys: blocking work, kept off the async thread and bounded.
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                // No runtime to block: the reads are local and resolve here.
                return Ok(enumerate());
            };
            let join = handle.spawn_blocking(enumerate).map_err(io_error)?;
            match tokio::time::timeout(SNAPSHOT_DEADLINE, join).await {
                Ok(Ok(snapshot)) => Ok(snapshot),
                // The task panicked or was cancelled; a timed-out enumeration also finishes on
                // its own. Either way this call resolves exactly once.
                Ok(Err(_)) => Err(ConnectivityError::new(ErrorKind::Backend)),
                Err(_) => Err(ConnectivityError::new(ErrorKind::Timeout)),
            }
        })
    }

    fn watch(&self, sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        // IfWatcher's socket registers with the tokio reactor, so a runtime must already exist.
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| ConnectivityError::new(ErrorKind::Backend))?;
        let mut watcher = IfWatcher::new().map_err(io_error)?;
        let (stop, mut stop_rx) = watch::channel(false);
        {
            let mut guard = lock_watch();
            if let Some(previous) = guard.replace(WatchState { stop: stop.clone() }) {
                // Wake the replaced loop even if it never sees another event.
                let _ = previous.stop.send(true);
            }
        }
        // The loop forwards one PlatformEvent per if-watch Up/Down event. It sends only while it
        // holds the watch lock and its own stop sender is still registered, so a replaced or
        // stopped loop pushes no further event.
        let task_stop = stop;
        let _watch_loop = handle.spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_rx.changed() => break,
                    event = poll_fn(|cx| watcher.poll_if_event(cx)) => match event {
                        Ok(_) => {
                            let guard = lock_watch();
                            let current = guard
                                .as_ref()
                                .is_some_and(|state| state.stop.same_channel(&task_stop));
                            drop(guard);
                            if !current || sink.send(PlatformEvent).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        Ok(())
    }

    fn stop_watching(&self) {
        if let Some(state) = lock_watch().take() {
            let _ = state.stop.send(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{connection_type, interface_info, io_error, snapshot_from_interfaces, NativeBackend};
    use crate::backend::Backend;
    use crate::error::ErrorKind;
    use crate::types::ConnectionType;
    use netdev::interface::types::InterfaceType;
    use netdev::Interface;
    use std::net::{Ipv4Addr, Ipv6Addr};

    // Linux if(3) flag bits: UP 0x1, LOOPBACK 0x8, POINTOPOINT 0x10, RUNNING 0x40.
    const UP_RUNNING: u32 = 0x1 | 0x40;
    const UP_RUNNING_LOOPBACK: u32 = 0x1 | 0x40 | 0x8;
    const UP_RUNNING_POINT_TO_POINT: u32 = 0x1 | 0x40 | 0x10;

    fn interface(name: &str, if_type: InterfaceType, flags: u32) -> Interface {
        let mut iface = Interface::dummy();
        iface.name = name.to_string();
        iface.if_type = if_type;
        iface.flags = flags;
        iface
    }

    #[test]
    fn ethernet_family_maps_to_ethernet() {
        for if_type in [
            InterfaceType::Ethernet,
            InterfaceType::Ethernet3Megabit,
            InterfaceType::FastEthernetT,
            InterfaceType::FastEthernetFx,
            InterfaceType::GigabitEthernet,
        ] {
            let iface = interface("eth0", if_type, UP_RUNNING);
            assert_eq!(connection_type(&iface), Some(ConnectionType::Ethernet));
        }
    }

    #[test]
    fn wireless80211_maps_to_wifi() {
        let iface = interface("wlan0", InterfaceType::Wireless80211, UP_RUNNING);
        assert_eq!(connection_type(&iface), Some(ConnectionType::Wifi));
    }

    #[test]
    fn wwan_kinds_map_to_cellular() {
        for if_type in [
            InterfaceType::Wwan,
            InterfaceType::Wwanpp,
            InterfaceType::Wwanpp2,
        ] {
            let iface = interface("wwan0", if_type, UP_RUNNING);
            assert_eq!(connection_type(&iface), Some(ConnectionType::Cellular));
        }
    }

    #[test]
    fn wiman_and_tunnel_map_to_wimax_and_vpn() {
        let wimax = interface("wmx0", InterfaceType::Wman, UP_RUNNING);
        assert_eq!(connection_type(&wimax), Some(ConnectionType::WiMax));
        let tunnel = interface("tun1", InterfaceType::Tunnel, UP_RUNNING);
        assert_eq!(connection_type(&tunnel), Some(ConnectionType::Vpn));
    }

    #[test]
    fn unrecognized_kinds_map_to_other() {
        for if_type in [
            InterfaceType::Unknown,
            InterfaceType::UnknownWithValue(65534),
            InterfaceType::Bridge,
            InterfaceType::Ppp,
            InterfaceType::Slip,
        ] {
            let iface = interface("odd0", if_type, UP_RUNNING);
            assert_eq!(connection_type(&iface), Some(ConnectionType::Other));
        }
    }

    #[test]
    fn point_to_point_unknown_interface_maps_to_vpn() {
        let tun = interface(
            "tun0",
            InterfaceType::UnknownWithValue(65534),
            UP_RUNNING_POINT_TO_POINT,
        );
        assert_eq!(connection_type(&tun), Some(ConnectionType::Vpn));
    }

    #[test]
    fn loopback_and_down_interfaces_are_not_connectivity() {
        let lo = interface("lo", InterfaceType::Loopback, UP_RUNNING_LOOPBACK);
        assert_eq!(connection_type(&lo), None);
        let down = interface("wlan0", InterfaceType::Wireless80211, 0);
        let no_carrier = interface("eth9", InterfaceType::Ethernet, 0x1);
        let snapshot = snapshot_from_interfaces(vec![lo, down, no_carrier]);
        assert!(snapshot.types.is_empty());
        assert!(snapshot.interface.is_none());
        assert!(!snapshot.is_expensive);
    }

    #[test]
    fn snapshot_orders_strongest_first_and_dedupes() {
        let wlan1 = interface("wlan1", InterfaceType::Wireless80211, UP_RUNNING);
        let tun = interface(
            "tun0",
            InterfaceType::UnknownWithValue(65534),
            UP_RUNNING_POINT_TO_POINT,
        );
        let eth = interface("eth0", InterfaceType::Ethernet, UP_RUNNING);
        let wlan0 = interface("wlan0", InterfaceType::Wireless80211, UP_RUNNING);
        let snapshot = snapshot_from_interfaces(vec![wlan1, tun, eth, wlan0]);
        assert_eq!(
            snapshot.types,
            vec![
                ConnectionType::Ethernet,
                ConnectionType::Wifi,
                ConnectionType::Vpn
            ]
        );
        // Without a default-route mark, the strongest live interface is the reported one.
        assert_eq!(snapshot.interface.expect("strongest reported").name, "eth0");
    }

    #[test]
    fn snapshot_reports_the_default_route_interface() {
        let mut eth = interface("eth0", InterfaceType::Ethernet, UP_RUNNING);
        let mut wlan = interface("wlan0", InterfaceType::Wireless80211, UP_RUNNING);
        wlan.default = true;
        eth.default = false;
        let snapshot = snapshot_from_interfaces(vec![eth, wlan]);
        assert_eq!(snapshot.types.first(), Some(&ConnectionType::Ethernet));
        let reported = snapshot.interface.expect("default route reported");
        assert_eq!(reported.name, "wlan0");
        assert_eq!(reported.r#type, ConnectionType::Wifi);
    }

    #[test]
    fn interface_info_prefers_ipv4_with_dotted_mask() {
        let mut iface = interface("wlan0", InterfaceType::Wireless80211, UP_RUNNING);
        iface.ipv4 = vec![netdev::ipnet::Ipv4Net::new(Ipv4Addr::new(192, 168, 1, 10), 24)
            .expect("valid test network")];
        let info = interface_info(&iface);
        assert_eq!(info.ip_address.as_deref(), Some("192.168.1.10"));
        assert_eq!(info.subnet.as_deref(), Some("255.255.255.0"));
        assert_eq!(info.ssid, None);
    }

    #[test]
    fn interface_info_reports_ipv6_address_without_mask() {
        let mut iface = interface("eth0", InterfaceType::Ethernet, UP_RUNNING);
        iface.ipv6 = vec![netdev::ipnet::Ipv6Net::new(
            "2001:db8::1".parse::<Ipv6Addr>().expect("valid test address"),
            64,
        )
        .expect("valid test network")];
        let info = interface_info(&iface);
        assert_eq!(info.ip_address.as_deref(), Some("2001:db8::1"));
        assert_eq!(info.subnet, None);
    }

    #[test]
    fn io_errors_map_to_backend_with_the_errno_as_status() {
        let error = io_error(std::io::Error::from_raw_os_error(13));
        assert_eq!(error.kind(), ErrorKind::Backend);
        assert_eq!(error.status(), Some(13));
        assert_eq!(error.code(), "ERR_CONNECTIVITY_BACKEND");

        let unsourced = io_error(std::io::Error::other("netlink socket closed"));
        assert_eq!(unsourced.kind(), ErrorKind::Backend);
        assert_eq!(unsourced.status(), None);
    }

    #[tokio::test]
    async fn snapshot_resolves_once_within_its_deadline() {
        let state = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            NativeBackend.snapshot(),
        )
        .await
        .expect("snapshot resolves exactly once");
        assert!(state.is_ok());
    }

    #[tokio::test]
    async fn watch_forwards_events_and_stop_watching_is_idempotent() {
        let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
        NativeBackend
            .watch(sink)
            .expect("netlink watch needs no privileges");
        // if-watch first delivers the current address table as Up events.
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await;
        assert!(first.is_ok_and(|event| event.is_some()), "no initial event");

        NativeBackend.stop_watching();
        NativeBackend.stop_watching();

        // Draining must reach a closed channel: no event is pushed after stop_watching returns.
        let mut drained = 0;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await {
                Ok(Some(_)) => {
                    drained += 1;
                    assert!(drained < 1024, "event stream never closed");
                }
                Ok(None) => break,
                Err(_) => panic!("event stream did not close after stop_watching"),
            }
        }
    }
}
