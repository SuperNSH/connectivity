import { expect, test } from "nsh/test";
import {
  type Configuration,
  Connectivity,
  type ConnectionType,
  type FetchLike,
  httpProbe,
  manualClock,
  memoryBackend,
  type NetworkState,
  type PlatformSnapshot,
  type ProbeOutcome,
  type Probe,
  scriptedProbe,
} from "../src";

// One test per edge case in docs/goals.md, through the public API over `memoryBackend`, on a
// manual clock. Reachability is a scripted probe or `httpProbe` over a scripted fetch: no test
// touches the network.

const DEBOUNCE = 500;

function setup(probe: Probe, configuration: Partial<Configuration> = {}) {
  const backend = memoryBackend();
  const clock = manualClock();
  const connectivity = new Connectivity({ backend, probe, timers: clock, configuration });
  return { backend, clock, connectivity };
}

function snapshot(types: ConnectionType[], isExpensive = false): PlatformSnapshot {
  const first = types[0];
  return {
    types,
    interface: first === undefined ? null : { name: "if0", type: first, ipAddress: null, subnet: null, ssid: null },
    isExpensive,
  };
}

/** A fetch answering `status`, recording the URLs asked. */
function answering(status: number, seen: string[] = []): FetchLike {
  return (url) => {
    seen.push(url);
    return Promise.resolve({ status });
  };
}

/** Records how a promise settles, and how many times. */
function track<T>(promise: Promise<T>) {
  const tracked = { settled: 0, value: undefined as T | undefined, error: undefined as unknown };
  promise.then(
    (value) => {
      tracked.settled += 1;
      tracked.value = value;
    },
    (error: unknown) => {
      tracked.settled += 1;
      tracked.error = error;
    },
  );
  return tracked;
}

test("interfaceUpIsNotInternetUpCaptivePortal", async () => {
  const { connectivity } = setup(httpProbe(answering(200)));
  const state = await connectivity.refresh();
  expect(state.isConnected).toBe(true);
  expect(state.isInternetReachable).toBe("unreachable");
  expect(state.details?.captivePortal).toBe(true);
});

test("probe204MeansReachable", async () => {
  const { connectivity } = setup(httpProbe(answering(204)));
  const state = await connectivity.refresh();
  expect(state.isInternetReachable).toBe("reachable");
  expect(state.details?.captivePortal).toBe(false);
});

test("probeTimeoutResolvesOnceUnreachable", async () => {
  // A fetch that never answers: only the deadline ends the probe.
  const { connectivity, clock } = setup(httpProbe(() => new Promise(() => {})));
  const pending = track(connectivity.refresh());
  await clock.advance(9999);
  expect(pending.settled).toBe(0);
  await clock.advance(1);
  expect(pending.settled).toBe(1);
  expect((pending.value as NetworkState).isInternetReachable).toBe("unreachable");
  expect((pending.value as NetworkState).isConnected).toBe(true);
  await clock.advance(20000);
  expect(pending.settled).toBe(1);
});

test("probeTransportErrorIsUnreachable", async () => {
  const refused: FetchLike = () => Promise.reject(new TypeError("connection refused"));
  const { connectivity } = setup(httpProbe(refused));
  const state = await connectivity.refresh();
  expect(state.isConnected).toBe(true);
  expect(state.isInternetReachable).toBe("unreachable");
  expect(state.details?.captivePortal).toBe(false);
});

test("debounceCoalescesFlappingEvents", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe, { probeOnEvents: false });
  const states: NetworkState[] = [];
  connectivity.addEventListener((state) => states.push(state));
  await clock.settle();
  expect(states.length).toBe(1);
  const reads = backend.snapshots();

  backend.setSnapshot(snapshot(["ethernet"]));
  expect(backend.emitEvent()).toBe(true);
  await clock.advance(DEBOUNCE / 2);
  backend.setSnapshot(snapshot(["cellular"]));
  expect(backend.emitEvent()).toBe(true);
  await clock.advance(DEBOUNCE / 2);
  expect(states.length).toBe(1);

  await clock.advance(DEBOUNCE);
  expect(states.length).toBe(2);
  expect(states[1]?.connectionType).toBe("cellular");
  expect(backend.snapshots()).toBe(reads + 1);
});

test("ssidIsNoneWithoutLocationPermission", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot({
    types: ["wifi"],
    interface: { name: "wlan0", type: "wifi", ipAddress: null, subnet: null, ssid: null },
    isExpensive: false,
  });
  const state = await connectivity.fetch();
  expect(state.connectionType).toBe("wifi");
  expect(state.details?.ssid).toBe(null);
});

test("ssidQueryRefusalIsPermissionDenied", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot("ERR_CONNECTIVITY_PERMISSION_DENIED");
  await expect(connectivity.fetch()).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_PERMISSION_DENIED" });
});

test("lateEventBurstDebouncesToOneState", async () => {
  const probe = scriptedProbe("reachable");
  const { connectivity, backend, clock } = setup(probe.probe);
  const states: NetworkState[] = [];
  connectivity.addEventListener((state) => states.push(state));
  await clock.settle();
  backend.setSnapshot(snapshot(["ethernet"]));
  for (let i = 0; i < 3; i++) expect(backend.emitEvent()).toBe(true);
  await clock.advance(DEBOUNCE * 2);
  expect(states.length).toBe(2);
  expect(states[1]?.connectionType).toBe("ethernet");
  expect(states[1]?.isInternetReachable).toBe("reachable");
  expect(probe.probes()).toBe(1);
  await clock.advance(DEBOUNCE * 4);
  expect(states.length).toBe(2);
});

test("vpnReportedAlongsideUnderlyingPath", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot(snapshot(["wifi", "vpn"]));
  expect(await connectivity.checkConnectivity()).toEqual(["wifi", "vpn"]);
  expect((await connectivity.fetch()).connectionType).toBe("wifi");
});

test("checkConnectivityNeverEmptyNoneAlone", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot(snapshot([]));
  expect(await connectivity.checkConnectivity()).toEqual(["none"]);
});

test("fetchUnknownInterfaceErrors", async () => {
  const { connectivity } = setup(scriptedProbe("reachable").probe);
  await expect(connectivity.fetch("eth7")).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_UNKNOWN_INTERFACE" });
  expect((await connectivity.fetch("wlan0")).connectionType).toBe("wifi");
});

test("concurrentRefreshSharesOneProbe", async () => {
  const seen: string[] = [];
  const { connectivity, backend } = setup(httpProbe(answering(204, seen)));
  const [a, b, c] = await Promise.all([connectivity.refresh(), connectivity.refresh(), connectivity.refresh()]);
  expect(a).toEqual(b);
  expect(a).toEqual(c);
  expect(a.isInternetReachable).toBe("reachable");
  expect(backend.snapshots()).toBe(1);
  expect(seen.length).toBe(1);
});

test("configureChangesNextRead", async () => {
  const seen: string[] = [];
  const { connectivity } = setup(httpProbe(answering(204, seen)), { reachabilityUrl: "https://a.test/204" });
  await connectivity.refresh();
  connectivity.configure({ reachabilityUrl: "https://b.test/204" });
  await connectivity.refresh();
  expect(seen).toEqual(["https://a.test/204", "https://b.test/204"]);
});

test("configureLeavesWorkInFlightUntouched", async () => {
  const probe = scriptedProbe(null);
  const { connectivity, clock } = setup(probe.probe, { reachabilityTimeoutMs: 1000 });
  const pending = track(connectivity.refresh());
  await clock.settle();
  connectivity.configure({ reachabilityTimeoutMs: 60000 });
  await clock.advance(1000);
  expect(pending.settled).toBe(1);
  expect((pending.value as NetworkState).isInternetReachable).toBe("unreachable");
});

test("subscribeYieldsCurrentStateFirst", async () => {
  const { connectivity, clock } = setup(scriptedProbe("reachable").probe);
  const now = await connectivity.refresh();
  const states: NetworkState[] = [];
  connectivity.addEventListener((state) => states.push(state));
  await clock.settle();
  expect(states).toEqual([now]);
});

test("stopIsIdempotentAndResolvesOnce", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe);
  const states: NetworkState[] = [];
  connectivity.addEventListener((state) => states.push(state));
  await clock.settle();
  expect(backend.watchers()).toBe(1);
  connectivity.stop();
  connectivity.stop();
  expect(backend.stopped()).toBe(1);
  expect(backend.emitEvent()).toBe(false);
  backend.setSnapshot(snapshot(["ethernet"]));
  await connectivity.refresh();
  await clock.advance(DEBOUNCE * 2);
  expect(states.length).toBe(1);
});

test("unsupportedPlatformNotAvailableNoPanic", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot("ERR_CONNECTIVITY_NOT_AVAILABLE");
  backend.setWatchError("ERR_CONNECTIVITY_NOT_AVAILABLE");
  const code = { code: "ERR_CONNECTIVITY_NOT_AVAILABLE" };
  await expect(connectivity.fetch()).rejects.toMatchObject(code);
  await expect(connectivity.refresh()).rejects.toMatchObject(code);
  await expect(connectivity.state()).rejects.toMatchObject(code);
  await expect(connectivity.checkConnectivity()).rejects.toMatchObject(code);
  // No synchronous throw: a listener simply hears nothing.
  const heard: NetworkState[] = [];
  const stop = connectivity.addEventListener((state) => heard.push(state));
  await clock.settle();
  expect(heard.length).toBe(0);
  stop();
});

test("wimaxBluetoothOtherMappingPreserved", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  for (const kind of ["wimax", "bluetooth", "other"] as const) {
    backend.setSnapshot(snapshot([kind]));
    expect(await connectivity.checkConnectivity()).toEqual([kind]);
    expect((await connectivity.fetch()).connectionType).toBe(kind);
  }
});

test("meteredPathSetsIsConnectionExpensive", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot(snapshot(["cellular"], true));
  expect((await connectivity.fetch()).details?.isConnectionExpensive).toBe(true);
});

test("detailsNoneExactlyWhenDisconnected", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  const connected = await connectivity.fetch();
  expect(connected.isConnected).toBe(true);
  expect(connected.details).not.toBe(null);
  backend.setSnapshot(snapshot([]));
  const gone = await connectivity.fetch();
  expect(gone.isConnected).toBe(false);
  expect(gone.connectionType).toBe("none");
  expect(gone.details).toBe(null);
});

test("onConnectivityChangedDistinctNeverEmpty", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe);
  const lists: ConnectionType[][] = [];
  connectivity.onConnectivityChanged((types) => lists.push(types));
  await clock.settle();
  expect(lists).toEqual([["wifi"]]);

  // The same list again is not re-emitted, even though the probe changed the state.
  expect(backend.emitEvent()).toBe(true);
  await clock.advance(DEBOUNCE * 2);
  expect(lists).toEqual([["wifi"]]);

  backend.setSnapshot(snapshot(["ethernet"]));
  backend.emitEvent();
  await clock.advance(DEBOUNCE * 2);
  backend.setSnapshot(snapshot([]));
  backend.emitEvent();
  await clock.advance(DEBOUNCE * 2);
  expect(lists).toEqual([["wifi"], ["ethernet"], ["none"]]);
});

test("watchErrorStillServesSnapshots", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe);
  backend.setWatchError("ERR_CONNECTIVITY_BACKEND");
  const states: NetworkState[] = [];
  connectivity.addEventListener((state) => states.push(state));
  await clock.settle();
  expect(backend.watchers()).toBe(0);
  expect(backend.emitEvent()).toBe(false);
  expect((await connectivity.fetch()).isConnected).toBe(true);
  const refreshed = await connectivity.refresh();
  expect(refreshed.isInternetReachable).toBe("reachable");
  expect(await connectivity.state()).toEqual(refreshed);
  expect(states[states.length - 1]).toEqual(refreshed);
});

test("droppedSubscriptionStopsCleanly", async () => {
  const { connectivity, backend, clock } = setup(scriptedProbe("reachable").probe);
  const dropped: NetworkState[] = [];
  const kept: NetworkState[] = [];
  const unsubscribe = connectivity.addEventListener((state) => dropped.push(state));
  const keep = connectivity.addEventListener((state) => kept.push(state));
  await clock.settle();
  unsubscribe();
  backend.setSnapshot(snapshot(["ethernet"]));
  backend.emitEvent();
  await clock.advance(DEBOUNCE * 2);
  expect(dropped.length).toBe(1);
  expect(kept[kept.length - 1]?.connectionType).toBe("ethernet");

  // The last listener gone, the watch ends once.
  keep();
  expect(backend.stopped()).toBe(1);
  expect(backend.emitEvent()).toBe(false);
});

test("stopCancelsInFlightOperations", async () => {
  const probe = scriptedProbe(null);
  const { connectivity, clock } = setup(probe.probe);
  const pending = track(connectivity.refresh());
  await clock.settle();
  expect(pending.settled).toBe(0);
  connectivity.stop();
  await clock.settle();
  expect(pending.settled).toBe(1);
  expect(pending.error).toMatchObject({ code: "ERR_CONNECTIVITY_CANCELLED" });
  await clock.advance(20000);
  expect(pending.settled).toBe(1);
  probe.setOutcome("reachable");
  expect((await connectivity.refresh()).isInternetReachable).toBe("reachable");
});

test("emptyArgumentIsInvalidArgument", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  await expect(connectivity.fetch("")).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_INVALID_ARGUMENT" });
  expect(backend.snapshots()).toBe(0);
});

test("timeoutErrorPropagatesWithCode", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot("ERR_CONNECTIVITY_TIMEOUT");
  await expect(connectivity.fetch()).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_TIMEOUT" });
  await expect(connectivity.refresh()).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_TIMEOUT" });
});

test("backendErrorPropagatesStatus", async () => {
  const { connectivity, backend } = setup(scriptedProbe("reachable").probe);
  backend.setSnapshot("ERR_CONNECTIVITY_BACKEND");
  await expect(connectivity.refresh()).rejects.toMatchObject({ code: "ERR_CONNECTIVITY_BACKEND" });
});

test("probeOutcomesClassifyOnlyA204AsReachable", async () => {
  const outcomes: ProbeOutcome[] = [];
  for (const status of [204, 200, 302, 511]) outcomes.push(await httpProbe(answering(status))("https://x.test/", 1000));
  expect(outcomes).toEqual(["reachable", "captivePortal", "captivePortal", "captivePortal"]);
});
