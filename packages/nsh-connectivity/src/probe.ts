import type { Probe } from "./backend";
import type { ProbeOutcome } from "./types";

/** The part of `fetch` the probe uses. */
export type FetchLike = (url: string, init: { method: string; signal?: AbortSignal }) => Promise<{ status: number }>;

/** 204 is reachable; any other complete response is a portal answering for the Internet. */
export function classify(status: number): ProbeOutcome {
  return status === 204 ? "reachable" : "captivePortal";
}

/**
 * The HTTP probe over `fetchImpl` (the global `fetch` by default): a GET, aborted at its
 * timeout. A timeout or a transport error is unreachable, never a rejection.
 */
export function httpProbe(fetchImpl?: FetchLike): Probe {
  return async (url, timeoutMs) => {
    const run = fetchImpl ?? (globalThis.fetch as unknown as FetchLike | undefined);
    if (run === undefined) return "unreachable";
    try {
      const signal = typeof AbortSignal.timeout === "function" ? AbortSignal.timeout(timeoutMs) : undefined;
      const response = await run(url, { method: "GET", signal });
      return classify(response.status);
    } catch {
      return "unreachable";
    }
  };
}
