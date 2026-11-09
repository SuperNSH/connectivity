# Connectivity: brief

Network status, change events, and real internet reachability.

## Options

- if-watch: Rust interface up and down events (check macOS behaviour first)
- netdev: Rust interface inspection
- NWPathMonitor: Apple; ConnectivityManager: Android
- An HTTP 204 probe on top of all of them

## Recommended path

Interface events from if-watch and netdev. Reachability from an HTTP 204 probe on the existing HTTP service.

## Edge cases to design for

- Interface up is not internet up: captive portals and VPNs
- Debounce flapping events
- Android SSID needs location permission on 12+ and FLAG_INCLUDE_LOCATION_INFO
- Background throttling changes what events arrive

## Research status

Research done 2026-11-09 by web search. Versions and licences are unverified until checked on the registry before pinning. Full catalogue: the research notes for the SuperNSH library set.
