import { expect, test } from "nsh/test";
import { asConnectivityError, ConnectivityError, type ConnectivityErrorCode } from "../src";

/** The same wording as the Rust crate's `ErrorKind::message`. */
const MESSAGES: Record<ConnectivityErrorCode, string> = {
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

test("everyCodeHasTheRustWording", () => {
  for (const code of Object.keys(MESSAGES) as ConnectivityErrorCode[]) {
    const error = new ConnectivityError(code);
    expect(error.code).toBe(code);
    expect(error.message).toBe(MESSAGES[code]);
    expect(error.name).toBe("ConnectivityError");
  }
});

test("aNativeRejectionKeepsItsCodeAndAnUnknownOneIsBackend", () => {
  expect(asConnectivityError({ code: "ERR_CONNECTIVITY_TIMEOUT", message: "x" }).code).toBe("ERR_CONNECTIVITY_TIMEOUT");
  expect(asConnectivityError({ code: "ERR_NSH_HOST_ERROR", message: "ERR_CONNECTIVITY_CANCELLED: gone" }).code).toBe(
    "ERR_CONNECTIVITY_CANCELLED",
  );
  expect(asConnectivityError(new TypeError("boom")).code).toBe("ERR_CONNECTIVITY_BACKEND");
  expect(asConnectivityError(null).code).toBe("ERR_CONNECTIVITY_BACKEND");
});
