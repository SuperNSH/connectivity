# Parity with `connectivity_plus` and `react-native-netinfo`

Every public API of the two references, and what this library does for each. The names come from the sources, not the READMEs: `.refs/plus_plugins/packages/connectivity_plus/connectivity_plus/lib/connectivity_plus.dart` with its platform interface, and `.refs/react-native-netinfo/src/index.ts` with `src/internal/types.ts`, all read 2026-11-11. Ours is `crates/nsh-connectivity` (Rust) and `packages/nsh-connectivity` (TypeScript).

Status words:

- **Mapped**: the reference API has a counterpart here, named in the row.
- **Omitted**: no counterpart, with the reason. An omitted configuration field is not in `Configuration`; a plain-JS caller that passes one changes nothing.
- **Ours**: no reference API; the row names what this library adds.

What is implemented and which gates ran is in the README's Status section. The testable contract, with a named test for each edge case, is [goals.md](goals.md).

## `Connectivity` (connectivity_plus)

| connectivity_plus | Rust | TypeScript | Status |
|---|---|---|---|
| `Connectivity()` (singleton factory) | `Connectivity::new()` | `new Connectivity()` | Mapped. The handle over the platform backend; `Connectivity::with_backend(backend)` builds one over a fake for tests. connectivity_plus forces a singleton because a second instance overrides its EventChannel; nothing here needs that guard |
| `checkConnectivity()` | `Connectivity::check_connectivity()` | `checkConnectivity()` | Mapped. The live types, strongest first; the list is never empty and is exactly `[none]` when nothing is live |
| `onConnectivityChanged` (stream, distinct, never empty) | `Connectivity::on_connectivity_changed()` -> `ConnectivityStream` | `onConnectivityChanged(listener)`, returns an unsubscribe function | Mapped. Yields distinct, never-empty lists, as connectivity_plus does; ours debounces platform events (`debounce`, default 500 ms) before a change can reach the stream |
| `ConnectivityPlatform` (the platform-interface override point) | the `Backend` trait, plus `Connectivity::with_backend` | the `ConnectivityBackend` interface, plus `memoryBackend` | Mapped |
| `hasConnectivity` (extension on `List<ConnectivityResult>`) | not carried | not carried | Omitted. On a never-empty list where `none` appears only alone, `list != [none]` is the whole check |

## `ConnectivityResult` and `NetInfoStateType`

One set here, `ConnectionType`: the union of both references' values. The TS values are the same lowercase strings netinfo uses, so a comparison such as `state.type === "wifi"` keeps working; the `NetInfoStateType` enum object itself is not exported.

| Reference value | Rust | TS | Status |
|---|---|---|---|
| `wifi` (both) | `ConnectionType::Wifi` | `"wifi"` | Mapped |
| `ethernet` (both) | `ConnectionType::Ethernet` | `"ethernet"` | Mapped |
| `bluetooth` (both) | `ConnectionType::Bluetooth` | `"bluetooth"` | Mapped |
| `mobile` (connectivity_plus) / `cellular` (netinfo) | `ConnectionType::Cellular` | `"cellular"` | Mapped. The two references name the same radio differently; ours takes netinfo's name |
| `none` (both) | `ConnectionType::None` | `"none"` | Mapped. Appears only alone: `[none]` means nothing is live |
| `vpn` (both) | `ConnectionType::Vpn` | `"vpn"` | Mapped. connectivity_plus documents that Apple reports `other` for a VPN path; ours reports the underlying type first and appends `Vpn`, so the radio under a VPN is not hidden |
| `satellite` (connectivity_plus) | `ConnectionType::Satellite` | `"satellite"` | Mapped. connectivity_plus reports it on Apple ultra-constrained paths and Android 15+ `TRANSPORT_SATELLITE`; netinfo has no satellite |
| `other` (both) | `ConnectionType::Other` | `"other"` | Mapped |
| `unknown` (netinfo) | `ConnectionType::Unknown` | `"unknown"` | Mapped. connectivity_plus has no unknown |
| `wimax` (netinfo) | `ConnectionType::WiMax` | `"wimax"` | Mapped. connectivity_plus has no wimax |

## NetInfo functions (react-native-netinfo)

| netinfo | Rust | TypeScript | Status |
|---|---|---|---|
| `configure(configuration)` | `Connectivity::configure(Configuration)` | `configure(Configuration)` | Mapped, with one difference: netinfo's configure tears the state manager down, so previously added listeners stop firing; ours keeps subscriptions and the next read uses the new configuration |
| `fetch(requestedInterface?)` | `Connectivity::fetch(interface: Option<&str>)` | `fetch(interfaceName?)` | Mapped. A name the platform does not know rejects with `ERR_CONNECTIVITY_UNKNOWN_INTERFACE` |
| `refresh()` | `Connectivity::refresh()` | `refresh()` | Mapped, including the coalescing: concurrent callers share one in-flight probe |
| `addEventListener(listener)` -> unsubscribe | `Connectivity::subscribe()` -> `watch::Receiver` | `addEventListener(listener)` -> unsubscribe | Mapped. The first delivery is the current state, then changes |
| `useNetInfo(configuration?)` | — | `useNetInfo(config?)` | Mapped in TS only; a hook is a React thing. The Rust counterpart is `subscribe()` |
| `useNetInfoInstance(isPaused?, configuration?)` | — | — | Omitted. It exists to run an isolated manager and pause it; v1 has one manager and the hook reads it, and no caller needs a paused isolated checker |
| the default export object (`NetInfo.configure`, ...) | — | named exports only | Omitted. NSH packages export names, not a default object, as `nsh-biometrics` does |
| `NetInfoChangeHandler`, `NetInfoSubscription` | — | the callback and unsubscribe types of `addEventListener` | Mapped structurally; no separate aliases |

Rust adds `state()` (the current state without forcing a probe) and `stop()` (cancels the watch and the event loop; idempotent). TS adds `useNetInfo()` and `rustBackend()`; the native bridge carries `stop()` for the Rust side of the seam.

## `NetInfoState` and its details

| netinfo | Rust | TypeScript | Status |
|---|---|---|---|
| `type` | `NetworkState::connection_type` | `connectionType` | Mapped; the values are the `ConnectionType` table above |
| `isConnected: boolean \| null` | `NetworkState::is_connected: bool` | `isConnected: boolean` | Mapped. netinfo types `null` for its `unknown` state; ours is a plain boolean on every state |
| `isInternetReachable: boolean \| null` | `is_internet_reachable: Reachability` | `isInternetReachable: Reachability` | Mapped as an explicit tri-state: `true` -> `"reachable"`, `false` -> `"unreachable"`, `null` -> `"unknown"` |
| `details` (null when disconnected) | `details: Option<NetworkDetails>` | `details: NetworkDetails \| null` | Mapped. `None` exactly when `is_connected` is false, as netinfo does |
| `isWifiEnabled?` | not carried | not carried | Omitted. netinfo marks it optional and fills it on some platforms only; the backends here report what the platform reports |
| `details.isConnectionExpensive` | `NetworkDetails::is_connection_expensive` | `isConnectionExpensive` | Mapped |
| — | `NetworkDetails::captive_portal` | `captivePortal` | Ours. Neither reference has the field: netinfo folds a portal into `isInternetReachable: false`; ours records the probe's `CaptivePortal` outcome here |
| `details.cellularGeneration` (`"2g"`..`"5g"`) | `cellular_generation: Option<CellularGeneration>` | `cellularGeneration` | Mapped, same strings |
| `details.carrier` | `carrier: Option<String>` | `carrier` | Mapped |
| `details.ssid` | `ssid: Option<String>` | `ssid` | Mapped. A missing permission is `None`, not an error; a platform refusal of the whole query is `ERR_CONNECTIVITY_PERMISSION_DENIED` |
| `details.bssid` | `bssid: Option<String>` | `bssid` | Mapped |
| `details.strength` | `strength: Option<u8>` | `strength` | Mapped |
| `details.ipAddress`, `details.subnet` | `ip_address`, `subnet` | `ipAddress`, `subnet` | Mapped |
| `details.frequency` | `frequency_mhz: Option<u32>` | `frequencyMhz` | Mapped |
| `details.linkSpeed` | `link_speed_mbps: Option<u32>` | `linkSpeedMbps` | Mapped |
| `details.rxLinkSpeed`, `details.txLinkSpeed` | not carried | not carried | Omitted. v1 carries one link speed; the receive/transmit split does not cross the Rust seam |

`InterfaceInfo` (name, type, ip address, subnet, ssid) is ours, not a reference name: it is what a backend reports for the current interface, and `fetch(interface)` answers for one named interface the way netinfo's `requestedInterface` does. `PlatformSnapshot` (the live types strongest first, the current `InterfaceInfo`, `is_expensive`) and `PlatformEvent` (a nudge to re-read) are the `Backend` seam, not reference APIs.

## `NetInfoConfiguration`

Ours is `Configuration`: `reachability_url` (default `https://connectivitycheck.gstatic.com/generate_204`), `reachability_timeout` (10 s), `debounce` (500 ms), `probe_on_events` (true). netinfo's default URL is `https://clients3.google.com/generate_204`; both are Google's 204 endpoints.

| netinfo | Ours | Status |
|---|---|---|
| `reachabilityUrl` | `reachability_url` / `reachabilityUrl` | Mapped. The probe GETs this URL |
| `reachabilityMethod` (`"HEAD"` \| `"GET"`; netinfo defaults to HEAD) | not carried | Omitted. The probe is GET-only: 204 endpoints reject HEAD, and one method keeps the seam fixed |
| `reachabilityHeaders` | not carried | Omitted. The probe sends no caller headers in v1 |
| `reachabilityTest` (response -> boolean) | not carried | Omitted. A function-valued option does not cross the Rust seam in v1; the classification is fixed: 204 -> `Reachable`, any other complete response -> `CaptivePortal`, a timeout or transport error -> `Unreachable` |
| `reachabilityRequestTimeout` (15 s) | `reachability_timeout` / `reachabilityTimeoutMs` | Mapped. The one deadline that bounds a probe, on both sides of the seam |
| `reachabilityLongTimeout` (60 s), `reachabilityShortTimeout` (5 s) | not carried | Omitted. netinfo re-probes on a timer, 60 s while reachable and 5 s while not; ours probes on `refresh()` and after a debounced change, so there is no poll interval to set |
| `reachabilityShouldRun` | not carried | Omitted. Function-valued; `probe_on_events` is the non-function knob that decides whether a debounced change triggers a probe |
| `shouldFetchWiFiSSID` (false) | not carried | Omitted. The SSID is part of `details` whenever the platform reports it, and `None` when it cannot be read; there is no separate query for the flag to gate |
| `useNativeReachability` (true) | not carried | Omitted. The 204 classification is implemented on both sides of the seam — `ureq` in the crate on native targets and through the browser's `fetch` on wasm, `fetch` in the TS package's own probe — so there is no side for the flag to choose; netinfo's default, `true`, is the behaviour an app sees |

`debounce` and `probe_on_events` are ours; neither reference debounces platform events.

## Errors

Neither reference has a typed error surface for these queries: netinfo's functions reject only on a broken bridge, and connectivity_plus surfaces platform exceptions. Ours has one: `ConnectivityError` with a stable `code`, a `status` carrying the platform's own code where it gives one, and `asConnectivityError(error)` in TS mapping an unknown throw onto a known code, defaulting to `ERR_CONNECTIVITY_BACKEND`.

| Code | When |
|---|---|
| `ERR_CONNECTIVITY_NOT_AVAILABLE` | the platform has no connectivity API here (an unsupported target, or the web lacks the API) |
| `ERR_CONNECTIVITY_PERMISSION_DENIED` | the OS refused the query, such as Android's missing location permission for SSID |
| `ERR_CONNECTIVITY_CANCELLED` | the caller cancelled: dropping the future, calling stop, or a cancel from the platform |
| `ERR_CONNECTIVITY_TIMEOUT` | an operation gave up at its deadline instead of hanging |
| `ERR_CONNECTIVITY_UNKNOWN_INTERFACE` | `fetch()` named an interface the platform does not have |
| `ERR_CONNECTIVITY_BACKEND` | the platform's own service failed; the error carries the platform status where it gives one |
| `ERR_CONNECTIVITY_INVALID_ARGUMENT` | an argument is empty or not allowed |

## Rust and TypeScript names

The same names modulo case, with one exception: a Rust `Duration` field is a milliseconds number in TS.

| Rust | TypeScript |
|---|---|
| `ConnectionType`, `Reachability`, `CellularGeneration`, `InterfaceInfo`, `NetworkDetails`, `NetworkState`, `PlatformSnapshot`, `PlatformEvent`, `Configuration`, `ProbeOutcome`, `ConnectivityError` | the same names |
| `check_connectivity()`, `on_connectivity_changed()` | `checkConnectivity()`, `onConnectivityChanged()` |
| `fetch()`, `refresh()`, `configure()` | `fetch()`, `refresh()`, `configure()` |
| `subscribe()` | `addEventListener()` |
| `connection_type`, `is_connected`, `is_internet_reachable` | `connectionType`, `isConnected`, `isInternetReachable` |
| `is_connection_expensive`, `cellular_generation`, `ip_address`, `frequency_mhz`, `link_speed_mbps` | `isConnectionExpensive`, `cellularGeneration`, `ipAddress`, `frequencyMhz`, `linkSpeedMbps` |
| `reachability_url` | `reachabilityUrl` |
| `reachability_timeout: Duration`, `debounce: Duration` | `reachabilityTimeoutMs`, `debounceMs` |
| `probe_on_events` | `probeOnEvents` |
| `ErrorKind` | the `code` strings |

Values, Rust variant to TS string: `ConnectionType` — `Unknown` -> `"unknown"`, `None` -> `"none"`, `Cellular` -> `"cellular"`, `Wifi` -> `"wifi"`, `Bluetooth` -> `"bluetooth"`, `Ethernet` -> `"ethernet"`, `WiMax` -> `"wimax"`, `Vpn` -> `"vpn"`, `Satellite` -> `"satellite"`, `Other` -> `"other"`. `Reachability` — `"unknown"`, `"reachable"`, `"unreachable"`. `CellularGeneration` — `Generation2` -> `"2g"` through `Generation5` -> `"5g"`. `ProbeOutcome` — `"reachable"`, `"captivePortal"`, `"unreachable"`.
