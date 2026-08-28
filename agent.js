import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import {
  ACTIONS,
  AgentError,
  CONFIG_SCHEMA_VERSION,
  PROTOCOL_VERSION,
  UPSTREAM_BASELINE_PROTOCOL,
} from "./protocol.js";

const DEFAULT_DEVICES = [
  {
    key: "unit:6be9d300",
    route: "receiver:mockbolt:slot:1",
    name: "MX Master 3S",
    kind: "Mouse",
    icon: "M",
    online: true,
    battery: 82,
    connection: "Bolt receiver - slot 1",
    capabilities: ["buttons", "pointer", "thumbwheel", "hires_wheel", "smartshift"],
    dpi: 1600,
    dpiPresets: [800, 1600, 2400, 3200],
    smartshift: "ratchet",
    torque: 50,
    invertScroll: false,
    scrollResolution: "high",
    managed: true,
    bindings: {
      Back: "BrowserBack",
      Forward: "BrowserForward",
      MiddleClick: "HoldShortcut: Ctrl+Space",
      DpiToggle: "CycleDpiPresets",
    },
    ring: {
      Top: "Copy",
      TopRight: "Paste",
      Right: "BrowserForward",
      BottomRight: "VolumeUp",
      Bottom: "ShowDesktop",
      BottomLeft: "VolumeDown",
      Left: "BrowserBack",
      TopLeft: "MissionControl",
    },
  },
  {
    key: "unit:keyboard77",
    route: "receiver:mockbolt:slot:2",
    name: "MX Keys S",
    kind: "Keyboard",
    icon: "K",
    online: true,
    battery: 67,
    connection: "Bolt receiver - slot 2",
    capabilities: ["buttons", "lighting", "fn_lock"],
    managed: true,
    lighting: "#18a06f",
    brightness: 72,
    fnLock: true,
    keys: {
      F1: "MissionControl",
      F2: "AppExpose",
      F3: "ShowDesktop",
      F4: "CaptureRegion",
      F5: "None",
      F6: "None",
      F7: "PrevTrack",
      F8: "PlayPause",
      F9: "NextTrack",
      F10: "MuteVolume",
      F11: "VolumeDown",
      F12: "VolumeUp",
    },
  },
  {
    key: "camera:brio:mock-01",
    route: "uvc:046d:085e:mock-01",
    name: "Brio 4K",
    kind: "Camera",
    icon: "C",
    online: true,
    battery: null,
    connection: "USB camera",
    capabilities: ["camera"],
    managed: true,
    camera: {
      profile: "Video call",
      zoom: 115,
      focusAuto: true,
      exposureAuto: true,
      brightness: 55,
      contrast: 48,
      saturation: 52,
      whiteBalance: 4600,
    },
  },
  {
    key: "light:litra:mock-01",
    route: "rawhid:046d:c900:mock-01",
    name: "Litra Glow",
    kind: "Light",
    icon: "L",
    online: false,
    battery: null,
    connection: "Standalone light",
    capabilities: ["light"],
    managed: true,
    light: {
      power: true,
      brightness: 64,
      temperature: 4200,
      autoPower: false,
    },
  },
];

const RECENT_APPS = [
  { id: "com.microsoft.VSCode", name: "Visual Studio Code" },
  { id: "com.apple.Safari", name: "Safari" },
  { id: "exe:sharex.exe", name: "ShareX" },
  { id: "org.gnome.Terminal", name: "Terminal" },
];

const RULES = {
  managed: { type: "boolean" },
  name: { type: "string", minLength: 1, maxLength: 80 },
  dpi: { type: "number", min: 200, max: 8000, capability: "pointer" },
  smartshift: { type: "enum", values: ["ratchet", "free_spin"], capability: "smartshift" },
  torque: { type: "number", min: 0, max: 100, capability: "smartshift" },
  invertScroll: { type: "boolean", capability: "pointer" },
  scrollResolution: { type: "enum", values: ["high", "low"], capability: "hires_wheel" },
  fnLock: { type: "boolean", capability: "fn_lock" },
  lighting: { type: "color", capability: "lighting" },
  brightness: { type: "number", min: 0, max: 100, capability: "lighting" },
  "camera.profile": { type: "enum", values: ["Default", "Streaming", "Video call"], capability: "camera" },
  "camera.zoom": { type: "number", min: 100, max: 500, capability: "camera" },
  "camera.focusAuto": { type: "boolean", capability: "camera" },
  "camera.exposureAuto": { type: "boolean", capability: "camera" },
  "camera.brightness": { type: "number", min: 0, max: 100, capability: "camera" },
  "camera.contrast": { type: "number", min: 0, max: 100, capability: "camera" },
  "camera.whiteBalance": { type: "number", min: 2800, max: 6500, capability: "camera" },
  "light.power": { type: "boolean", capability: "light" },
  "light.brightness": { type: "number", min: 0, max: 100, capability: "light" },
  "light.temperature": { type: "number", min: 2700, max: 6500, capability: "light" },
  "light.autoPower": { type: "boolean", capability: "light" },
};

const clone = (value) => structuredClone(value);

function ruleFor(path) {
  if (path.startsWith("bindings.")) return { type: "action", capability: "buttons" };
  if (path.startsWith("ring.")) return { type: "action", capability: "buttons" };
  if (path.startsWith("keys.")) return { type: "action", capability: "buttons" };
  return RULES[path];
}

function assertValue(path, value, rule) {
  let valid = true;
  if (rule.type === "boolean") valid = typeof value === "boolean";
  if (rule.type === "number") valid = typeof value === "number" && Number.isFinite(value) && value >= rule.min && value <= rule.max;
  if (rule.type === "string") valid = typeof value === "string" && value.trim().length >= rule.minLength && value.length <= rule.maxLength;
  if (rule.type === "enum") valid = rule.values.includes(value);
  if (rule.type === "action") valid = ACTIONS.includes(value);
  if (rule.type === "color") valid = typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
  if (!valid) {
    throw new AgentError("INVALID_VALUE", `Invalid value for ${path}`, { path, value });
  }
}

function setPath(target, path, value) {
  const parts = path.split(".");
  const leaf = parts.pop();
  let cursor = target;
  for (const part of parts) {
    if (!cursor[part] || typeof cursor[part] !== "object") {
      throw new AgentError("UNSUPPORTED_FEATURE", `Field ${path} is not available`, { path });
    }
    cursor = cursor[part];
  }
  if (!(leaf in cursor)) {
    throw new AgentError("UNSUPPORTED_FEATURE", `Field ${path} is not available`, { path });
  }
  cursor[leaf] = typeof value === "string" ? value.trim() : value;
}

function tomlString(value) {
  return `"${String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("\n", "\\n")}"`;
}

function formatAction(action) {
  if (action.startsWith("CustomShortcut: ")) return `{ CustomShortcut = ${tomlString(action.slice(16))} }`;
  if (action.startsWith("HoldShortcut: ")) return `{ HoldShortcut = ${tomlString(action.slice(14))} }`;
  return tomlString(action);
}

export function renderConfig(device) {
  const lines = [
    `schema_version = ${CONFIG_SCHEMA_VERSION}`,
    `selected_device = ${tomlString(device.key)}`,
    "",
    "[app_settings]",
    "show_in_menu_bar = true",
    "capture_mouse_events = true",
    'device_view_mode = "grid"',
    "",
    `[devices.${tomlString(device.key)}]`,
    `custom_name = ${tomlString(device.name)}`,
    `enabled = ${device.managed}`,
  ];

  if (device.dpi) {
    lines.push(`dpi = ${device.dpi}`);
    lines.push(`dpi_presets = [${device.dpiPresets.join(", ")}]`);
    lines.push(`invert_scroll = ${device.invertScroll}`);
    lines.push(`scroll_resolution = ${tomlString(device.scrollResolution)}`);
  }
  if (device.bindings) {
    lines.push("", `[devices.${tomlString(device.key)}.bindings]`);
    for (const [button, action] of Object.entries(device.bindings)) lines.push(`${button} = ${formatAction(action)}`);
  }
  if (device.ring) {
    lines.push("", `[devices.${tomlString(device.key)}.action_ring.default.slots]`);
    for (const [slot, action] of Object.entries(device.ring)) {
      lines.push(`${slot} = { action = ${formatAction(action)}, label = ${tomlString(slot)} }`);
    }
  }
  if (device.keys) {
    lines.push("", `[devices.${tomlString(device.key)}.keyboard]`);
    lines.push(`fn_lock = ${device.fnLock}`);
    if (device.lighting) {
      lines.push("", `[devices.${tomlString(device.key)}.keyboard.lighting]`);
      lines.push(`color = ${tomlString(device.lighting.replace(/^#/, ""))}`);
      lines.push(`brightness = ${device.brightness}`);
    }
    lines.push("", `[devices.${tomlString(device.key)}.keyboard.bindings]`);
    for (const [key, action] of Object.entries(device.keys)) lines.push(`${key} = ${formatAction(action)}`);
  }
  return lines.join("\n");
}

export class MockAgent extends EventEmitter {
  constructor({ stateFile = null } = {}) {
    super();
    this.stateFile = stateFile;
    this.revision = 0;
    this.devices = clone(DEFAULT_DEVICES);
    this.load();
  }

  load() {
    if (!this.stateFile || !existsSync(this.stateFile)) return;
    try {
      const saved = JSON.parse(readFileSync(this.stateFile, "utf8"));
      if (Array.isArray(saved.devices) && Number.isInteger(saved.revision)) {
        this.devices = saved.devices;
        this.revision = saved.revision;
      }
    } catch (error) {
      throw new AgentError("CONFIG_LOAD_FAILED", "Persisted mock state is invalid", { cause: error.message });
    }
  }

  persist() {
    if (!this.stateFile) return;
    mkdirSync(dirname(this.stateFile), { recursive: true });
    const temporary = `${this.stateFile}.${process.pid}.tmp`;
    writeFileSync(temporary, JSON.stringify({ revision: this.revision, devices: this.devices }, null, 2));
    renameSync(temporary, this.stateFile);
  }

  snapshot() {
    const configPreviews = Object.fromEntries(this.devices.map((device) => [device.key, renderConfig(device)]));
    return {
      protocolVersion: PROTOCOL_VERSION,
      configSchemaVersion: CONFIG_SCHEMA_VERSION,
      upstreamBaselineProtocol: UPSTREAM_BASELINE_PROTOCOL,
      revision: this.revision,
      inventoryStatus: "ready",
      transport: "http+sse",
      foregroundApp: RECENT_APPS[0],
      recentApps: clone(RECENT_APPS),
      actions: [...ACTIONS],
      devices: clone(this.devices),
      configPreviews,
    };
  }

  commit() {
    this.revision += 1;
    this.persist();
    const snapshot = this.snapshot();
    this.emit("snapshot", snapshot);
    return snapshot;
  }

  findDevice(key) {
    const device = this.devices.find((candidate) => candidate.key === key);
    if (!device) throw new AgentError("DEVICE_NOT_FOUND", `Unknown device: ${key}`, { key });
    return device;
  }

  writeDevice(key, path, value, expectedRevision) {
    const device = this.validateDeviceWrite(key, path, value, expectedRevision);
    setPath(device, path, value);
    return this.commit();
  }

  validateDeviceWrite(key, path, value, expectedRevision) {
    if (expectedRevision !== undefined && expectedRevision !== this.revision) {
      throw new AgentError("REVISION_CONFLICT", "Agent state changed; refresh and retry", {
        expectedRevision,
        actualRevision: this.revision,
      });
    }
    const device = this.findDevice(key);
    const rule = ruleFor(path);
    if (!rule) throw new AgentError("UNSUPPORTED_FEATURE", `Field ${path} is not writable`, { path });
    if (rule.capability && !device.capabilities.includes(rule.capability)) {
      throw new AgentError("UNSUPPORTED_FEATURE", `${device.name} does not support ${rule.capability}`, { path, capability: rule.capability });
    }
    if (!device.online && path !== "managed" && path !== "name") {
      throw new AgentError("DEVICE_OFFLINE", `${device.name} is offline`, { key });
    }
    assertValue(path, value, rule);
    return device;
  }

  runCommand(name, { deviceKey } = {}) {
    if (name === "pair") {
      const device = deviceKey ? this.findDevice(deviceKey) : this.devices.find((candidate) => !candidate.online);
      if (!device) throw new AgentError("PAIRING_NOT_REQUIRED", "No offline device is available to pair");
      device.online = true;
      return this.commit();
    }
    if (name === "cycle-dpi") {
      const device = this.findDevice(deviceKey);
      if (!device.online) throw new AgentError("DEVICE_OFFLINE", `${device.name} is offline`, { deviceKey });
      if (!device.capabilities.includes("pointer")) throw new AgentError("UNSUPPORTED_FEATURE", `${device.name} has no pointer sensor`);
      const index = device.dpiPresets.indexOf(device.dpi);
      device.dpi = device.dpiPresets[(index + 1) % device.dpiPresets.length];
      return this.commit();
    }
    if (name === "reload-config") {
      this.load();
      const snapshot = this.snapshot();
      this.emit("snapshot", snapshot);
      return snapshot;
    }
    if (name === "sync-assets") return this.snapshot();
    throw new AgentError("UNKNOWN_COMMAND", `Unknown command: ${name}`, { name });
  }
}
