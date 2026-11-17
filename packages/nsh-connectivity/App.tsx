import { mount, signal, Text, View } from "nsh";
import { Connectivity, ConnectivityError } from "./src";

/** The device's connectivity, through the Rust crate. Only snapshots: this screen never probes. */
const connectivity = new Connectivity();

const message = signal("checking");

function codeOf(error: unknown): string {
  return error instanceof ConnectivityError ? error.code : "unknown error";
}

connectivity.checkConnectivity().then(
  (types) => message.set(`types ${types.join(",")}`),
  (error: unknown) => message.set(`error ${codeOf(error)}`),
);

function App() {
  const stop = () => {
    connectivity.stop();
    message.set("stopped");
  };
  return (
    <View class="flex-1 gap-3 p-4">
      <Text testID="title" class="text-foreground">nsh-connectivity</Text>
      <Text testID="status" class="text-foreground">{message()}</Text>
      <Text testID="stop" role="button" onPress={stop} class="rounded-(--radius) bg-muted px-3 py-2">
        Stop
      </Text>
    </View>
  );
}

mount(App);
