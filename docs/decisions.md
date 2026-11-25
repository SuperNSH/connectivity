# Decisions

Each entry says what was chosen and why. [parity.md](parity.md) lists what each reference API does here.

## Open

None. The three questions this repo opened with are decided below; a new question gets an entry before code is written for it.

## Decided

### NSH is pinned to 0.7.0 from crates.io (0.7.1 resolves)

The brief left the version open because native features may exist only on NSH's unreleased `main`. `nsh-biometrics` built its released line on `nsh-core = "0.4.0"`, and this crate needs the same `#[native_module]` and `#[class]` machinery, so the crate started on 0.4.0, what biometrics shipped on, and moved to 0.5.0 with the SuperNSH set on 2026-11-11, then to the 0.5.1 bug-fix release (no API changes) on 2026-11-16, to 0.6.0 on 2026-11-18, and to 0.7.0 on 2026-11-25. NSH 0.7 is the first release with `nsh add`, so CI installs the tagged 0.7.1 instead of a nightly.

One consequence carries over from the biometrics work: NSH writes the Android manifest itself. 0.5.0 declares only `INTERNET`, plus `POST_NOTIFICATIONS` for the notifications capability and `RECORD_AUDIO`/`CAMERA` for web-view media (`crates/nsh-cli/src/android.rs` at v0.5.0), so this crate cannot get `ACCESS_FINE_LOCATION` into an app. Reading the SSID on Android 12+ needs that permission, so the SSID reads `None` there — not an error — until NSH can declare it. The connectivity queries themselves need `ACCESS_NETWORK_STATE`, which 0.5.x does not declare either (`crates/nsh-cli/src/android.rs`, checked against the v0.5.0 source on 2026-11-17): Android then throws `SecurityException`, which the backend reports as `ERR_CONNECTIVITY_PERMISSION_DENIED`. An earlier version of this entry said the queries need no permission; that was wrong. NSH 0.7.1 changes the last point: its manifest declares `ACCESS_NETWORK_STATE` (`crates/nsh-cli/src/android.rs` at v0.7.1, checked 2026-11-25), and `android.permissions` in `nsh.json` lets an app add `ACCESS_FINE_LOCATION` itself, so the SSID is an app choice now, though this crate does not read it on Android yet.

### The platform libraries

- **Apple: NWPathMonitor through Network.framework's C API, bound by hand.** One API answers both the snapshot and the change events on macOS and iOS, and carries the `isExpensive` flag behind `is_connection_expensive`. No objc2 crate binds Network.framework: `objc2-network` on crates.io is a reserved 0.0.0 placeholder (checked 2026-11-16), `objc2-network-extension` binds NetworkExtension, a different framework, and `cidre` is a whole-SDK crate. The monitor needs about ten `nw_path_*`/`nw_path_monitor_*` C functions from `<Network/Network.h>`, linked with `#[link(name = "Network", kind = "framework")]`; `dispatch2` (0.3) supplies the queue and `block2` (0.6) the update handler, the pair `objc2` itself uses.
- **Android: `ConnectivityManager` over JNI (`jni` 0.21, `ndk-context` 0.1).** The framework API both references use on Android; `ndk-context` supplies the VM and the application context, the same seam `nsh-biometrics` uses.
- **Windows: `NetworkInformation` through the `windows` crate (0.62, feature `Networking_Connectivity`).** The connection profiles answer the snapshot and `NetworkStatusChanged` answers the events; a failed COM call is `ERR_CONNECTIVITY_BACKEND` with the HRESULT as the status.
- **Linux: `netdev` (0.47) for inspection, `if-watch` (3.2, tokio) for events.** The brief's recommended path.
- **Web: `navigator.onLine` plus `online`/`offline` events, and the probe through the browser's `fetch`.** What netinfo's own web module does; the browser cannot name the radio, so the snapshot reports `wifi` when online and leaves the reachability answer to the probe.
- **The reachability probe: `ureq` (3, rustls) in the crate on native targets, the browser's `fetch` on wasm.** The same HTTP 204 classification on every target, mirrored in the TS package's own probe. It is GET-only: 204 endpoints reject HEAD, so `reachabilityMethod` is not carried.

The crate manifest (`crates/nsh-connectivity/Cargo.toml`) is the record of the versions; the gates compile against whatever it pins, which is the check that matters here.

### All five platforms are in scope

macOS and iOS (one Apple backend), Android, Windows, Linux and the web. CI compiles each host's bridge on its own runner and cross-checks `aarch64-apple-ios`, `aarch64-linux-android` and `wasm32-unknown-unknown`; a target with no bridge gets the unsupported backend, which answers `ERR_CONNECTIVITY_NOT_AVAILABLE` without panicking.

### The references are cloned under `.refs/`

All six research URLs resolved to their canonical repositories and are cloned under `.refs/` (gitignored, so CI never sees them):

- https://github.com/fluttercommunity/plus_plugins
- https://github.com/react-native-netinfo/react-native-netinfo
- https://github.com/libp2p/if-watch
- https://github.com/shellrow/netdev
- https://github.com/longbridge/nsh
- https://github.com/SuperNSH/biometrics

[parity.md](parity.md) reads the sources in these clones, not the READMEs.

### The core runs on tokio on native targets, on the page on the web

The debounce loop is spawned on the current tokio runtime: NSH polls its tasks inside one (`nsh-core` 0.6.0, `element/task.rs`), and tests run on tokio's paused clock. So `BackendFuture` is `Send` on native targets, a narrowing of the seam the first draft of goals.md gave. Timers are tokio's (a thread times them when no runtime is current), so an NSH test's manual clock does not move the Rust core's debounce; the TS core takes its timers as a parameter instead. On wasm32 the loop runs with `wasm_bindgen_futures::spawn_local` and `setTimeout`. `subscribe` and `on_connectivity_changed` are `async` and return a `Result`, because they read the current state first and starting the loop can fail with no runtime (`NotAvailable`).

### The TS package runs its own core over a thin native seam

`NativeConnectivity` exposes the platform seam (`snapshot`, `watch`, `nextEvent`, `stopWatching`), not the whole Rust API. The TS `Connectivity` debounces and probes in TS, with the engine's `fetch` as the probe, so it follows NSH's clocks and needs no tokio context from a JS call. `nextEvent` is a pull: a JS `Callback` is `Rc`-based and cannot be called from the platform's threads.

### What each backend can and cannot answer

- Apple: `NWPathMonitor`, twelve C functions declared by hand. A path carries no SSID or addresses, so those are `None`. A VPN is recognised by its tunnel's name (`utun*`, `ipsec*`, `ppp*`, `tun*`, `tap*`), because Network reports it as `other`. Satellite needs `nw_path_is_ultra_constrained` (macOS/iOS 26) and is not read.
- Android: snapshot only. Change events need a `ConnectivityManager.NetworkCallback` subclass, which is Java code. A crate can ship that: `nsh-secure-storage` embeds a dex and loads it through `InMemoryDexClassLoader`, and NSH 0.7.1 has no mechanism that stops it. The manifest no longer does: `registerDefaultNetworkCallback` needs `ACCESS_NETWORK_STATE`, which NSH 0.7.1 declares. The dex is still unwritten, so `watch` answers `NotAvailable`. No polling fallback was added, since the poll would hit the same permission.
- Windows: connection profiles and `NetworkStatusChanged`. IP address and subnet are not read.
- Linux: netdev and if-watch as planned; no SSID (that needs NetworkManager over D-Bus).
- Web: `navigator.onLine` reports `wifi` when online; the probe needs a CORS-enabled URL, or the browser hides the status and it reads unreachable.
- No backend fills `bssid`, `strength`, `frequency_mhz`, `link_speed_mbps`, `cellular_generation` or `carrier` yet: the `PlatformSnapshot` seam does not carry them.
