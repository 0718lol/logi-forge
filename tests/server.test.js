import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

async function waitForServer(url, child) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Server exited with ${child.exitCode}`);
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // The process has not bound its socket yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("Server did not become healthy");
}

async function waitForNative(base) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    const snapshot = await (await fetch(`${base}/api/v1/snapshot`)).json();
    if (snapshot.nativeAgent.status === "online") return snapshot;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("Native agent did not become healthy");
}

test("hosted server exposes UI, health, snapshots, and validated writes", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "logi-forge-server-"));
  const port = await freePort();
  const child = spawn(process.execPath, ["server.js"], {
    cwd: root,
    env: {
      ...process.env,
      PORT: String(port),
      LOGI_FORGE_STATE: join(directory, "state.json"),
      LOGI_FORGE_CONFIG: join(directory, "native-config.toml"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  t.after(() => {
    child.kill("SIGTERM");
    rmSync(directory, { recursive: true, force: true });
  });

  const base = `http://127.0.0.1:${port}`;
  await waitForServer(`${base}/health`, child);

  const page = await fetch(`${base}/`);
  assert.equal(page.status, 200);
  assert.match(await page.text(), /Logi Forge/);

  const before = await waitForNative(base);
  assert.ok(before.nativeAgent);
  assert.equal(before.nativeAgent.protocolVersion, 1);
  const write = await fetch(`${base}/api/v1/devices/${encodeURIComponent("unit:6be9d300")}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "dpi", value: 3200, revision: before.revision }),
  });
  assert.equal(write.status, 200);
  const after = await write.json();
  assert.equal(after.devices.find((device) => device.kind === "Mouse").dpi, 3200);

  const keyboardWrite = await fetch(`${base}/api/v1/devices/${encodeURIComponent("unit:keyboard77")}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "fnLock", value: false, revision: after.revision }),
  });
  assert.equal(keyboardWrite.status, 200);
  const keyboardAfter = await keyboardWrite.json();
  assert.equal(keyboardAfter.devices.find((device) => device.kind === "Keyboard").fnLock, false);
  assert.equal(keyboardAfter.nativeAgent.apply.status, "disabled");
  assert.match(readFileSync(join(directory, "native-config.toml"), "utf8"), /fn_lock = false/);

  const invalid = await fetch(`${base}/api/v1/devices/${encodeURIComponent("unit:6be9d300")}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "dpi", value: 99999, revision: keyboardAfter.revision }),
  });
  assert.equal(invalid.status, 400);
  assert.equal((await invalid.json()).error.code, "INVALID_VALUE");
});
