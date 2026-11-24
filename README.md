# nsh-connectivity

Network status, change events, and real internet reachability.

Part of the Native Shell (NSH) capability set. Same layout as the other SuperNSH repos: a Rust crate, a TypeScript package, docs in `docs/`, CI on GitHub Actions, and a web status page on GitHub Pages at https://supernsh.github.io/connectivity/.

The API follows Flutter's `connectivity_plus` and React Native's `NetInfo`, so an app that knows either knows this one. [docs/parity.md](docs/parity.md) lists every reference API and what it does here; [docs/decisions.md](docs/decisions.md) says why each choice was made.

## Install

In the app's directory:

```sh
nsh add nsh-connectivity
```

`nsh add` needs NSH main or the nightly: the 0.6.0 and 0.6.1 releases do not have it. In a Rust app, `cargo add nsh-connectivity` works with any of them.

TSX imports `nsh-connectivity`, with `nsh-connectivity/rust` and `nsh-connectivity/memory` alongside it.

## Status

**Implemented and verified by CI on `master`; not run on devices.** The crate `crates/nsh-connectivity` carries the core API — check, fetch, refresh, state, configure, subscribe, the change stream, stop, the debounce loop, the HTTP 204 probe — with backends for Apple (NWPathMonitor), Android (ConnectivityManager over JNI), Windows (NetworkInformation), Linux (netdev and if-watch), the web (navigator.onLine plus fetch), and an unsupported-target fallback. The package `packages/nsh-connectivity` carries the same API in camelCase over a thin native seam, with `useNetInfo` and a memory backend, scripted probe and manual clock for tests.

Verified by [CI run 44381667703](https://github.com/SuperNSH/connectivity/actions/runs/44381667703) on `master` (commit `42b0d6c`, NSH 0.6.0; the code is unchanged since `fe88dc4`, whose run 44376306446 was green too), all eight jobs green:

- `format`: `cargo fmt --all -- --check`.
- `rust (ubuntu-latest)`, `rust (macos-latest)`, `rust (windows-latest)`: `cargo clippy --all-targets -- -D warnings` and `cargo test --all-targets`. Unit tests: 22 on Ubuntu (including the Linux backend's netlink tests), 14 on macOS (including a live NWPathMonitor snapshot and watch), 12 on Windows; plus the 28 tests in `tests/edge_cases.rs` on each host (the 26 named cases of [docs/goals.md](docs/goals.md), a stalled-server probe test and an in-flight configure test).
- `cross-check (aarch64-apple-ios)`, `cross-check (aarch64-linux-android)`, `cross-check (wasm32-unknown-unknown)`: clippy with `-D warnings`, compile and lint only.
- `typescript`: `nsh check`, `nsh build` (no warning, enforced by the job) and `nsh test` (32 tests, including one that opens the app over the host's real platform backend, answering a type list or a coded error; no network).

Every new test was seen to fail by breaking the code on purpose before it counted (done on a developer machine before the AGENTS.md no-local-builds rule applied to this change).

Not verified: no device or simulator ran this code, the Windows and Android backends never executed, the web backend never ran in a browser, and no probe hit the live Internet. Not implemented: change events on Android (`watch` answers `ERR_CONNECTIVITY_NOT_AVAILABLE`), the SSID on Apple, Linux and Android, satellite on Apple, and the detail fields no backend reads (`bssid`, `strength`, frequency, link speed, cellular generation, carrier). On Android, NSH 0.6.0 does not declare `ACCESS_NETWORK_STATE`, so snapshots there answer `ERR_CONNECTIVITY_PERMISSION_DENIED` until it does. [docs/decisions.md](docs/decisions.md) has the reasons.

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

CI runs every gate on push and on pull requests: format, Clippy with `-D warnings`, the Rust tests, the TypeScript typecheck and tests, and the cross-target Clippy builds. The CI run on `master` is the one that counts; Status names the run that verified the current code.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/connectivity
