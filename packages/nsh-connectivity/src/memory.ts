import type { ConnectivityBackend, Probe, Timers } from "./backend";
import { type ConnectivityErrorCode, ConnectivityError } from "./errors";
import type { PlatformEvent, PlatformSnapshot, ProbeOutcome } from "./types";

/** A backend the test scripts and drives, the twin of the Rust crate's `FakeBackend`. */
export interface MemoryBackend extends ConnectivityBackend {
  /** What the next snapshots answer: a snapshot, or a code they reject with. */
  setSnapshot(answer: PlatformSnapshot | ConnectivityErrorCode): void;
  /** Makes the next `watch` reject with `code`; `null` lets it succeed. */
  setWatchError(code: ConnectivityErrorCode | null): void;
  /** The platform has news: one event to the watcher. True when a watcher existed. */
  emitEvent(): boolean;
  /** How many `watch` calls succeeded. */
  watchers(): number;
  /** How many times `stopWatching` ran. */
  stopped(): number;
  /** How many snapshots were read. */
  snapshots(): number;
}

/** A device on Wi-Fi (`wlan0`), connected. */
export const WIFI_SNAPSHOT: PlatformSnapshot = {
  types: ["wifi"],
  interface: { name: "wlan0", type: "wifi", ipAddress: "192.168.1.20", subnet: "255.255.255.0", ssid: "Home" },
  isExpensive: false,
};

/** A memory backend, starting as a device on Wi-Fi. */
export function memoryBackend(): MemoryBackend {
  let answer: PlatformSnapshot | ConnectivityErrorCode = WIFI_SNAPSHOT;
  let watchError: ConnectivityErrorCode | null = null;
  let sink: ((event: PlatformEvent) => void) | null = null;
  let watchers = 0;
  let stopped = 0;
  let snapshots = 0;
  return {
    snapshot() {
      snapshots += 1;
      const now = answer;
      return typeof now === "string" ? Promise.reject(new ConnectivityError(now)) : Promise.resolve(structuredCopy(now));
    },
    watch(next) {
      if (watchError !== null) {
        const code = watchError;
        watchError = null;
        return Promise.reject(new ConnectivityError(code));
      }
      sink = next;
      watchers += 1;
      return Promise.resolve();
    },
    stopWatching() {
      sink = null;
      stopped += 1;
    },
    setSnapshot(next) {
      answer = next;
    },
    setWatchError(code) {
      watchError = code;
    },
    emitEvent() {
      if (sink === null) return false;
      sink({});
      return true;
    },
    watchers: () => watchers,
    stopped: () => stopped,
    snapshots: () => snapshots,
  };
}

function structuredCopy(snapshot: PlatformSnapshot): PlatformSnapshot {
  return {
    types: [...snapshot.types],
    interface: snapshot.interface === null ? null : { ...snapshot.interface },
    isExpensive: snapshot.isExpensive,
  };
}

/** A probe the test scripts: an outcome, or no answer at all (`null`). */
export interface ScriptedProbe {
  readonly probe: Probe;
  /** What later probes answer; `null` never answers. */
  setOutcome(outcome: ProbeOutcome | null): void;
  /** How many probes ran. */
  probes(): number;
  /** The URLs probed, in order. */
  urls(): string[];
}

export function scriptedProbe(outcome: ProbeOutcome | null): ScriptedProbe {
  let next = outcome;
  const urls: string[] = [];
  return {
    probe: (url) => {
      urls.push(url);
      const now = next;
      return now === null ? new Promise<ProbeOutcome>(() => {}) : Promise.resolve(now);
    },
    setOutcome(value) {
      next = value;
    },
    probes: () => urls.length,
    urls: () => [...urls],
  };
}

/** Timers that move only when the test says: the TS twin of tokio's paused clock. */
export interface ManualClock extends Timers {
  /** Milliseconds moved so far. */
  now(): number;
  /** Moves the clock by `ms`, running the timers due in order and the work they start. */
  advance(ms: number): Promise<void>;
  /** Runs the work already queued (promise callbacks) without moving the clock. */
  settle(): Promise<void>;
}

export function manualClock(): ManualClock {
  let now = 0;
  let nextId = 1;
  const timers = new Map<number, { at: number; run: () => void }>();
  const settle = async () => {
    for (let i = 0; i < 50; i++) await Promise.resolve();
  };
  return {
    setTimeout(run, ms) {
      const id = nextId++;
      timers.set(id, { at: now + Math.max(0, ms), run });
      return id;
    },
    clearTimeout(handle) {
      timers.delete(handle as number);
    },
    now: () => now,
    settle,
    async advance(ms) {
      const target = now + ms;
      await settle();
      for (;;) {
        let due: [number, { at: number; run: () => void }] | null = null;
        for (const entry of timers) {
          if (entry[1].at <= target && (due === null || entry[1].at < due[1].at)) due = entry;
        }
        if (due === null) break;
        timers.delete(due[0]);
        now = due[1].at;
        due[1].run();
        await settle();
      }
      now = target;
      await settle();
    },
  };
}
