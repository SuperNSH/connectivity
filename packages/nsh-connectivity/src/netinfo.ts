import { type Accessor, cleanup, signal } from "nsh";
import { Connectivity } from "./state";
import { type Configuration, type NetworkState, UNKNOWN_STATE } from "./types";

let shared: Connectivity | null = null;

/** The handle the module-level functions use, made on first use over the Rust backend. */
export function defaultConnectivity(): Connectivity {
  shared ??= new Connectivity();
  return shared;
}

/** Merges `config` over the current configuration; the next read uses it. */
export function configure(config: Partial<Configuration>): void {
  defaultConnectivity().configure(config);
}

/** NetInfo's `fetch`: the state now, or one interface's. */
export function fetch(interfaceName?: string): Promise<NetworkState> {
  return defaultConnectivity().fetch(interfaceName);
}

/** NetInfo's `refresh`: a fresh snapshot and probe, shared by concurrent callers. */
export function refresh(): Promise<NetworkState> {
  return defaultConnectivity().refresh();
}

/** NetInfo's `addEventListener`: the debounced state, the current one first. */
export function addEventListener(listener: (state: NetworkState) => void): () => void {
  return defaultConnectivity().addEventListener(listener);
}

/**
 * NetInfo's `useNetInfo` as an NSH accessor: the state, starting as `UNKNOWN_STATE` until the
 * first delivery. Call it in a component; the subscription ends with it. `connectivity`
 * replaces the default handle (a test's memory backend).
 */
export function useNetInfo(config?: Partial<Configuration>, connectivity?: Connectivity): Accessor<NetworkState> {
  const handle = connectivity ?? defaultConnectivity();
  if (config !== undefined) handle.configure(config);
  const state = signal<NetworkState>(UNKNOWN_STATE);
  const stop = handle.addEventListener((next) => state.set(next));
  cleanup(stop);
  return state;
}
