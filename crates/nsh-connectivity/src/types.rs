//! The values the API reports: connection types, reachability, the state and its details.

use std::time::Duration;

/// A kind of connection: the union of connectivity_plus's and netinfo's sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConnectionType {
    /// The platform could not tell.
    #[default]
    Unknown,
    /// Nothing is live.
    None,
    /// A cellular radio (connectivity_plus's `mobile`).
    Cellular,
    /// Wi-Fi.
    Wifi,
    /// Bluetooth tethering.
    Bluetooth,
    /// Wired Ethernet.
    Ethernet,
    /// WiMAX.
    WiMax,
    /// A VPN, reported after the path it runs over.
    Vpn,
    /// A satellite link.
    Satellite,
    /// A live link of no kind above.
    Other,
}

impl ConnectionType {
    /// The TS string: `"wifi"`, `"cellular"`, ...
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectionType::Unknown => "unknown",
            ConnectionType::None => "none",
            ConnectionType::Cellular => "cellular",
            ConnectionType::Wifi => "wifi",
            ConnectionType::Bluetooth => "bluetooth",
            ConnectionType::Ethernet => "ethernet",
            ConnectionType::WiMax => "wimax",
            ConnectionType::Vpn => "vpn",
            ConnectionType::Satellite => "satellite",
            ConnectionType::Other => "other",
        }
    }
}

/// Whether the Internet answered the reachability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Reachability {
    /// No probe answer yet.
    #[default]
    Unknown,
    /// The probe got its 204.
    Reachable,
    /// The probe got anything else, or nothing.
    Unreachable,
}

impl Reachability {
    /// The TS string.
    pub fn as_str(self) -> &'static str {
        match self {
            Reachability::Unknown => "unknown",
            Reachability::Reachable => "reachable",
            Reachability::Unreachable => "unreachable",
        }
    }
}

/// A cellular network's generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellularGeneration {
    /// 2G.
    Generation2,
    /// 3G.
    Generation3,
    /// 4G.
    Generation4,
    /// 5G.
    Generation5,
}

impl CellularGeneration {
    /// The TS string: `"2g"` to `"5g"`.
    pub fn as_str(self) -> &'static str {
        match self {
            CellularGeneration::Generation2 => "2g",
            CellularGeneration::Generation3 => "3g",
            CellularGeneration::Generation4 => "4g",
            CellularGeneration::Generation5 => "5g",
        }
    }
}

/// What one reachability probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeOutcome {
    /// HTTP 204.
    Reachable,
    /// Another complete HTTP response: something answered, the Internet did not.
    CaptivePortal,
    /// A timeout or a transport error.
    Unreachable,
}

impl ProbeOutcome {
    /// The TS string.
    pub fn as_str(self) -> &'static str {
        match self {
            ProbeOutcome::Reachable => "reachable",
            ProbeOutcome::CaptivePortal => "captivePortal",
            ProbeOutcome::Unreachable => "unreachable",
        }
    }
}

/// The interface a backend reports for the current path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceInfo {
    /// The OS's name for it, such as `en0` or `wlan0`.
    pub name: String,
    /// Its connection type.
    pub r#type: ConnectionType,
    /// Its address, when the backend reads one.
    pub ip_address: Option<String>,
    /// Its subnet mask, when the backend reads one.
    pub subnet: Option<String>,
    /// The Wi-Fi network's name; `None` when unread or not permitted.
    pub ssid: Option<String>,
}

/// What is known about a live connection (netinfo's `details`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkDetails {
    /// The path is metered or otherwise expensive.
    pub is_connection_expensive: bool,
    /// The last probe got a non-204 answer.
    pub captive_portal: bool,
    /// The Wi-Fi network's name.
    pub ssid: Option<String>,
    /// The access point's BSSID.
    pub bssid: Option<String>,
    /// Signal strength, 0 to 100.
    pub strength: Option<u8>,
    /// The interface's address.
    pub ip_address: Option<String>,
    /// The interface's subnet mask.
    pub subnet: Option<String>,
    /// The Wi-Fi frequency.
    pub frequency_mhz: Option<u32>,
    /// The link speed.
    pub link_speed_mbps: Option<u32>,
    /// The cellular generation.
    pub cellular_generation: Option<CellularGeneration>,
    /// The cellular carrier.
    pub carrier: Option<String>,
}

/// The network as an app sees it (netinfo's `NetInfoState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkState {
    /// The strongest live connection, or `None`.
    pub connection_type: ConnectionType,
    /// Something is live.
    pub is_connected: bool,
    /// The latest probe answer.
    pub is_internet_reachable: Reachability,
    /// `None` exactly when `is_connected` is false.
    pub details: Option<NetworkDetails>,
}

impl NetworkState {
    /// Nothing live.
    pub fn disconnected() -> Self {
        NetworkState {
            connection_type: ConnectionType::None,
            is_connected: false,
            is_internet_reachable: Reachability::Unreachable,
            details: None,
        }
    }
}

/// What a backend reports for one moment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlatformSnapshot {
    /// The live types, strongest first; empty means nothing is live.
    pub types: Vec<ConnectionType>,
    /// The current interface, when the backend names one.
    pub interface: Option<InterfaceInfo>,
    /// The path is metered.
    pub is_expensive: bool,
}

/// How the probe and the event pipeline behave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// The URL the probe GETs; 204 means reachable.
    pub reachability_url: String,
    /// The probe's deadline.
    pub reachability_timeout: Duration,
    /// How long platform events must stay quiet before the state is re-read.
    pub debounce: Duration,
    /// Probe after each debounced change.
    pub probe_on_events: bool,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            reachability_url: "https://connectivitycheck.gstatic.com/generate_204".to_owned(),
            reachability_timeout: Duration::from_secs(10),
            debounce: Duration::from_millis(500),
            probe_on_events: true,
        }
    }
}

/// The never-empty list connectivity_plus reports: `[None]` alone when nothing is live.
pub(crate) fn listed(types: &[ConnectionType]) -> Vec<ConnectionType> {
    if types.is_empty() {
        vec![ConnectionType::None]
    } else {
        types.to_vec()
    }
}

/// The state a snapshot gives, with the reachability known for it.
pub(crate) fn state_of(
    snapshot: &PlatformSnapshot,
    reachability: Reachability,
    captive_portal: bool,
) -> NetworkState {
    let Some(&connection_type) = snapshot.types.first() else {
        return NetworkState::disconnected();
    };
    let interface = snapshot.interface.as_ref();
    NetworkState {
        connection_type,
        is_connected: true,
        is_internet_reachable: reachability,
        details: Some(NetworkDetails {
            is_connection_expensive: snapshot.is_expensive,
            captive_portal,
            ssid: interface.and_then(|i| i.ssid.clone()),
            ip_address: interface.and_then(|i| i.ip_address.clone()),
            subnet: interface.and_then(|i| i.subnet.clone()),
            ..NetworkDetails::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_is_never_empty() {
        assert_eq!(listed(&[]), vec![ConnectionType::None]);
        assert_eq!(
            listed(&[ConnectionType::Wifi, ConnectionType::Vpn]),
            vec![ConnectionType::Wifi, ConnectionType::Vpn]
        );
    }

    #[test]
    fn defaults_match_the_contract() {
        let config = Configuration::default();
        assert_eq!(
            config.reachability_url,
            "https://connectivitycheck.gstatic.com/generate_204"
        );
        assert_eq!(config.reachability_timeout, Duration::from_secs(10));
        assert_eq!(config.debounce, Duration::from_millis(500));
        assert!(config.probe_on_events);
    }

    #[test]
    fn wire_strings_match_the_ts_values() {
        assert_eq!(ConnectionType::WiMax.as_str(), "wimax");
        assert_eq!(ProbeOutcome::CaptivePortal.as_str(), "captivePortal");
        assert_eq!(CellularGeneration::Generation5.as_str(), "5g");
        assert_eq!(Reachability::Unknown.as_str(), "unknown");
    }
}
