/** A kind of connection: the union of connectivity_plus's and NetInfo's sets. */
export type ConnectionType =
  | "unknown"
  | "none"
  | "cellular"
  | "wifi"
  | "bluetooth"
  | "ethernet"
  | "wimax"
  | "vpn"
  | "satellite"
  | "other";

/** NetInfo's `boolean | null` reachability as an explicit tri-state. */
export type Reachability = "unknown" | "reachable" | "unreachable";

/** A cellular network's generation. */
export type CellularGeneration = "2g" | "3g" | "4g" | "5g";

/** What one reachability probe found. */
export type ProbeOutcome = "reachable" | "captivePortal" | "unreachable";

/** The interface a backend reports for the current path. */
export interface InterfaceInfo {
  name: string;
  type: ConnectionType;
  ipAddress: string | null;
  subnet: string | null;
  /** `null` when unread or not permitted. */
  ssid: string | null;
}

/** What is known about a live connection (NetInfo's `details`). */
export interface NetworkDetails {
  isConnectionExpensive: boolean;
  /** The last probe got a non-204 answer. */
  captivePortal: boolean;
  ssid: string | null;
  bssid: string | null;
  strength: number | null;
  ipAddress: string | null;
  subnet: string | null;
  frequencyMhz: number | null;
  linkSpeedMbps: number | null;
  cellularGeneration: CellularGeneration | null;
  carrier: string | null;
}

/** The network as an app sees it. `details` is `null` exactly when `isConnected` is false. */
export interface NetworkState {
  connectionType: ConnectionType;
  isConnected: boolean;
  isInternetReachable: Reachability;
  details: NetworkDetails | null;
}

/** What a backend reports for one moment. */
export interface PlatformSnapshot {
  /** The live types, strongest first; empty means nothing is live. */
  types: ConnectionType[];
  interface: InterfaceInfo | null;
  isExpensive: boolean;
}

/** A nudge to re-read the snapshot; it carries no data. */
export type PlatformEvent = Record<string, never>;

/** How the probe and the event pipeline behave. */
export interface Configuration {
  /** The URL the probe GETs; 204 means reachable. */
  reachabilityUrl: string;
  reachabilityTimeoutMs: number;
  /** How long platform events must stay quiet before the state is re-read. */
  debounceMs: number;
  /** Probe after each debounced change. */
  probeOnEvents: boolean;
}

export const DEFAULT_CONFIGURATION: Configuration = {
  reachabilityUrl: "https://connectivitycheck.gstatic.com/generate_204",
  reachabilityTimeoutMs: 10000,
  debounceMs: 500,
  probeOnEvents: true,
};

/** Before anything is known: what `useNetInfo` shows until the first delivery. */
export const UNKNOWN_STATE: NetworkState = {
  connectionType: "unknown",
  isConnected: false,
  isInternetReachable: "unknown",
  details: null,
};

/** connectivity_plus's never-empty list: `["none"]` alone when nothing is live. */
export function listed(types: readonly ConnectionType[]): ConnectionType[] {
  return types.length === 0 ? ["none"] : [...types];
}

/** The state a snapshot gives, with the reachability known for it. */
export function stateOf(snapshot: PlatformSnapshot, reach: Reachability, captivePortal: boolean): NetworkState {
  const first = snapshot.types[0];
  if (first === undefined) {
    return { connectionType: "none", isConnected: false, isInternetReachable: "unreachable", details: null };
  }
  const iface = snapshot.interface;
  return {
    connectionType: first,
    isConnected: true,
    isInternetReachable: reach,
    details: {
      isConnectionExpensive: snapshot.isExpensive,
      captivePortal,
      ssid: iface?.ssid ?? null,
      bssid: null,
      strength: null,
      ipAddress: iface?.ipAddress ?? null,
      subnet: iface?.subnet ?? null,
      frequencyMhz: null,
      linkSpeedMbps: null,
      cellularGeneration: null,
      carrier: null,
    },
  };
}

/** Structural equality for the plain data above. */
export function sameValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
