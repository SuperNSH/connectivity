/** The codes a connectivity rejection carries. Apps match on `code`. */
export type ConnectivityErrorCode =
  | "ERR_CONNECTIVITY_NOT_AVAILABLE"
  | "ERR_CONNECTIVITY_PERMISSION_DENIED"
  | "ERR_CONNECTIVITY_CANCELLED"
  | "ERR_CONNECTIVITY_TIMEOUT"
  | "ERR_CONNECTIVITY_UNKNOWN_INTERFACE"
  | "ERR_CONNECTIVITY_BACKEND"
  | "ERR_CONNECTIVITY_INVALID_ARGUMENT";

/** Same wording as the Rust crate's `ErrorKind::message`. */
export const MESSAGES: Record<ConnectivityErrorCode, string> = {
  ERR_CONNECTIVITY_NOT_AVAILABLE:
    "The platform has no connectivity API here (an unsupported target, or the web lacks the API).",
  ERR_CONNECTIVITY_PERMISSION_DENIED:
    "The OS refused the query, such as Android's missing location permission for SSID.",
  ERR_CONNECTIVITY_CANCELLED:
    "The caller cancelled: dropping the future, calling stop, or a cancel from the platform.",
  ERR_CONNECTIVITY_TIMEOUT: "The operation gave up at its deadline instead of hanging.",
  ERR_CONNECTIVITY_UNKNOWN_INTERFACE: "fetch() named an interface the platform does not have.",
  ERR_CONNECTIVITY_BACKEND:
    "The platform's own service failed; the error carries the platform status where it gives one.",
  ERR_CONNECTIVITY_INVALID_ARGUMENT: "An argument is empty or not allowed.",
};

const KNOWN = new Set<string>(Object.keys(MESSAGES));

/** A rejection of the connectivity API. `code` is the stable name. */
export class ConnectivityError extends Error {
  readonly code: ConnectivityErrorCode;

  constructor(code: ConnectivityErrorCode) {
    super(MESSAGES[code]);
    this.name = "ConnectivityError";
    this.code = code;
  }
}

/** Whether `code` is one of the codes above. */
export function isConnectivityErrorCode(code: unknown): code is ConnectivityErrorCode {
  return typeof code === "string" && KNOWN.has(code);
}

/**
 * Turns whatever the native side or a host threw into a coded `ConnectivityError`. A host's
 * rejection may carry `ERR_NSH_HOST_ERROR` with the code leading its message. Anything unknown
 * is a backend failure.
 */
export function asConnectivityError(error: unknown): ConnectivityError {
  if (error instanceof ConnectivityError) return error;
  const raw = (error ?? {}) as { code?: unknown; message?: unknown };
  if (isConnectivityErrorCode(raw.code)) return new ConnectivityError(raw.code);
  const message = typeof raw.message === "string" ? raw.message : "";
  const named = /^(ERR_CONNECTIVITY_[A-Z_]+)\b/.exec(message)?.[1];
  return isConnectivityErrorCode(named) ? new ConnectivityError(named) : new ConnectivityError("ERR_CONNECTIVITY_BACKEND");
}
