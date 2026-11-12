# Decisions

Each entry says what was chosen and why. [parity.md](parity.md) lists what each reference API does here.

## Open

None. The three questions this repo opened with are decided below; a new question gets an entry before code is written for it.

## Decided

### NSH is pinned to 0.5.0 from crates.io

The brief left the version open because native features may exist only on NSH's unreleased `main`. `nsh-biometrics` built its released line on `nsh-core = "0.4.0"`, and this crate needs the same `#[native_module]` and `#[class]` machinery, so the crate started on 0.4.0, what biometrics shipped on, and moved to 0.5.0 with the SuperNSH set on 2026-11-11.

One consequence carries over from the biometrics work: NSH writes the Android manifest itself. 0.5.0 declares only `INTERNET`, plus `POST_NOTIFICATIONS` for the notifications capability and `RECORD_AUDIO`/`CAMERA` for web-view media (`crates/nsh-cli/src/android.rs` at v0.5.0), so this crate cannot get `ACCESS_FINE_LOCATION` into an app. Reading the SSID on Android 12+ needs that permission, so the SSID reads `None` there — not an error — until NSH can declare it. The connectivity queries themselves need no permission.

### The platform libraries

- **Apple: NWPathMonitor through `objc2-network` (0.3).** One API answers both the snapshot and the change events on macOS and iOS, and carries the `isExpensive` flag behind `is_connection_expensive`.
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
