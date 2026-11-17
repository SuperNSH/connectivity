//! Windows: `Windows.Networking.Connectivity.NetworkInformation`. The connection profiles
//! answer the snapshot and `NetworkStatusChanged` the events. A failed WinRT call is `Backend`
//! with its HRESULT as the status.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::mpsc::UnboundedSender;
use windows::Networking::Connectivity::{
    ConnectionProfile, NetworkConnectivityLevel, NetworkCostType, NetworkInformation,
    NetworkStatusChangedEventHandler,
};

use crate::{
    backend::{Backend, BackendFuture, PlatformEvent},
    error::{ConnectivityError, ErrorKind},
    types::{ConnectionType, InterfaceInfo, PlatformSnapshot},
};

pub(crate) struct NativeBackend;

// IANA ifType values NetworkAdapter.IanaInterfaceType reports.
const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_PROP_VIRTUAL: u32 = 53;
const IF_TYPE_IEEE80211: u32 = 71;
const IF_TYPE_TUNNEL: u32 = 131;
const IF_TYPE_WWANPP: u32 = 243;
const IF_TYPE_WWANPP2: u32 = 244;

/// The registered handler's token and the sink it nudges.
struct Watch {
    token: i64,
    sink: Arc<Mutex<Option<UnboundedSender<PlatformEvent>>>>,
}

static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

fn hresult(error: windows::core::Error) -> ConnectivityError {
    ConnectivityError::new(ErrorKind::Backend).with_status(i64::from(error.code().0))
}

/// What an adapter's IANA type means.
fn iana_type(if_type: u32) -> ConnectionType {
    match if_type {
        IF_TYPE_ETHERNET => ConnectionType::Ethernet,
        IF_TYPE_IEEE80211 => ConnectionType::Wifi,
        IF_TYPE_WWANPP | IF_TYPE_WWANPP2 => ConnectionType::Cellular,
        IF_TYPE_TUNNEL | IF_TYPE_PPP | IF_TYPE_PROP_VIRTUAL => ConnectionType::Vpn,
        _ => ConnectionType::Other,
    }
}

/// Higher is stronger; a VPN sorts after the path it runs over.
fn strength(connection: ConnectionType) -> u8 {
    match connection {
        ConnectionType::Ethernet => 6,
        ConnectionType::Wifi => 5,
        ConnectionType::Cellular => 4,
        ConnectionType::Other => 2,
        ConnectionType::Vpn => 1,
        _ => 0,
    }
}

/// Live types strongest first, without duplicates.
fn ordered(mut types: Vec<ConnectionType>) -> Vec<ConnectionType> {
    types.sort_by_key(|t| std::cmp::Reverse(strength(*t)));
    types.dedup();
    types
}

fn profile_type(profile: &ConnectionProfile) -> Result<ConnectionType, windows::core::Error> {
    if profile.IsWlanConnectionProfile()? {
        return Ok(ConnectionType::Wifi);
    }
    if profile.IsWwanConnectionProfile()? {
        return Ok(ConnectionType::Cellular);
    }
    Ok(iana_type(profile.NetworkAdapter()?.IanaInterfaceType()?))
}

fn read() -> Result<PlatformSnapshot, windows::core::Error> {
    let profiles = NetworkInformation::GetConnectionProfiles()?;
    let mut types = Vec::new();
    for index in 0..profiles.Size()? {
        let profile = profiles.GetAt(index)?;
        if profile.GetNetworkConnectivityLevel()? == NetworkConnectivityLevel::None {
            continue;
        }
        types.push(profile_type(&profile)?);
    }
    // No Internet profile is not an error: nothing reaches the Internet.
    let Ok(internet) = NetworkInformation::GetInternetConnectionProfile() else {
        return Ok(PlatformSnapshot {
            types: ordered(types),
            interface: None,
            is_expensive: false,
        });
    };
    let cost = internet.GetConnectionCost()?;
    let cost_type = cost.NetworkCostType()?;
    let metered = cost_type == NetworkCostType::Fixed || cost_type == NetworkCostType::Variable;
    let is_expensive = metered || cost.Roaming()?;
    let kind = profile_type(&internet)?;
    // A missing SSID (no Wi-Fi, no location consent) is None, not an error.
    let ssid = if internet.IsWlanConnectionProfile()? {
        internet
            .WlanConnectionProfileDetails()
            .and_then(|details| details.GetConnectedSsid())
            .ok()
            .map(|ssid| ssid.to_string())
            .filter(|ssid| !ssid.is_empty())
    } else {
        None
    };
    Ok(PlatformSnapshot {
        types: ordered(types),
        interface: Some(InterfaceInfo {
            name: internet.ProfileName()?.to_string(),
            r#type: kind,
            ip_address: None,
            subnet: None,
            ssid,
        }),
        is_expensive,
    })
}

fn lock_watch() -> std::sync::MutexGuard<'static, Option<Watch>> {
    WATCH.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        Box::pin(async {
            crate::rt::blocking(|| read().map_err(hresult))
                .await
                .unwrap_or_else(|| Err(ErrorKind::Backend.into()))
        })
    }

    fn watch(&self, sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        self.stop_watching();
        let shared = Arc::new(Mutex::new(Some(sink)));
        let on_change = shared.clone();
        let handler = NetworkStatusChangedEventHandler::new(move |_sender| {
            let sink = on_change.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(sink) = sink.as_ref() {
                let _ = sink.send(PlatformEvent);
            }
            Ok(())
        });
        let token = NetworkInformation::NetworkStatusChanged(&handler).map_err(hresult)?;
        *lock_watch() = Some(Watch {
            token,
            sink: shared,
        });
        Ok(())
    }

    fn stop_watching(&self) {
        let watch = lock_watch().take();
        if let Some(watch) = watch {
            watch
                .sink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let _ = NetworkInformation::RemoveNetworkStatusChanged(watch.token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iana_types_map_to_connection_types() {
        assert_eq!(iana_type(6), ConnectionType::Ethernet);
        assert_eq!(iana_type(71), ConnectionType::Wifi);
        assert_eq!(iana_type(243), ConnectionType::Cellular);
        assert_eq!(iana_type(131), ConnectionType::Vpn);
        assert_eq!(iana_type(1), ConnectionType::Other);
    }

    #[test]
    fn types_sort_strongest_first_with_vpn_last() {
        assert_eq!(
            ordered(vec![
                ConnectionType::Vpn,
                ConnectionType::Wifi,
                ConnectionType::Ethernet,
                ConnectionType::Wifi
            ]),
            vec![
                ConnectionType::Ethernet,
                ConnectionType::Wifi,
                ConnectionType::Vpn
            ]
        );
    }

    #[test]
    fn a_failed_call_carries_its_hresult() {
        let error = hresult(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x8007_0005_u32 as i32,
        )));
        assert_eq!(error.kind(), ErrorKind::Backend);
        assert_eq!(error.status(), Some(0x8007_0005_u32 as i32 as i64));
    }

    #[tokio::test]
    async fn snapshot_resolves_once() {
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(10), NativeBackend.snapshot())
                .await
                .expect("snapshot resolves");
        assert!(result.is_ok(), "{result:?}");
    }
}
