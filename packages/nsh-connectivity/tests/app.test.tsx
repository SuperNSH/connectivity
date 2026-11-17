import { expect, open, test } from "nsh/test";

test("the screen shows the platform's types or a coded error, and stop says so", async () => {
  // The real Rust backend answers: a snapshot only, so nothing goes over the network.
  const app = await open("../App.tsx");
  await app.waitFor(
    "text:/^(types (none|unknown|cellular|wifi|bluetooth|ethernet|wimax|vpn|satellite|other)(,[a-z]+)*|error ERR_CONNECTIVITY_[A-Z_]+)$/",
  );
  await app.press("id:stop");
  await app.waitFor("text:stopped");
  expect(app.shows("text:stopped")).toBe(true);
});

test("useNetInfo follows the state and unsubscribes with its component", async () => {
  const app = await open("./netinfo-app.tsx", { clock: {} });
  await app.waitFor("text:wifi unknown");
  await app.press("id:ethernet");
  await app.waitFor("text:ethernet reachable");
  expect(app.texts()).toContain("watchers 1");
  await app.press("id:hide");
  await app.waitFor("text:stopped 1");
});
