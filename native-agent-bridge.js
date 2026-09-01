import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { connect } from "node:net";
import { join } from "node:path";
import { spawn } from "node:child_process";

export class NativeAgentBridge {
  constructor({ root, onSnapshot = () => {} } = {}) {
    this.root = root;
    this.onSnapshot = onSnapshot;
    this.socketPath = process.env.LOGI_FORGE_AGENT_SOCKET || join(root, ".runtime", `native-agent-${process.pid}.sock`);
    this.configPath = process.env.LOGI_FORGE_CONFIG || join(root, ".runtime", "native-config.toml");
    this.status = process.env.LOGI_FORGE_NATIVE_AGENT === "off" ? "disabled" : "starting";
    this.snapshot = null;
    this.child = null;
    this.pollTimer = null;
    this.restartTimer = null;
    this.stopped = false;
  }

  start() {
    if (this.status === "disabled" || this.stopped || this.child) return;
    this.status = "starting";
    mkdirSync(join(this.root, ".runtime"), { recursive: true });
    if (!existsSync(this.configPath)) {
      copyFileSync(join(this.root, "native", "examples", "logi-forge.toml"), this.configPath);
    }
    const configured = process.env.LOGI_FORGE_NATIVE_AGENT;
    const binary = configured || join(this.root, "native", "target", "debug", "logi-forge-agent");
    const useCargo = !configured && !existsSync(binary);
    const command = useCargo ? "cargo" : binary;
    const args = useCargo
      ? ["run", "--offline", "-p", "logi-forge-agent", "--manifest-path", join(this.root, "native", "Cargo.toml")]
      : [];
    this.child = spawn(command, args, {
      cwd: join(this.root, "native"),
      env: {
        ...process.env,
        LOGI_FORGE_AGENT_SOCKET: this.socketPath,
        LOGI_FORGE_CONFIG: this.configPath,
      },
      stdio: ["ignore", "ignore", "pipe"],
    });
    this.child.stderr.on("data", (chunk) => process.stderr.write(`[native-agent] ${chunk}`));
    this.child.once("error", (error) => {
      this.child = null;
      if (this.pollTimer) clearInterval(this.pollTimer);
      this.pollTimer = null;
      this.markOffline(error.message);
      this.scheduleRestart();
    });
    this.child.once("exit", (code, signal) => {
      this.child = null;
      if (this.pollTimer) clearInterval(this.pollTimer);
      this.pollTimer = null;
      if (this.status !== "stopped") {
        this.markOffline(`exited code=${code} signal=${signal}`);
        this.scheduleRestart();
      }
    });
    this.poll();
    this.pollTimer = setInterval(() => this.poll(), 1000);
  }

  scheduleRestart() {
    if (this.stopped || this.status === "disabled" || this.restartTimer) return;
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      this.start();
    }, 1000);
  }

  async poll() {
    try {
      const next = await this.request("snapshot");
      const changed = JSON.stringify(next) !== JSON.stringify(this.snapshot);
      this.snapshot = next;
      this.status = "online";
      if (changed) this.onSnapshot(next);
    } catch (error) {
      if (this.status === "online") this.markOffline(error.message);
    }
  }

  request(method, payload = {}) {
    return new Promise((resolve, reject) => {
      const socket = connect(this.socketPath);
      let body = "";
      const timeout = setTimeout(() => socket.destroy(new Error("native agent timed out")), 800);
      socket.setEncoding("utf8");
      socket.once("connect", () => socket.write(`${JSON.stringify({ method, ...payload })}\n`));
      socket.on("data", (chunk) => {
        body += chunk;
        const newline = body.indexOf("\n");
        if (newline < 0) return;
        clearTimeout(timeout);
        socket.end();
        try {
          resolve(JSON.parse(body.slice(0, newline)));
        } catch (error) {
          reject(error);
        }
      });
      socket.once("error", (error) => {
        clearTimeout(timeout);
        reject(error);
      });
    });
  }

  async write(path, value) {
    if (this.status !== "online" || !this.snapshot) {
      return { status: "unavailable" };
    }
    const result = await this.request("write", { path, value, revision: this.snapshot.revision });
    if (result.error) {
      const error = new Error(result.error.message);
      error.code = result.error.code;
      error.details = result.error.details;
      throw error;
    }
    if (result.snapshot) {
      this.snapshot = result.snapshot;
      this.status = "online";
      this.onSnapshot(result.snapshot);
    }
    return result;
  }

  markOffline(error) {
    this.status = "offline";
    this.onSnapshot(this.view(error));
  }

  view(error = null) {
    if (this.snapshot && this.status === "online") return this.snapshot;
    return {
      status: this.status,
      protocolVersion: 1,
      inventoryStatus: this.status,
      transport: "native-unix",
      devices: [],
      config: { status: "unavailable" },
      apply: { status: "unavailable" },
      ...(error ? { error } : {}),
    };
  }

  stop() {
    this.stopped = true;
    this.status = "stopped";
    if (this.pollTimer) clearInterval(this.pollTimer);
    if (this.restartTimer) clearTimeout(this.restartTimer);
    this.pollTimer = null;
    this.restartTimer = null;
    if (this.child && this.child.exitCode === null) this.child.kill("SIGTERM");
    this.child = null;
  }
}
