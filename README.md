# nsh-connectivity

Network status, change events, and real internet reachability.

Part of the Native Shell (NSH) capability set. Same layout as the other SuperNSH repos: a Rust crate, a TypeScript package, docs in `docs/`, CI on GitHub Actions, and a web status page on GitHub Pages at https://supernsh.github.io/connectivity/.

The API follows Flutter's `connectivity_plus` and React Native's `NetInfo`, so an app that knows either knows this one. [docs/parity.md](docs/parity.md) lists every reference API and what it does here; [docs/decisions.md](docs/decisions.md) says why each choice was made.

## Status

**Implemented, gated locally, not run on devices.** The crate `crates/nsh-connectivity` carries the core API — check, fetch, refresh, state, configure, subscribe, the change stream, stop, the debounce loop, the HTTP 204 probe — with backends for Apple (NWPathMonitor), Android (ConnectivityManager over JNI), Windows (NetworkInformation), Linux (netdev and if-watch), the web (navigator.onLine plus fetch), and an unsupported-target fallback. The package `packages/nsh-connectivity` carries the same API in camelCase over a thin native seam, with `useNetInfo` and a memory backend, scripted probe and manual clock for tests.

What ran on 2026-11-17, before any CI run of this change:

- macOS host: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` (14 unit tests, including a live NWPathMonitor snapshot and watch, and the 28 tests in `tests/edge_cases.rs`).
- Cross clippy with `-D warnings` from macOS: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android` (NDK 28.2), `wasm32-unknown-unknown`.
- Ubuntu 24.04 (a cloud box, with the same sources): `cargo clippy --all-targets -- -D warnings` and `cargo test --all-targets` (22 unit tests including the Linux backend's live netlink tests, plus the 28 edge cases); `cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings` (Windows compiled and linted only, never run).
- `packages/nsh-connectivity` with NSH 0.5.1: `nsh check`, `nsh build` (no warning), `nsh test` (32 tests, including one that opens the app on the real Apple backend).
- Every new test was seen to fail by breaking the code on purpose, then passed again once it was restored.

Not verified: no device or simulator ran this code, the Windows and Android backends never executed, the web backend never ran in a browser, and no probe hit the live Internet. Not implemented: change events on Android (`watch` answers `ERR_CONNECTIVITY_NOT_AVAILABLE`), the SSID on Apple, Linux and Android, satellite on Apple, and the detail fields no backend reads (`bssid`, `strength`, frequency, link speed, cellular generation, carrier). On Android, NSH 0.5.1 does not declare `ACCESS_NETWORK_STATE`, so snapshots there answer `ERR_CONNECTIVITY_PERMISSION_DENIED` until it does. [docs/decisions.md](docs/decisions.md) has the reasons.

## Approach

Interface events per platform (the libraries named in Status), debounced, then one HTTP 204 reachability probe — `ureq` with rustls on native targets, `fetch` on the web — shared by concurrent refreshes. [docs/goals.md](docs/goals.md) is the testable contract, with a named test for each edge case in both languages.

## Layout

- `crates/nsh-connectivity/`: the Rust crate — core, backends, probe, fake backend for tests.
- `packages/nsh-connectivity/`: the TypeScript package — the same API, the hook, the memory backend.
- `docs/brief.md`: options, recommended path and edge cases from the research.
- `docs/goals.md`: the contract every test reads.
- `docs/decisions.md`: what was chosen and why.
- `docs/parity.md`: reference API for reference API.
- `site/`: static status page, published by `.github/workflows/pages.yml`.
- `.refs/`: gitignored clones of the reference sources; local only, never in CI.

## Checks

CI runs every gate on push and on pull requests: format, Clippy with `-D warnings`, the Rust tests, the TypeScript typecheck and tests, and the cross-target Clippy builds. The local gate runs for this change are recorded in Status; the CI run on `master` is the one that counts.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/connectivity
