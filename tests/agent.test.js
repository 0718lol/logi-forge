import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { MockAgent, renderConfig } from "../agent.js";
import { AgentError } from "../protocol.js";

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "logi-forge-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return { directory, stateFile: join(directory, "state.json") };
}

function expectCode(code, callback) {
  assert.throws(callback, (error) => error instanceof AgentError && error.code === code);
}

test("snapshot exposes a versioned, capability-gated inventory", () => {
  const snapshot = new MockAgent().snapshot();
  assert.equal(snapshot.protocolVersion, 1);
  assert.equal(snapshot.upstreamBaselineProtocol, 29);
  assert.equal(snapshot.inventoryStatus, "ready");
  assert.equal(snapshot.devices.length, 4);
  assert.ok(snapshot.devices.find((device) => device.kind === "Mouse").capabilities.includes("pointer"));
  assert.match(snapshot.configPreviews["unit:6be9d300"], /dpi = 1600/);
});

test("validated writes increment revision and survive restart", (t) => {
  const { stateFile } = fixture(t);
  const agent = new MockAgent({ stateFile });
  const first = agent.writeDevice("unit:6be9d300", "dpi", 2400, 0);
  assert.equal(first.revision, 1);
  assert.equal(first.devices[0].dpi, 2400);
  assert.equal(JSON.parse(readFileSync(stateFile, "utf8")).revision, 1);

  const restarted = new MockAgent({ stateFile }).snapshot();
  assert.equal(restarted.revision, 1);
  assert.equal(restarted.devices[0].dpi, 2400);
});

test("agent returns typed errors for hardware and concurrency boundaries", () => {
  const agent = new MockAgent();
  expectCode("UNSUPPORTED_FEATURE", () => agent.writeDevice("camera:brio:mock-01", "dpi", 1600, 0));
  expectCode("DEVICE_OFFLINE", () => agent.writeDevice("light:litra:mock-01", "light.brightness", 80, 0));
  expectCode("INVALID_VALUE", () => agent.writeDevice("unit:6be9d300", "dpi", 10000, 0));

  agent.writeDevice("unit:6be9d300", "dpi", 2400, 0);
  expectCode("REVISION_CONFLICT", () => agent.writeDevice("unit:6be9d300", "dpi", 3200, 0));
});

test("pairing and DPI commands mutate through the same snapshot contract", () => {
  const agent = new MockAgent();
  const paired = agent.runCommand("pair");
  assert.equal(paired.devices.find((device) => device.kind === "Light").online, true);

  const cycled = agent.runCommand("cycle-dpi", { deviceKey: "unit:6be9d300" });
  assert.equal(cycled.devices.find((device) => device.kind === "Mouse").dpi, 2400);
  assert.equal(cycled.revision, 2);
});

test("TOML preview escapes user-controlled device names", () => {
  const device = new MockAgent().snapshot().devices[0];
  device.name = 'Desk "Mouse"\\Primary';
  const config = renderConfig(device);
  assert.match(config, /custom_name = "Desk \\"Mouse\\"\\\\Primary"/);
});

test("keyboard TOML preview matches the native device-scoped schema", () => {
  const keyboard = new MockAgent().snapshot().devices.find((device) => device.kind === "Keyboard");
  const config = renderConfig(keyboard);
  assert.match(config, /\[devices\."unit:keyboard77"\.keyboard\]/);
  assert.match(config, /fn_lock = true/);
  assert.match(config, /\[devices\."unit:keyboard77"\.keyboard\.lighting\]/);
  assert.match(config, /color = "18a06f"/);
  assert.match(config, /\[devices\."unit:keyboard77"\.keyboard\.bindings\]/);
  assert.doesNotMatch(config, /^\[keyboard\.bindings\]$/m);
});
