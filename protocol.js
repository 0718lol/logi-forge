export const PROTOCOL_VERSION = 1;
export const CONFIG_SCHEMA_VERSION = 1;
export const UPSTREAM_BASELINE_PROTOCOL = 29;

export const ACTIONS = [
  "None",
  "BrowserBack",
  "BrowserForward",
  "MissionControl",
  "AppExpose",
  "CaptureRegion",
  "Copy",
  "Paste",
  "Undo",
  "Redo",
  "ShowDesktop",
  "CycleDpiPresets",
  "ToggleSmartShift",
  "PrevTrack",
  "PlayPause",
  "NextTrack",
  "VolumeUp",
  "VolumeDown",
  "MuteVolume",
  "CustomShortcut: Cmd+Shift+P",
  "HoldShortcut: Ctrl+Space",
];

export class AgentError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = "AgentError";
    this.code = code;
    this.details = details;
  }
}

export function errorStatus(code) {
  if (code === "UNAUTHORIZED") return 401;
  if (code === "DEVICE_NOT_FOUND") return 404;
  if (code === "REVISION_CONFLICT") return 409;
  if (code === "DEVICE_OFFLINE") return 409;
  if (code === "NATIVE_WRITE_FAILED") return 409;
  if (code === "PAYLOAD_TOO_LARGE") return 413;
  if (code === "INTERNAL_ERROR") return 500;
  return 400;
}
