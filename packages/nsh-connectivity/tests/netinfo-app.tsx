import { mount, signal, Text, View } from "nsh";
import { Connectivity, memoryBackend, scriptedProbe, useNetInfo } from "../src";

// A screen for the useNetInfo test: a memory backend the buttons drive, on the app's clock.
const backend = memoryBackend();
const connectivity = new Connectivity({ backend, probe: scriptedProbe("reachable").probe });
const shown = signal(true);
const watchers = signal(0);
const stopped = signal(0);

function Status() {
  const state = useNetInfo({ debounceMs: 500 }, connectivity);
  return <Text testID="state">{`${state().connectionType} ${state().isInternetReachable}`}</Text>;
}

function App() {
  const ethernet = () => {
    backend.setSnapshot({ types: ["ethernet"], interface: null, isExpensive: false });
    backend.emitEvent();
    watchers.set(backend.watchers());
  };
  const hide = () => {
    shown.set(false);
    // Read after the component's cleanup ran.
    setTimeout(() => stopped.set(backend.stopped()), 0);
  };
  return (
    <View class="flex-1 gap-3 p-4">
      <If when={shown()}>
        <Status />
      </If>
      <Text testID="watchers">{`watchers ${watchers()}`}</Text>
      <Text testID="stopped">{`stopped ${stopped()}`}</Text>
      <Text testID="ethernet" role="button" onPress={ethernet}>
        Ethernet
      </Text>
      <Text testID="hide" role="button" onPress={hide}>
        Hide
      </Text>
    </View>
  );
}

mount(App);
