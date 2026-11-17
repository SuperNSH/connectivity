import { type ConnectivityBackend, type Probe, SYSTEM_TIMERS, type Timers } from "./backend";
import { asConnectivityError, ConnectivityError } from "./errors";
import { httpProbe } from "./probe";
import { rustBackend } from "./rust";
import {
  type Configuration,
  type ConnectionType,
  DEFAULT_CONFIGURATION,
  listed,
  type NetworkState,
  type PlatformSnapshot,
  type ProbeOutcome,
  type Reachability,
  sameValue,
  stateOf,
} from "./types";

/** What a `Connectivity` is built over; each part defaults to the real one. */
export interface ConnectivityOptions {
  /** The platform; `rustBackend()` by default. */
  backend?: ConnectivityBackend;
  /** The reachability probe; `httpProbe()` over the global `fetch` by default. */
  probe?: Probe;
  /** The timers debouncing and probe deadlines use; the global ones by default. */
  timers?: Timers;
  /** Over the defaults. */
  configuration?: Partial<Configuration>;
}

type StateListener = (state: NetworkState) => void;
type TypesListener = (types: ConnectionType[]) => void;

/**
 * The connectivity of this device: connectivity_plus's `checkConnectivity` and
 * `onConnectivityChanged`, and NetInfo's `fetch`, `refresh` and `addEventListener`. Every call
 * resolves once: a value, a coded `ConnectivityError`, or `ERR_CONNECTIVITY_CANCELLED`.
 */
export class Connectivity {
  private readonly backend: ConnectivityBackend;
  private readonly probe: Probe;
  private readonly timers: Timers;
  private config: Configuration;
  private current: NetworkState | null = null;
  private types: ConnectionType[] | null = null;
  private reach: [Reachability, boolean] = ["unknown", false];
  private inflight: Promise<NetworkState> | null = null;
  private readonly cancels = new Set<(error: ConnectivityError) => void>();
  private readonly stateListeners = new Set<StateListener>();
  private readonly typesListeners = new Set<TypesListener>();
  private lastTypes: ConnectionType[] | null = null;
  /** Bumped by `stop` and by the last listener leaving: a running watch from before is stale. */
  private generation = 0;
  private watching = false;
  private hubOpen = false;
  /** Listeners waiting for the hub's first state. */
  private opening: (() => void)[] | null = null;
  private debounce: unknown = null;

  constructor(options: ConnectivityOptions = {}) {
    this.backend = options.backend ?? rustBackend();
    this.probe = options.probe ?? httpProbe();
    this.timers = options.timers ?? SYSTEM_TIMERS;
    this.config = { ...DEFAULT_CONFIGURATION, ...options.configuration };
  }

  /** The live types, strongest first: never empty, `["none"]` alone when nothing is live. */
  checkConnectivity(): Promise<ConnectionType[]> {
    return this.guarded(async () => listed((await this.snapshot()).types));
  }

  /**
   * Calls `listener` with distinct, never-empty type lists, starting with the current one.
   * Returns the function that stops it.
   */
  onConnectivityChanged(listener: TypesListener): () => void {
    this.typesListeners.add(listener);
    this.openHub(() => {
      if (this.typesListeners.has(listener) && this.types !== null) listener(listed(this.types));
    });
    return () => this.leave(() => this.typesListeners.delete(listener));
  }

  /**
   * The state now, from a fresh snapshot and the last probe answer for the same path. With
   * `interfaceName`, that interface's state; a name the platform does not report rejects
   * `ERR_CONNECTIVITY_UNKNOWN_INTERFACE`, an empty one `ERR_CONNECTIVITY_INVALID_ARGUMENT`.
   */
  fetch(interfaceName?: string): Promise<NetworkState> {
    if (interfaceName === "") return Promise.reject(new ConnectivityError("ERR_CONNECTIVITY_INVALID_ARGUMENT"));
    return this.guarded(async () => {
      const snapshot = await this.snapshot();
      const state = this.absorb(snapshot, null);
      if (interfaceName === undefined) return state;
      const found = snapshot.interface;
      if (found === null || found.name !== interfaceName) {
        throw new ConnectivityError("ERR_CONNECTIVITY_UNKNOWN_INTERFACE");
      }
      return { ...state, connectionType: found.type };
    });
  }

  /** A fresh snapshot and a fresh probe. Concurrent callers share one cycle. */
  refresh(): Promise<NetworkState> {
    if (this.inflight === null) {
      const cycle = this.cycle();
      this.inflight = cycle;
      const clear = () => {
        if (this.inflight === cycle) this.inflight = null;
      };
      cycle.then(clear, clear);
    }
    const shared = this.inflight;
    return this.guarded(() => shared);
  }

  /** The last state this handle knew, without a probe; a fresh snapshot when it knows none. */
  state(): Promise<NetworkState> {
    const current = this.current;
    if (current !== null) return Promise.resolve(current);
    return this.fetch();
  }

  /** Merges `config` over the current configuration; the next read uses it. */
  configure(config: Partial<Configuration>): void {
    this.config = { ...this.config, ...config };
  }

  /** The configuration now. */
  configuration(): Configuration {
    return { ...this.config };
  }

  /**
   * Calls `listener` with the debounced state, starting with the current one. Returns the
   * function that stops it; the platform watch ends with the last listener.
   */
  addEventListener(listener: StateListener): () => void {
    this.stateListeners.add(listener);
    this.openHub(() => {
      if (this.stateListeners.has(listener) && this.current !== null) listener(this.current);
    });
    return () => this.leave(() => this.stateListeners.delete(listener));
  }

  /** Cancels in-flight work, the platform watch and every listener. Idempotent. */
  stop(): void {
    const cancelled = new ConnectivityError("ERR_CONNECTIVITY_CANCELLED");
    for (const cancel of [...this.cancels]) cancel(cancelled);
    this.cancels.clear();
    this.inflight = null;
    this.stateListeners.clear();
    this.typesListeners.clear();
    this.closeHub();
  }

  private async snapshot(): Promise<PlatformSnapshot> {
    try {
      return await this.backend.snapshot();
    } catch (error) {
      throw asConnectivityError(error);
    }
  }

  /** Runs `work` unless `stop` lands first, which rejects `ERR_CONNECTIVITY_CANCELLED`. */
  private guarded<T>(work: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      let settled = false;
      const cancel = (error: ConnectivityError) => {
        if (settled) return;
        settled = true;
        reject(error);
      };
      this.cancels.add(cancel);
      work().then(
        (value) => {
          this.cancels.delete(cancel);
          if (settled) return;
          settled = true;
          resolve(value);
        },
        (error: unknown) => {
          this.cancels.delete(cancel);
          if (settled) return;
          settled = true;
          reject(asConnectivityError(error));
        },
      );
    });
  }

  private async cycle(): Promise<NetworkState> {
    const snapshot = await this.snapshot();
    if (snapshot.types.length === 0) return this.absorb(snapshot, null);
    const outcome = await this.runProbe();
    return this.absorb(snapshot, outcome);
  }

  /** The probe, bounded by the configured timeout whatever the probe does. */
  private runProbe(): Promise<ProbeOutcome> {
    const { reachabilityUrl, reachabilityTimeoutMs } = this.config;
    return new Promise<ProbeOutcome>((resolve) => {
      let done = false;
      const finish = (outcome: ProbeOutcome) => {
        if (done) return;
        done = true;
        this.timers.clearTimeout(timer);
        resolve(outcome);
      };
      const timer = this.timers.setTimeout(() => finish("unreachable"), reachabilityTimeoutMs);
      this.probe(reachabilityUrl, reachabilityTimeoutMs).then(finish, () => finish("unreachable"));
    });
  }

  /** Records the state a snapshot gives and tells the listeners when it changed. */
  private absorb(snapshot: PlatformSnapshot, outcome: ProbeOutcome | null): NetworkState {
    const samePath = this.types !== null && sameValue(this.types, snapshot.types);
    let reach: [Reachability, boolean];
    if (outcome === "reachable") reach = ["reachable", false];
    else if (outcome === "captivePortal") reach = ["unreachable", true];
    else if (outcome === "unreachable") reach = ["unreachable", false];
    else reach = samePath ? this.reach : ["unknown", false];
    const state = stateOf(snapshot, reach[0], reach[1]);
    const changed = this.current === null || !sameValue(this.current, state);
    this.reach = reach;
    this.types = [...snapshot.types];
    this.current = state;
    // While the hub opens, its listeners get the first state from `openHub`, not here.
    if (this.hubOpen && this.opening === null) {
      if (changed) for (const listener of [...this.stateListeners]) listener(state);
      const types = listed(snapshot.types);
      if (this.lastTypes === null || !sameValue(this.lastTypes, types)) {
        this.lastTypes = types;
        for (const listener of [...this.typesListeners]) listener(types);
      }
    }
    return state;
  }

  /** Starts the watch on the first listener; `deliver` hands the new listener the current value. */
  private openHub(deliver: () => void): void {
    if (this.opening !== null) {
      this.opening.push(deliver);
      return;
    }
    if (this.hubOpen) {
      queueMicrotask(deliver);
      return;
    }
    this.hubOpen = true;
    const waiting = [deliver];
    this.opening = waiting;
    const generation = this.generation;
    const opened = (known: boolean) => {
      if (generation !== this.generation) return;
      this.opening = null;
      if (known) {
        this.lastTypes = listed(this.types ?? []);
        for (const run of waiting) run();
      }
      this.startWatch(generation);
    };
    // No state to deliver (an unsupported platform): the listeners simply hear nothing.
    this.state().then(
      () => opened(true),
      () => opened(false),
    );
  }

  private startWatch(generation: number): void {
    this.backend.watch(() => this.onEvent(generation)).then(
      () => {
        if (generation === this.generation) this.watching = true;
        else this.backend.stopWatching();
      },
      // A watch error: no events, snapshots still work.
      () => {},
    );
  }

  /** Each event restarts the debounce window; when it closes, one re-read. */
  private onEvent(generation: number): void {
    if (generation !== this.generation) return;
    if (this.debounce !== null) this.timers.clearTimeout(this.debounce);
    this.debounce = this.timers.setTimeout(() => {
      this.debounce = null;
      void this.onChange(generation);
    }, this.config.debounceMs);
  }

  private async onChange(generation: number): Promise<void> {
    try {
      const snapshot = await this.snapshot();
      if (generation !== this.generation) return;
      const outcome = this.config.probeOnEvents && snapshot.types.length > 0 ? await this.runProbe() : null;
      if (generation !== this.generation) return;
      this.absorb(snapshot, outcome);
    } catch {
      // A failed snapshot publishes nothing; the next event tries again.
    }
  }

  private leave(remove: () => void): void {
    remove();
    if (this.hubOpen && this.stateListeners.size === 0 && this.typesListeners.size === 0) this.closeHub();
  }

  private closeHub(): void {
    this.generation += 1;
    this.hubOpen = false;
    this.opening = null;
    this.lastTypes = null;
    if (this.debounce !== null) {
      this.timers.clearTimeout(this.debounce);
      this.debounce = null;
    }
    if (this.watching) {
      this.watching = false;
      this.backend.stopWatching();
    }
  }
}
