import { accessSync, constants, createReadStream, existsSync, readdirSync } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { MockAgent } from "./agent.js";
import { NativeAgentBridge } from "./native-agent-bridge.js";
import { AgentError, errorStatus } from "./protocol.js";

const root = resolve(fileURLToPath(new URL(".", import.meta.url)));
const stateFile = process.env.LOGI_FORGE_STATE || join(root, ".runtime", "state.json");
const agent = new MockAgent({ stateFile });
const port = Number(process.env.PORT || 3000);
const host = "0.0.0.0";
const clients = new Set();
let mutationQueue = Promise.resolve();
const nativeAgent = new NativeAgentBridge({ root, onSnapshot: () => sendSnapshot(currentSnapshot()) });

function currentSnapshot() {
  const base = agent.snapshot();
  const native = nativeAgent.view();
  return {
    ...base,
    runtime: {
      mode: native.status === "online" ? "native+demo" : "demo",
      nativeAvailable: native.status === "online",
    },
    transport: native.status === "online" ? "http+sse+native-unix" : base.transport,
    nativeAgent: native,
  };
}

function runMutation(operation) {
  const result = mutationQueue.then(operation, operation);
  mutationQueue = result.catch(() => {});
  return result;
}

const contentTypes = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
};

function json(response, status, payload) {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
    "Cache-Control": "no-store",
  });
  response.end(body);
}

async function readJson(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 64 * 1024) throw new AgentError("PAYLOAD_TOO_LARGE", "Request body exceeds 64 KiB");
    chunks.push(chunk);
  }
  if (!chunks.length) return {};
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new AgentError("INVALID_JSON", "Request body must be valid JSON");
  }
}

function sendSnapshot(snapshot) {
  const event = `event: snapshot\ndata: ${JSON.stringify(snapshot)}\n\n`;
  for (const client of clients) client.write(event);
}

function inspectDirectory(path, matcher) {
  if (!existsSync(path)) return { status: "missing", count: 0, nodes: [] };
  try {
    const nodes = readdirSync(path).filter(matcher).sort();
    return { status: nodes.length ? "detected" : "empty", count: nodes.length, nodes };
  } catch (error) {
    return { status: "denied", count: 0, nodes: [], error: error.code || "READ_FAILED" };
  }
}

function inspectPath(path) {
  if (!existsSync(path)) return { status: "missing", path };
  try {
    accessSync(path, constants.R_OK | constants.W_OK);
    return { status: "ready", path };
  } catch (error) {
    return { status: "denied", path, error: error.code || "ACCESS_FAILED" };
  }
}

function diagnostics() {
  const native = nativeAgent.view();
  const hidraw = inspectDirectory("/dev", (name) => name.startsWith("hidraw"));
  const input = inspectDirectory("/dev/input", (name) => name.startsWith("event"));
  return {
    generatedAt: new Date().toISOString(),
    runtime: {
      mode: native.status === "online" ? "native+demo" : "demo",
      nativeAvailable: native.status === "online",
    },
    host: {
      platform: process.platform,
      arch: process.arch,
      nodes: {
        hidraw,
        input,
        uinput: ["/dev/uinput", "/dev/input/uinput"].map(inspectPath),
      },
    },
    nativeAgent: {
      status: native.status,
      protocolVersion: native.protocolVersion ?? null,
      inventoryStatus: native.inventoryStatus ?? "unavailable",
      deviceCount: native.devices?.length ?? 0,
      configStatus: native.config?.status ?? "unavailable",
      applyStatus: native.apply?.status ?? "unavailable",
      error: native.error ?? null,
    },
  };
}

agent.on("snapshot", () => sendSnapshot(currentSnapshot()));

async function serveStatic(pathname, response) {
  const requested = pathname === "/" ? "index.html" : pathname.slice(1);
  const file = resolve(root, normalize(requested));
  if ((file !== root && !file.startsWith(`${root}${sep}`)) || file.startsWith(join(root, ".runtime"))) {
    json(response, 404, { error: { code: "NOT_FOUND", message: "Not found" } });
    return;
  }
  try {
    const info = await stat(file);
    if (!info.isFile()) throw new Error("not a file");
    response.writeHead(200, {
      "Content-Type": contentTypes[extname(file)] || "application/octet-stream",
      "Content-Length": info.size,
      "Cache-Control": extname(file) === ".html" ? "no-cache" : "public, max-age=60",
    });
    createReadStream(file).pipe(response);
  } catch {
    json(response, 404, { error: { code: "NOT_FOUND", message: "Not found" } });
  }
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host || "localhost"}`);
  try {
    if (request.method === "GET" && url.pathname === "/health") {
      json(response, 200, {
        status: "ok",
        ready: nativeAgent.view().status === "online",
        protocolVersion: currentSnapshot().protocolVersion,
        nativeAgent: nativeAgent.view().status,
      });
      return;
    }
    if (request.method === "GET" && url.pathname === "/api/v1/snapshot") {
      json(response, 200, currentSnapshot());
      return;
    }
    if (request.method === "GET" && url.pathname === "/api/v1/diagnostics") {
      json(response, 200, diagnostics());
      return;
    }
    if (request.method === "GET" && url.pathname === "/api/v1/events") {
      response.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });
      response.write(`event: snapshot\ndata: ${JSON.stringify(currentSnapshot())}\n\n`);
      clients.add(response);
      request.on("close", () => clients.delete(response));
      return;
    }
    const deviceMatch = url.pathname.match(/^\/api\/v1\/devices\/(.+)$/);
    if (request.method === "PATCH" && deviceMatch) {
      const body = await readJson(request);
      await runMutation(async () => {
        const key = decodeURIComponent(deviceMatch[1]);
        agent.validateDeviceWrite(key, body.path, body.value, body.revision);
        if (["fnLock", "lighting", "brightness"].includes(body.path)) {
          try {
            const result = await nativeAgent.write(body.path, body.value);
            if (result.apply?.status === "error") {
              throw new AgentError("NATIVE_WRITE_FAILED", result.apply.error || "Native hardware write failed", {
                path: body.path,
              });
            }
          } catch (error) {
            if (error instanceof AgentError) throw error;
            throw new AgentError(error.code || "NATIVE_WRITE_FAILED", error.message, error.details || {});
          }
        }
        agent.writeDevice(key, body.path, body.value, body.revision);
      });
      json(response, 200, currentSnapshot());
      return;
    }
    const commandMatch = url.pathname.match(/^\/api\/v1\/commands\/([a-z-]+)$/);
    if (request.method === "POST" && commandMatch) {
      const body = await readJson(request);
      await runMutation(async () => agent.runCommand(commandMatch[1], body));
      json(response, 200, currentSnapshot());
      return;
    }
    if (url.pathname.startsWith("/api/")) {
      json(response, 404, { error: { code: "NOT_FOUND", message: "API route not found" } });
      return;
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      json(response, 405, { error: { code: "METHOD_NOT_ALLOWED", message: "Method not allowed" } });
      return;
    }
    await serveStatic(url.pathname, response);
  } catch (error) {
    const known = error instanceof AgentError;
    const code = known ? error.code : "INTERNAL_ERROR";
    json(response, errorStatus(code), {
      error: {
        code,
        message: known ? error.message : "Unexpected agent failure",
        details: known ? error.details : {},
      },
    });
    if (!known) console.error(error);
  }
});

server.listen(port, host, () => {
  console.log(`Logi Forge agent listening on ${host}:${port}`);
  nativeAgent.start();
});

function shutdown() {
  nativeAgent.stop();
  for (const client of clients) client.end();
  server.close(() => process.exit(0));
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
