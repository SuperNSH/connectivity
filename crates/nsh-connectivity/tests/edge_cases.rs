//! One test per edge case in docs/goals.md, through the public API over `FakeBackend`. Timing
//! runs on tokio's paused clock; reachability is a `FakeProbe` or a loopback HTTP server, so
//! nothing leaves the machine.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use nsh_connectivity::{
    Configuration, ConnectionType, Connectivity, ConnectivityError, ErrorKind, FakeBackend,
    FakeProbe, InterfaceInfo, PlatformSnapshot, ProbeOutcome, Reachability,
};

const DEBOUNCE: Duration = Duration::from_millis(500);

/// A loopback server: `/generate_204` answers 204, `/portal` 200, `/stall` never answers.
struct Server {
    base: String,
    paths: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let seen = paths.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let seen = seen.clone();
                thread::spawn(move || answer(stream, &seen));
            }
        });
        Server { base, paths }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("paths").clone()
    }
}

fn answer(mut stream: TcpStream, seen: &Mutex<Vec<String>>) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => request.push(byte[0]),
            _ => return,
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
    seen.lock().expect("paths").push(path.clone());
    let status = match path.as_str() {
        "/generate_204" => "204 No Content",
        "/portal" => "200 OK",
        _ => {
            // A stalled answer: hold the connection open past any test's deadline.
            thread::sleep(Duration::from_secs(30));
            return;
        }
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

fn wifi(name: &str) -> InterfaceInfo {
    InterfaceInfo {
        name: name.to_owned(),
        r#type: ConnectionType::Wifi,
        ip_address: None,
        subnet: None,
        ssid: None,
    }
}

fn snapshot(types: &[ConnectionType]) -> PlatformSnapshot {
    PlatformSnapshot {
        types: types.to_vec(),
        interface: types.first().map(|t| InterfaceInfo {
            name: "if0".to_owned(),
            r#type: *t,
            ip_address: None,
            subnet: None,
            ssid: None,
        }),
        is_expensive: false,
    }
}

fn config(url: String, timeout: Duration) -> Configuration {
    Configuration {
        reachability_url: url,
        reachability_timeout: timeout,
        ..Configuration::default()
    }
}

fn faked(probe: &FakeProbe) -> (Connectivity, FakeBackend) {
    let backend = FakeBackend::new();
    let connectivity = Connectivity::with_backend(backend.clone()).with_probe(probe.clone());
    (connectivity, backend)
}

fn code(result: Result<impl std::fmt::Debug, ConnectivityError>) -> &'static str {
    result.expect_err("an error").code()
}

#[tokio::test]
async fn interface_up_is_not_internet_up_captive_portal() {
    let server = Server::start();
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    connectivity.configure(config(server.url("/portal"), Duration::from_secs(5)));
    let state = connectivity.refresh().await.expect("refresh");
    assert!(state.is_connected);
    assert_eq!(state.is_internet_reachable, Reachability::Unreachable);
    assert!(state.details.expect("connected").captive_portal);
}

#[tokio::test]
async fn probe_204_means_reachable() {
    let server = Server::start();
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    connectivity.configure(config(server.url("/generate_204"), Duration::from_secs(5)));
    let state = connectivity.refresh().await.expect("refresh");
    assert_eq!(state.is_internet_reachable, Reachability::Reachable);
    assert!(!state.details.expect("connected").captive_portal);
}

#[tokio::test(start_paused = true)]
async fn probe_timeout_resolves_once_unreachable() {
    let probe = FakeProbe::stalled();
    let (connectivity, _backend) = faked(&probe);
    let started = tokio::time::Instant::now();
    let state = connectivity
        .refresh()
        .await
        .expect("a timeout is an answer");
    assert_eq!(state.is_internet_reachable, Reachability::Unreachable);
    assert!(state.is_connected);
    assert!(started.elapsed() >= Configuration::default().reachability_timeout);
    assert_eq!(probe.probes(), 1);
}

#[tokio::test]
async fn http_probe_against_a_stalled_server_is_unreachable() {
    let server = Server::start();
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    connectivity.configure(config(server.url("/stall"), Duration::from_millis(300)));
    let state = tokio::time::timeout(Duration::from_secs(10), connectivity.refresh())
        .await
        .expect("bounded by the probe timeout")
        .expect("refresh");
    assert_eq!(state.is_internet_reachable, Reachability::Unreachable);
    assert!(!state.details.expect("connected").captive_portal);
}

#[tokio::test]
async fn probe_transport_error_is_unreachable() {
    // A port bound and released at once: nothing listens there, so connecting is refused.
    let url = {
        let closed = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        format!("http://{}/generate_204", closed.local_addr().expect("addr"))
    };
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    connectivity.configure(config(url, Duration::from_secs(5)));
    let state = connectivity.refresh().await.expect("not an error state");
    assert!(state.is_connected);
    assert_eq!(state.is_internet_reachable, Reachability::Unreachable);
    assert!(!state.details.expect("connected").captive_portal);
}

#[tokio::test(start_paused = true)]
async fn debounce_coalesces_flapping_events() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    connectivity.configure(Configuration {
        probe_on_events: false,
        ..Configuration::default()
    });
    let mut states = connectivity.subscribe().await.expect("subscribe");
    states.mark_unchanged();
    let reads = backend.snapshots();

    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Ethernet])));
    assert!(backend.emit_event());
    tokio::time::sleep(DEBOUNCE / 2).await;
    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Cellular])));
    assert!(backend.emit_event());
    tokio::time::sleep(DEBOUNCE / 2).await;
    assert!(
        !states.has_changed().expect("open"),
        "published inside the window"
    );

    tokio::time::sleep(DEBOUNCE).await;
    assert!(states.has_changed().expect("open"));
    assert_eq!(
        states.borrow_and_update().connection_type,
        ConnectionType::Cellular
    );
    assert_eq!(backend.snapshots(), reads + 1, "one read per window");
    tokio::time::sleep(DEBOUNCE * 4).await;
    assert!(!states.has_changed().expect("open"));
}

#[tokio::test]
async fn ssid_is_none_without_location_permission() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Ok(PlatformSnapshot {
        types: vec![ConnectionType::Wifi],
        interface: Some(wifi("wlan0")),
        is_expensive: false,
    }));
    let state = Connectivity::with_backend(backend)
        .fetch(None)
        .await
        .expect("silence, not an error");
    assert_eq!(state.connection_type, ConnectionType::Wifi);
    assert_eq!(state.details.expect("connected").ssid, None);
}

#[tokio::test]
async fn ssid_query_refusal_is_permission_denied() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Err(ErrorKind::PermissionDenied.into()));
    let connectivity = Connectivity::with_backend(backend);
    assert_eq!(
        code(connectivity.fetch(None).await),
        "ERR_CONNECTIVITY_PERMISSION_DENIED"
    );
}

#[tokio::test(start_paused = true)]
async fn late_event_burst_debounces_to_one_state() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    let mut states = connectivity.subscribe().await.expect("subscribe");
    states.mark_unchanged();
    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Ethernet])));
    for _ in 0..3 {
        assert!(backend.emit_event());
    }
    tokio::time::sleep(DEBOUNCE * 2).await;
    assert!(states.has_changed().expect("open"));
    let state = states.borrow_and_update().clone();
    assert_eq!(state.connection_type, ConnectionType::Ethernet);
    assert_eq!(state.is_internet_reachable, Reachability::Reachable);
    assert_eq!(probe.probes(), 1, "one probe for the burst");
    tokio::time::sleep(DEBOUNCE * 4).await;
    assert!(!states.has_changed().expect("open"), "one published state");
}

#[tokio::test]
async fn vpn_reported_alongside_underlying_path() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Wifi, ConnectionType::Vpn])));
    let connectivity = Connectivity::with_backend(backend);
    assert_eq!(
        connectivity.check_connectivity().await.expect("check"),
        vec![ConnectionType::Wifi, ConnectionType::Vpn]
    );
    assert_eq!(
        connectivity
            .fetch(None)
            .await
            .expect("fetch")
            .connection_type,
        ConnectionType::Wifi
    );
}

#[tokio::test]
async fn check_connectivity_never_empty_none_alone() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Ok(PlatformSnapshot::default()));
    let connectivity = Connectivity::with_backend(backend);
    assert_eq!(
        connectivity.check_connectivity().await.expect("check"),
        vec![ConnectionType::None]
    );
}

#[tokio::test]
async fn fetch_unknown_interface_errors() {
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    assert_eq!(
        code(connectivity.fetch(Some("eth7")).await),
        "ERR_CONNECTIVITY_UNKNOWN_INTERFACE"
    );
    let known = connectivity.fetch(Some("wlan0")).await.expect("known");
    assert_eq!(known.connection_type, ConnectionType::Wifi);
}

#[tokio::test]
async fn concurrent_refresh_shares_one_probe() {
    let server = Server::start();
    let backend = FakeBackend::new();
    let connectivity = Connectivity::with_backend(backend.clone());
    connectivity.configure(config(server.url("/generate_204"), Duration::from_secs(5)));
    let (a, b, c) = tokio::join!(
        connectivity.refresh(),
        connectivity.refresh(),
        connectivity.refresh()
    );
    let a = a.expect("a");
    assert_eq!(a, b.expect("b"));
    assert_eq!(a, c.expect("c"));
    assert_eq!(a.is_internet_reachable, Reachability::Reachable);
    assert_eq!(backend.snapshots(), 1);
    assert_eq!(server.paths(), vec!["/generate_204"]);
}

#[tokio::test]
async fn configure_changes_next_read() {
    let server = Server::start();
    let connectivity = Connectivity::with_backend(FakeBackend::new());
    connectivity.configure(config(server.url("/generate_204"), Duration::from_secs(5)));
    let first = connectivity.refresh().await.expect("first");
    assert_eq!(first.is_internet_reachable, Reachability::Reachable);

    connectivity.configure(config(server.url("/portal"), Duration::from_secs(5)));
    let second = connectivity.refresh().await.expect("second");
    assert_eq!(server.paths(), vec!["/generate_204", "/portal"]);
    assert!(second.details.expect("connected").captive_portal);
}

#[tokio::test(start_paused = true)]
async fn configure_leaves_work_in_flight_untouched() {
    let probe = FakeProbe::stalled();
    let (connectivity, _backend) = faked(&probe);
    connectivity.configure(config("http://a.test/".to_owned(), Duration::from_secs(1)));
    let pending = connectivity.refresh();
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    connectivity.configure(config("http://b.test/".to_owned(), Duration::from_secs(60)));
    let started = tokio::time::Instant::now();
    let state = pending.await.expect("the old deadline");
    assert_eq!(state.is_internet_reachable, Reachability::Unreachable);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "kept its own timeout"
    );
    probe.set_outcome(Some(ProbeOutcome::Reachable));
    connectivity.refresh().await.expect("next");
    assert_eq!(probe.urls(), vec!["http://a.test/", "http://b.test/"]);
}

#[tokio::test(start_paused = true)]
async fn subscribe_yields_current_state_first() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, _backend) = faked(&probe);
    let now = connectivity.refresh().await.expect("refresh");
    let states = connectivity.subscribe().await.expect("subscribe");
    assert_eq!(*states.borrow(), now);
    assert_eq!(
        states.borrow().is_internet_reachable,
        Reachability::Reachable
    );
}

#[tokio::test(start_paused = true)]
async fn stop_is_idempotent_and_resolves_once() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    let mut states = connectivity.subscribe().await.expect("subscribe");
    assert_eq!(backend.watchers(), 1);
    connectivity.stop();
    connectivity.stop();
    assert_eq!(backend.stopped(), 1, "the watch ends exactly once");
    assert!(!backend.emit_event(), "no watcher after stop");
    assert!(states.changed().await.is_err(), "the subscription ended");
}

#[tokio::test]
async fn unsupported_platform_not_available_no_panic() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Err(ErrorKind::NotAvailable.into()));
    backend.set_watch_error(Some(ErrorKind::NotAvailable.into()));
    let connectivity = Connectivity::with_backend(backend);
    let expected = "ERR_CONNECTIVITY_NOT_AVAILABLE";
    assert_eq!(code(connectivity.fetch(None).await), expected);
    assert_eq!(code(connectivity.refresh().await), expected);
    assert_eq!(code(connectivity.state().await), expected);
    assert_eq!(code(connectivity.check_connectivity().await), expected);
    assert_eq!(code(connectivity.subscribe().await), expected);
}

#[tokio::test]
async fn wimax_bluetooth_other_mapping_preserved() {
    let backend = FakeBackend::new();
    let connectivity = Connectivity::with_backend(backend.clone());
    for kind in [
        ConnectionType::WiMax,
        ConnectionType::Bluetooth,
        ConnectionType::Other,
    ] {
        backend.set_snapshot(Ok(snapshot(&[kind])));
        assert_eq!(
            connectivity.check_connectivity().await.expect("check"),
            vec![kind]
        );
        assert_eq!(
            connectivity
                .fetch(None)
                .await
                .expect("fetch")
                .connection_type,
            kind
        );
    }
    assert_eq!(ConnectionType::WiMax.as_str(), "wimax");
    assert_eq!(ConnectionType::Bluetooth.as_str(), "bluetooth");
    assert_eq!(ConnectionType::Other.as_str(), "other");
}

#[tokio::test]
async fn metered_path_sets_is_connection_expensive() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Ok(PlatformSnapshot {
        is_expensive: true,
        ..snapshot(&[ConnectionType::Cellular])
    }));
    let state = Connectivity::with_backend(backend)
        .fetch(None)
        .await
        .expect("fetch");
    assert!(state.details.expect("connected").is_connection_expensive);
}

#[tokio::test]
async fn details_none_exactly_when_disconnected() {
    let backend = FakeBackend::new();
    let connectivity = Connectivity::with_backend(backend.clone());
    let connected = connectivity.fetch(None).await.expect("connected");
    assert!(connected.is_connected);
    assert!(connected.details.is_some());

    backend.set_snapshot(Ok(PlatformSnapshot::default()));
    let gone = connectivity.fetch(None).await.expect("disconnected");
    assert!(!gone.is_connected);
    assert_eq!(gone.connection_type, ConnectionType::None);
    assert!(gone.details.is_none());
}

#[tokio::test(start_paused = true)]
async fn on_connectivity_changed_distinct_never_empty() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    let mut stream = connectivity
        .on_connectivity_changed()
        .await
        .expect("stream");
    assert_eq!(stream.next().await, Some(vec![ConnectionType::Wifi]));

    // The same list again is not re-emitted.
    assert!(backend.emit_event());
    let repeat = tokio::time::timeout(DEBOUNCE * 4, stream.next()).await;
    assert!(repeat.is_err(), "re-emitted an equal list: {repeat:?}");

    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Ethernet])));
    assert!(backend.emit_event());
    assert_eq!(stream.next().await, Some(vec![ConnectionType::Ethernet]));

    backend.set_snapshot(Ok(PlatformSnapshot::default()));
    assert!(backend.emit_event());
    assert_eq!(stream.next().await, Some(vec![ConnectionType::None]));
}

#[tokio::test(start_paused = true)]
async fn watch_error_still_serves_snapshots() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    backend.set_watch_error(Some(ErrorKind::Backend.into()));
    let states = connectivity
        .subscribe()
        .await
        .expect("subscribes without events");
    assert_eq!(backend.watchers(), 0);
    assert!(!backend.emit_event());
    assert!(connectivity.fetch(None).await.is_ok());
    let refreshed = connectivity.refresh().await.expect("refresh");
    assert_eq!(refreshed.is_internet_reachable, Reachability::Reachable);
    assert_eq!(connectivity.state().await.expect("state"), refreshed);
    assert_eq!(*states.borrow(), refreshed);
}

#[tokio::test(start_paused = true)]
async fn dropped_subscription_stops_cleanly() {
    let probe = FakeProbe::answering(ProbeOutcome::Reachable);
    let (connectivity, backend) = faked(&probe);
    let dropped = connectivity.subscribe().await.expect("first");
    let mut kept = connectivity.subscribe().await.expect("second");
    kept.mark_unchanged();
    drop(dropped);

    backend.set_snapshot(Ok(snapshot(&[ConnectionType::Ethernet])));
    assert!(backend.emit_event());
    tokio::time::sleep(DEBOUNCE * 2).await;
    assert!(kept.has_changed().expect("open"));
    assert_eq!(
        kept.borrow_and_update().connection_type,
        ConnectionType::Ethernet
    );

    // The last subscriber gone, the loop retires and the watch ends once.
    drop(kept);
    tokio::time::sleep(DEBOUNCE).await;
    assert_eq!(backend.stopped(), 1);
    assert!(!backend.emit_event());
}

#[tokio::test(start_paused = true)]
async fn stop_cancels_in_flight_operations() {
    let probe = FakeProbe::stalled();
    let (connectivity, _backend) = faked(&probe);
    let pending = connectivity.refresh();
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err(),
        "the probe holds it"
    );
    connectivity.stop();
    let result = pending.await;
    assert_eq!(code(result), "ERR_CONNECTIVITY_CANCELLED");
    // Nothing else is left waiting on the cancelled cycle.
    probe.set_outcome(Some(ProbeOutcome::Reachable));
    assert!(connectivity.refresh().await.is_ok());
}

#[tokio::test]
async fn empty_argument_is_invalid_argument() {
    let backend = FakeBackend::new();
    let connectivity = Connectivity::with_backend(backend.clone());
    assert_eq!(
        code(connectivity.fetch(Some("")).await),
        "ERR_CONNECTIVITY_INVALID_ARGUMENT"
    );
    assert_eq!(backend.snapshots(), 0, "refused before the platform");
}

#[tokio::test]
async fn timeout_error_propagates_with_code() {
    let backend = FakeBackend::new();
    backend.set_snapshot(Err(ErrorKind::Timeout.into()));
    let connectivity = Connectivity::with_backend(backend);
    assert_eq!(
        code(connectivity.fetch(None).await),
        "ERR_CONNECTIVITY_TIMEOUT"
    );
    assert_eq!(
        code(connectivity.refresh().await),
        "ERR_CONNECTIVITY_TIMEOUT"
    );
}

#[tokio::test]
async fn backend_error_propagates_status() {
    let backend = FakeBackend::new();
    let hresult = i64::from(0x8007_0005_u32 as i32);
    backend.set_snapshot(Err(
        ConnectivityError::new(ErrorKind::Backend).with_status(hresult)
    ));
    let error = Connectivity::with_backend(backend)
        .refresh()
        .await
        .expect_err("backend");
    assert_eq!(error.code(), "ERR_CONNECTIVITY_BACKEND");
    assert_eq!(error.status(), Some(hresult));
}
