import type { PlatformEvent, PlatformSnapshot, ProbeOutcome } from "./types";

/** What a `Connectivity` runs on: the Rust crate, or a memory backend in tests. */
export interface ConnectivityBackend {
  /** The platform's view now. Resolves or rejects; never hangs. */
  snapshot(): Promise<PlatformSnapshot>;
  /** Starts the change stream: `sink` once per native notification. A rejection means no stream. */
  watch(sink: (event: PlatformEvent) => void): Promise<void>;
  /** Stops the change stream. Idempotent; no event follows it. */
  stopWatching(): void;
}

/** GETs `url` and classifies the answer; the caller also bounds it by `timeoutMs`. */
export type Probe = (url: string, timeoutMs: number) => Promise<ProbeOutcome>;

/** The timers a `Connectivity` uses: the global ones, or a manual clock in tests. */
export interface Timers {
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export const SYSTEM_TIMERS: Timers = {
  setTimeout: (run, ms) => setTimeout(run, ms),
  clearTimeout: (handle) => clearTimeout(handle as number),
};
