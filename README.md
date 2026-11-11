# nsh-connectivity

Network status, change events, and real internet reachability.

Part of the Native Shell (NSH) capability set. Same layout as the other SuperNSH repos: a Rust crate, a TypeScript package, docs in `docs/`, CI on GitHub Actions, and a web status page on GitHub Pages at https://supernsh.github.io/connectivity/.

The API follows Flutter's `connectivity_plus` and React Native's `NetInfo`, so an app that knows either knows this one. [docs/parity.md](docs/parity.md) lists every reference API and what it does here; [docs/decisions.md](docs/decisions.md) says why each choice was made.

## Status

**Implemented and gated. Not run on devices.** The crate `crates/nsh-connectivity` carries the core API — check, fetch, refresh, subscribe, the debounce loop, the HTTP 204 probe — with five platform backends: Apple (NWPathMonitor), Android (ConnectivityManager over JNI), Windows (NetworkInformation), Linux (netdev and if-watch), web (navigator.onLine plus the probe), and an unsupported-target backend. The package `packages/nsh-connectivity` carries the same API in camelCase, with a `useNetInfo` hook and a memory backend for tests.

How that is verified: the gates ran locally for this change — `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, then `npm run typecheck` and `npm test` in `packages/nsh-connectivity` — and [.github/workflows/ci.yml](.github/workflows/ci.yml) runs the same gates on every push, with cross-checks that compile each platform bridge for `aarch64-apple-ios`, `aarch64-linux-android` and `wasm32-unknown-unknown`. A local run proves the code compiles and the offline test suite passes on the machine that ran it: the tests drive fake and memory backends and script the probe's HTTP answers. No device ran this code and no probe hit the live Internet. Per AGENTS.md, a claim with no passing run is unverified; the CI run on `master` is the run that stands behind this section.

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

CI runs every gate on push and on pull requests: format, Clippy with `-D warnings`, the Rust tests, the TypeScript typecheck and tests, and the cross-target Clippy builds. Nothing is built on a developer's machine; the one local gate run for this change is recorded in Status.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/connectivity
