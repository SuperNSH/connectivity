# Goals

Network status, change events, and real internet reachability, delivered as one Rust crate (`crates/nsh-connectivity`) and one TypeScript package (`packages/nsh-connectivity`) with the same API names and the same behavior. The layout follows the biometrics library (local clone under `.refs/biometrics`): one crate, one package, integration tests in both trees, one native module bridging them.

The API unions the two references this library is modeled on:

- connectivity_plus: `checkConnectivity()` and `onConnectivityChanged` — the returned list is never empty, `none` appears only and exactly when nothing is live, and the stream is distinct.
- react-native-netinfo: `configure`, `fetch`, `refresh`, `addEventListener`, `useNetInfo` — per-interface fetch and a configuration-driven reachability probe.

Reachability is probed, not inferred: a GET against a 204 endpoint on top of the radio status, because interface up is not internet up (captive portals, VPNs).

Deltas fixed by this contract: connectivity_plus's `mobile` is `cellular` here; netinfo's `boolean | null` reachability becomes the tri-state `unknown | reachable | unreachable`; platform events are debounced (neither reference debounces); the probe is GET-only in v1, so netinfo's `reachabilityMethod` and `reachabilityTest` knobs are not carried over. The full per-API mapping of both references is parity.md.

Platforms in scope: Apple, macOS and iOS (NWPathMonitor via objc2-network); Android (ConnectivityManager over JNI with ndk-context); Windows (Windows.Networking.Connectivity via the windows crate); Linux (netdev for inspection, if-watch for events); web (navigator.onLine plus the fetch probe). Every other target compiles the unsupported backend and answers `ERR_CONNECTIVITY_NOT_AVAILABLE`. CI compiles all five bridges — the host matrix plus cross-checks — and per AGENTS.md nothing in this file is verified until a CI run on `master` passes it. Status lives in parity.md, not here.

## Acceptance criteria

Done is what the gates say, nothing else:

1. `cargo fmt --all -- --check` — clean.
2. `cargo clippy --all-targets -- -D warnings` — clean on the host.
3. `cargo test --all-targets` — passes, including every Rust test named in [Edge cases](#edge-cases).
4. `npm --prefix packages/nsh-connectivity run typecheck` — passes.
5. `npm --prefix packages/nsh-connectivity test` — passes, including every TS test named in [Edge cases](#edge-cases).
6. `.github/workflows/ci.yml` runs both toolchains on push: the format job; clippy and tests on ubuntu, macos, and windows (each host compiles its own bridge); cross-check clippy for `aarch64-apple-ios`, `aarch64-linux-android`, and `wasm32-unknown-unknown`; the TypeScript job.

Every edge case below names its test in both languages, and the suites use those exact names. A case that turns out to be inexpressible in one language's integration suite is recorded here with the reason before the suite lands; there is currently no such case.

## Public API contract

Same names modulo case: Rust `snake_case`, TS `camelCase`.

### Types

Rust:

```rust
enum ConnectionType { Unknown, None, Cellular, Wifi, Bluetooth, Ethernet, WiMax, Vpn, Satellite, Other }
enum Reachability { Unknown, Reachable, Unreachable }
enum CellularGeneration { Generation2, Generation3, Generation4, Generation5 }
enum ProbeOutcome { Reachable, CaptivePortal, Unreachable }

struct InterfaceInfo {
    name: String,
    r#type: ConnectionType,
    ip_address: Option<String>,
    subnet: Option<String>,
    ssid: Option<String>,
}

struct NetworkDetails {
    is_connection_expensive: bool,
    captive_portal: bool,
    ssid: Option<String>,
    bssid: Option<String>,
    strength: Option<u8>,
    ip_address: Option<String>,
    subnet: Option<String>,
    frequency_mhz: Option<u32>,
    link_speed_mbps: Option<u32>,
    cellular_generation: Option<CellularGeneration>,
    carrier: Option<String>,
}

struct NetworkState {
    connection_type: ConnectionType,
    is_connected: bool,
    is_internet_reachable: Reachability,
    details: Option<NetworkDetails>,
}

struct PlatformSnapshot {
    types: Vec<ConnectionType>, // strongest first; empty means nothing is live
    interface: Option<InterfaceInfo>,
    is_expensive: bool,
}

struct Configuration {
    reachability_url: String,       // default "https://connectivitycheck.gstatic.com/generate_204"
    reachability_timeout: Duration, // default 10 s
    debounce: Duration,             // default 500 ms
    probe_on_events: bool,          // default true
}
```

TS mirrors with `null` for `Option` and ms-valued numbers: `reachabilityUrl`, `reachabilityTimeoutMs` (10000), `debounceMs` (500), `probeOnEvents` (true).

- `ConnectionType` "none" (no radio) is distinct from `Reachability` "unknown" (no probe answer yet).
- Ordering: strongest type first; a VPN is appended after the underlying type, so a VPN over Wi-Fi reports `[wifi, vpn]`.
- `details` is `None`/`null` exactly when `is_connected` is false (netinfo parity). A missing SSID without permission is `None`, not an error (netinfo parity).

Snapshot-to-state rules:

1. `types` empty: `connection_type` none, `is_connected` false, `details` none, `is_internet_reachable` unreachable.
2. `types` non-empty: `connection_type` is `types[0]`; `is_connected` true; `details` some; `is_internet_reachable` starts unknown and holds the latest probe answer.
3. Probe reachable: reachable. Probe captivePortal: unreachable and `details.captive_portal` true. Probe unreachable: unreachable.
4. Snapshot `is_expensive` sets `details.is_connection_expensive`.
5. `fetch(Some(name))`: the interfaces a backend knows are the ones its snapshot names in `PlatformSnapshot.interface`; any other name is the `UnknownInterface` error. `fetch(None)` answers the aggregate state.

The probe GETs `reachability_url`: HTTP 204 means reachable; any other complete HTTP response means captivePortal (a portal answers, the Internet does not); a timeout or a transport error means unreachable. It is bounded twice — `tokio::time::timeout` plus the client's own timeout (ureq with rustls on native targets, `fetch` with an `AbortController` on the web) — and resolves exactly once on every path.

### Rust crate

All methods resolve exactly once: success, a coded error, or a cancel. Never a hang.

| Signature | Semantics |
|---|---|
| `Connectivity::new() -> Connectivity` | uses the platform backend |
| `Connectivity::with_backend(backend) -> Connectivity` | for tests and fakes |
| `check_connectivity() -> Result<Vec<ConnectionType>, ConnectivityError>` | connectivity_plus parity: the list is never empty; exactly `[None]` when nothing is live |
| `fetch(interface: Option<&str>) -> Result<NetworkState, ConnectivityError>` | netinfo parity; a name the platform does not know is `UnknownInterface` |
| `refresh() -> Result<NetworkState, ConnectivityError>` | forces a fresh probe; concurrent callers share one in-flight probe |
| `state() -> Result<NetworkState, ConnectivityError>` | the current state without forcing a probe |
| `configure(config: Configuration)` | the next read uses the new configuration |
| `subscribe() -> Result<watch::Receiver<NetworkState>, ConnectivityError>` | the debounced state, starting with the current one |
| `on_connectivity_changed() -> ConnectivityStream` | yields distinct, never-empty `Vec<ConnectionType>` lists |
| `stop()` | cancels the watch and the event loop; idempotent; resolves exactly once |

Event pipeline:

1. `watch` starts the platform's change stream, one `PlatformEvent` per native notification. A `watch` error means there is no change stream; the core still serves snapshots.
2. An event opens a debounce window; further events inside it restart the window. When the window closes, the core re-reads `snapshot()` (`PlatformEvent` is a nudge, not data), rebuilds the state, publishes it, and — when `probe_on_events` — runs one probe.
3. Work in flight when `stop()` lands resolves once with `Cancelled`; a dropped subscription stops cleanly.

The platform seam (`backend.rs`):

```rust
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug, Clone, Copy)]
pub struct PlatformEvent; // a nudge to re-read a snapshot

pub trait Backend: Send + Sync {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>>;
    fn watch(&self, sink: tokio::sync::mpsc::UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError>;
    fn stop_watching(&self);
}
```

`snapshot` resolves or errors, never hangs. `stop_watching` is idempotent; after it returns no more events are pushed. Backends: apple (macOS, iOS), android, windows, linux, web (wasm32), and unsupported for everything else, which answers `NotAvailable`.

The test fake (`fake.rs`): `FakeBackend::new()` is a device on Wi-Fi, connected; `set_snapshot(Result<PlatformSnapshot, ConnectivityError>)`, `set_watch_error(Option<ConnectivityError>)` fails the next `watch`, `emit_event() -> bool` (true when a watcher existed), `watchers() -> usize`, `stopped() -> usize`, `snapshots() -> usize`. Clones share one state.

### TypeScript package

Exports from `src/index.ts`:

| Export | Semantics |
|---|---|
| `configure(config: Partial<Configuration>): void` | merges over the defaults; the next read uses it |
| `fetch(interfaceName?: string): Promise<NetworkState>` | unknown name rejects `ERR_CONNECTIVITY_UNKNOWN_INTERFACE`; empty string rejects `ERR_CONNECTIVITY_INVALID_ARGUMENT` |
| `refresh(): Promise<NetworkState>` | forces a fresh probe; concurrent calls share one in-flight probe |
| `addEventListener(listener) -> unsubscribe` | debounced states; the first delivery is the current state |
| `useNetInfo(config?) -> NetworkState` | the same stream as a React hook; unsubscribes on unmount, never loops |
| `class Connectivity` | `checkConnectivity(): Promise<ConnectionType[]>` (never empty, `["none"]` alone when nothing is live) and `onConnectivityChanged(listener) -> unsubscribe` (distinct, never-empty lists) |

`ConnectivityError`, `asConnectivityError`, and every type above are re-exported from the package root. Configuration carries data only; function-valued netinfo knobs (`reachabilityTest`, `reachabilityShouldRun`) do not cross the Rust seam — parity.md records the full mapping.

Test and platform seams:

- `interface ConnectivityBackend { snapshot(): Promise<PlatformSnapshot>; watch(sink: (event: PlatformEvent) => void): Promise<void>; stopWatching(): void }` — a `watch` rejection is the watch error.
- `memoryBackend()` mirrors `FakeBackend` (`setSnapshot`, `emitEvent`, `watchers`, `stopped`, `snapshots`) and drives the TS suite.
- `rustBackend()` wraps `new NativeConnectivity()` from `nsh-connectivity/native`, mapping rejections through `asConnectivityError`. The native module's members are camelCase: `checkConnectivity`, `fetch`, `refresh`, `configure`, `subscribe`, `stop`; rejections carry the stable code.

### Layout

```
crates/nsh-connectivity/src/    error.rs types.rs backend.rs fake.rs probe.rs lib.rs native.rs
                                platform/{mod,apple,android,windows,linux,web,unsupported}.rs
crates/nsh-connectivity/tests/  one named Rust test per edge case
packages/nsh-connectivity/src/  errors.ts types.ts backend.ts memory.ts probe.ts state.ts index.ts rust.ts native.d.ts
packages/nsh-connectivity/tests/ one named TS test per edge case
```

## Errors

One code per failure mode, identical wording in both languages.

| Kind (Rust variant) | Code | Message |
|---|---|---|
| `NotAvailable` | `ERR_CONNECTIVITY_NOT_AVAILABLE` | The platform has no connectivity API here (an unsupported target, or the web lacks the API). |
| `PermissionDenied` | `ERR_CONNECTIVITY_PERMISSION_DENIED` | The OS refused the query, such as Android's missing location permission for SSID. |
| `Cancelled` | `ERR_CONNECTIVITY_CANCELLED` | The caller cancelled: dropping the future, calling stop, or a cancel from the platform. |
| `Timeout` | `ERR_CONNECTIVITY_TIMEOUT` | The operation gave up at its deadline instead of hanging. |
| `UnknownInterface` | `ERR_CONNECTIVITY_UNKNOWN_INTERFACE` | fetch() named an interface the platform does not have. |
| `Backend` | `ERR_CONNECTIVITY_BACKEND` | The platform's own service failed; the error carries the platform status where it gives one. |
| `InvalidArgument` | `ERR_CONNECTIVITY_INVALID_ARGUMENT` | An argument is empty or not allowed. |

Rust: `ConnectivityError` (thiserror) holds `kind: ErrorKind` and `status: Option<i64>`, with `new()`, `with_status()`, `kind()`, `status()`, `code()`; `ErrorKind` has `code()` and `message()` (the table wording); `Display` prints the code.

TS: `class ConnectivityError extends Error` with `readonly code`; a record of messages with the same wording; `asConnectivityError(error)` maps an unknown throw onto a known code, defaulting to `ERR_CONNECTIVITY_BACKEND`.

Every row maps to at least one test below: NotAvailable to 16, PermissionDenied to 7, Cancelled to 23, Timeout to 25, UnknownInterface to 11, Backend to 26, InvalidArgument to 24.

## Edge cases

Both languages drive the public API through the fakes: `FakeBackend` in Rust, `memoryBackend` in TS. Reachability is scripted, never live — a local in-process HTTP server in Rust, a mocked global `fetch` in TS — so no test depends on the real Internet. Test names are exact and stable; each row is one test per language, in `crates/nsh-connectivity/tests/` and `packages/nsh-connectivity/tests/`.

| # | Case | Must hold | Rust test | TS test |
|---|---|---|---|---|
| 1 | Interface up is not internet up | A connected snapshot whose probe gets a complete non-204 response reports `details.captive_portal` true and `is_internet_reachable` unreachable; `is_connected` stays true. | `interface_up_is_not_internet_up_captive_portal` | `interfaceUpIsNotInternetUpCaptivePortal` |
| 2 | Probe returns 204 | The probe classifies reachable and the state reports `is_internet_reachable` reachable. | `probe_204_means_reachable` | `probe204MeansReachable` |
| 3 | Probe times out | A stalled answer outlives `reachability_timeout`; the probe classifies unreachable and the pending `refresh` resolves exactly once, Ok with unreachable. | `probe_timeout_resolves_once_unreachable` | `probeTimeoutResolvesOnceUnreachable` |
| 4 | Probe transport error | A refused connection classifies unreachable; it is not an error state. | `probe_transport_error_is_unreachable` | `probeTransportErrorIsUnreachable` |
| 5 | Flapping events debounce | Two events inside one debounce window publish at most one state; the last snapshot wins. | `debounce_coalesces_flapping_events` | `debounceCoalescesFlappingEvents` |
| 6 | SSID without location permission | A connected Wi-Fi snapshot with ssid none fetches Ok and reports `details.ssid` none — silence, not an error (Android 12+ needs location permission and FLAG_INCLUDE_LOCATION_INFO to fill it). | `ssid_is_none_without_location_permission` | `ssidIsNoneWithoutLocationPermission` |
| 7 | SSID query refused | A backend erroring PermissionDenied surfaces code `ERR_CONNECTIVITY_PERMISSION_DENIED`. | `ssid_query_refusal_is_permission_denied` | `ssidQueryRefusalIsPermissionDenied` |
| 8 | Throttled late burst | Three events delivered together (background throttling) debounce into one published state and, with `probe_on_events`, one probe. | `late_event_burst_debounces_to_one_state` | `lateEventBurstDebouncesToOneState` |
| 9 | VPN alongside the underlying path | Snapshot types `[wifi, vpn]` give `check_connectivity` `[wifi, vpn]` (underlying first, Vpn appended) and `fetch().connection_type` wifi. | `vpn_reported_alongside_underlying_path` | `vpnReportedAlongsideUnderlyingPath` |
| 10 | checkConnectivity never empty | A snapshot with nothing live yields exactly `[ConnectionType::None]` / `["none"]` — never an empty list. | `check_connectivity_never_empty_none_alone` | `checkConnectivityNeverEmptyNoneAlone` |
| 11 | fetch of an unknown interface | `fetch` with a name the platform does not know errors `ERR_CONNECTIVITY_UNKNOWN_INTERFACE`. | `fetch_unknown_interface_errors` | `fetchUnknownInterfaceErrors` |
| 12 | Concurrent refresh | Concurrent `refresh` calls share one in-flight cycle — one backend snapshot (`snapshots() == 1`) and one probe — and every caller resolves the same state. | `concurrent_refresh_shares_one_probe` | `concurrentRefreshSharesOneProbe` |
| 13 | configure changes the next read | After `configure`, the next `refresh` uses the new values (a swapped `reachability_url` is what the scripted server sees next); work already in flight is untouched. | `configure_changes_next_read` | `configureChangesNextRead` |
| 14 | subscribe returns the current state first | The first value a subscriber receives is the state that existed at subscribe time. | `subscribe_yields_current_state_first` | `subscribeYieldsCurrentStateFirst` |
| 15 | stop is idempotent | `stop()` twice returns without error both times; the watch and event loop end exactly once; no subscriber sees a further state. | `stop_is_idempotent_and_resolves_once` | `stopIsIdempotentAndResolvesOnce` |
| 16 | Unsupported platform | A backend answering NotAvailable surfaces `ERR_CONNECTIVITY_NOT_AVAILABLE` from `fetch`, `refresh`, `state`, and `check_connectivity`, with no panic and no synchronous throw — the unsupported backend does exactly this. | `unsupported_platform_not_available_no_panic` | `unsupportedPlatformNotAvailableNoPanic` |
| 17 | WiMax, Bluetooth, Other preserved | Snapshot types wimax, bluetooth, other reach `check_connectivity` and `fetch` unchanged (TS strings `"wimax"`, `"bluetooth"`, `"other"`). | `wimax_bluetooth_other_mapping_preserved` | `wimaxBluetoothOtherMappingPreserved` |
| 18 | Metered path | Snapshot `is_expensive` true sets `details.is_connection_expensive` true. | `metered_path_sets_is_connection_expensive` | `meteredPathSetsIsConnectionExpensive` |
| 19 | details none exactly when disconnected | Connected: `details` some. Nothing live: `is_connected` false and `details` none (netinfo parity). | `details_none_exactly_when_disconnected` | `detailsNoneExactlyWhenDisconnected` |
| 20 | onConnectivityChanged distinct, never empty | Consecutive equal lists are not re-emitted; every emitted list is non-empty; `[none]` only when nothing is live. | `on_connectivity_changed_distinct_never_empty` | `onConnectivityChangedDistinctNeverEmpty` |
| 21 | watch error still serves snapshots | A failed `watch` (`set_watch_error`) leaves `fetch`, `refresh`, and `state` working; no hang, no panic. | `watch_error_still_serves_snapshots` | `watchErrorStillServesSnapshots` |
| 22 | Dropped subscription stops cleanly | Dropping the receiver / calling the unsubscribe function stops that subscription's work; later events reach remaining subscribers only, and nothing hangs. | `dropped_subscription_stops_cleanly` | `droppedSubscriptionStopsCleanly` |
| 23 | stop cancels in-flight work | A `refresh` awaiting the scripted probe when `stop()` lands resolves exactly once with `ERR_CONNECTIVITY_CANCELLED` — never a hang, never a second resolution. | `stop_cancels_in_flight_operations` | `stopCancelsInFlightOperations` |
| 24 | Empty interface name | `fetch` with an empty interface name (`Some("")` / `""`) rejects `ERR_CONNECTIVITY_INVALID_ARGUMENT` — an empty name is invalid, not an unknown interface. | `empty_argument_is_invalid_argument` | `emptyArgumentIsInvalidArgument` |
| 25 | Timeout code propagates | A backend erroring Timeout surfaces `ERR_CONNECTIVITY_TIMEOUT` from the public call. | `timeout_error_propagates_with_code` | `timeoutErrorPropagatesWithCode` |
| 26 | Backend code and status | A backend erroring Backend with a status (an HRESULT, an errno) surfaces `ERR_CONNECTIVITY_BACKEND`; on Rust, `status()` carries the platform status unchanged. | `backend_error_propagates_status` | `backendErrorPropagatesStatus` |
