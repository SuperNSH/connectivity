import { NativeConnectivity } from "nsh-connectivity/native";
import type { ConnectivityBackend } from "./backend";
import { asConnectivityError } from "./errors";
import type { ConnectionType, PlatformEvent, PlatformSnapshot } from "./types";

/**
 * The Rust crate's platform backend (`nsh-connectivity/native`). The app or package must list
 * the crate under `"rust"` in `nsh.json`; importing this module without it fails the bundle.
 */
export function rustBackend(): ConnectivityBackend {
  return new RustBackend();
}

class RustBackend implements ConnectivityBackend {
  private native: NativeConnectivity | null = null;

  private handle(): NativeConnectivity {
    this.native ??= new NativeConnectivity();
    return this.native;
  }

  async snapshot(): Promise<PlatformSnapshot> {
    try {
      const raw = await this.handle().snapshot();
      return {
        types: raw.types as ConnectionType[],
        interface:
          raw.interface === null || raw.interface === undefined
            ? null
            : {
                name: raw.interface.name,
                type: raw.interface.kind as ConnectionType,
                ipAddress: raw.interface.ipAddress ?? null,
                subnet: raw.interface.subnet ?? null,
                ssid: raw.interface.ssid ?? null,
              },
        isExpensive: raw.isExpensive,
      };
    } catch (error) {
      throw asConnectivityError(error);
    }
  }

  async watch(sink: (event: PlatformEvent) => void): Promise<void> {
    const native = this.handle();
    try {
      await native.watch();
    } catch (error) {
      throw asConnectivityError(error);
    }
    // Pulls events until the watch stops; `nextEvent` resolves false then.
    void (async () => {
      try {
        while (await native.nextEvent()) sink({});
      } catch {
        // The native object went away: the stream simply ends.
      }
    })();
  }

  stopWatching(): void {
    this.native?.stopWatching();
  }
}
