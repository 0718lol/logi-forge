let snapshot = null;
let devices = [];
let actions = [];
let recentApps = [];
let diagnostics = null;
let currentIndex = 0;
let currentTab = "Buttons";
let toastTimer = null;

const $ = (selector) => document.querySelector(selector);
const escapeHtml = (value) => String(value)
  .replaceAll("&", "&amp;")
  .replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;")
  .replaceAll('"', "&quot;")
  .replaceAll("'", "&#039;");

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { "Content-Type": "application/json", ...(options.headers || {}) },
  });
  const payload = await response.json();
  if (!response.ok) {
    const error = new Error(payload.error?.message || "Agent request failed");
    error.code = payload.error?.code || "REQUEST_FAILED";
    throw error;
  }
  return payload;
}

function acceptSnapshot(next) {
  if (next.protocolVersion !== 1) throw new Error(`Unsupported agent protocol v${next.protocolVersion}`);
  const selectedKey = devices[currentIndex]?.key;
  snapshot = next;
  devices = next.devices;
  actions = next.actions;
  recentApps = next.recentApps;
  const selectedIndex = devices.findIndex((device) => device.key === selectedKey);
  currentIndex = selectedIndex >= 0 ? selectedIndex : 0;
  setAgentStatus("online");
  render();
}

function setAgentStatus(status) {
  const label = $("#agentLabel");
  const statusNode = $("#agentStatus");
  if (!label || !statusNode) return;
  statusNode.dataset.status = status;
  const nativeOnline = snapshot?.nativeAgent?.status === "online";
  label.textContent = status === "online"
    ? nativeOnline ? "Native agent online" : "Demo agent online"
    : status === "connecting" ? "Connecting agent" : "Agent disconnected";
}

async function writeDevice(device, path, value, successMessage = "Agent accepted write") {
  if (device.hardwareWritable === false) {
    showToast("Native device writes are not available yet", true);
    return;
  }
  try {
    const next = await api(`/api/v1/devices/${encodeURIComponent(device.key)}`, {
      method: "PATCH",
      body: JSON.stringify({ path, value, revision: snapshot.revision }),
    });
    acceptSnapshot(next);
    showToast(successMessage);
  } catch (error) {
    if (error.code === "REVISION_CONFLICT") await loadSnapshot();
    else render();
    showToast(`${error.code}: ${error.message}`, true);
  }
}

async function runCommand(name, body = {}, successMessage = "Command completed") {
  try {
    const next = await api(`/api/v1/commands/${name}`, { method: "POST", body: JSON.stringify(body) });
    acceptSnapshot(next);
    showToast(successMessage);
  } catch (error) {
    showToast(`${error.code}: ${error.message}`, true);
  }
}

async function loadSnapshot() {
  setAgentStatus("connecting");
  try {
    acceptSnapshot(await api("/api/v1/snapshot"));
  } catch (error) {
    setAgentStatus("offline");
    $("#panelMount").innerHTML = `<div class="empty-state"><strong>Agent unavailable</strong><span>${escapeHtml(error.message)}</span></div>`;
  }
}

function tabsFor(device) {
  const tabs = [];
  if (device.kind === "Camera") tabs.push("Camera");
  if (device.capabilities.includes("buttons") && device.kind === "Mouse") tabs.push("Buttons");
  if (device.kind === "Mouse") tabs.push("Actions Ring");
  if (device.kind === "Keyboard") tabs.push("Keys");
  if (device.capabilities.includes("pointer")) tabs.push("Pointer");
  if (device.capabilities.includes("lighting")) tabs.push("Lighting");
  if (device.capabilities.includes("light")) tabs.push("Light");
  tabs.push("Device");
  return tabs;
}

function render() {
  if (!snapshot || !devices.length) return;
  const device = devices[currentIndex];
  const tabs = tabsFor(device);
  if (!tabs.includes(currentTab)) currentTab = tabs[0];

  $("#deviceTitle").textContent = device.name;
  $("#deviceRoute").textContent = device.connection || device.route || "Unknown route";
  $("#managedToggle").checked = Boolean(device.managed);
  $("#managedToggle").disabled = device.hardwareWritable === false;
  $("#managedToggle").title = device.hardwareWritable === false ? "Native adapter does not expose writes yet" : "Manage device";
  $("#pairDevice").disabled = device.source === "native";
  $("#pairDevice").title = device.source === "native" ? "Native pairing is not available yet" : "Pair device";
  $("#protocol").textContent = `v${snapshot.protocolVersion}`;

  renderDevices();
  renderTabs(tabs);
  renderPanel(device);
  renderInspector(device);
}

function renderDevices() {
  const list = $("#deviceList");
  list.innerHTML = "";
  devices.forEach((device, index) => {
    const button = document.createElement("button");
    button.className = `device-card ${index === currentIndex ? "active" : ""}`;
    button.type = "button";
    button.innerHTML = `
      <span class="device-icon">${escapeHtml(device.icon)}</span>
      <span>
        <span class="device-name">${escapeHtml(device.name)}</span>
        <span class="device-meta">${device.online ? "Online" : "Offline"} - ${escapeHtml(device.kind)}</span>
      </span>
      <span class="battery">${device.battery == null ? "" : `${device.battery}%`}</span>
    `;
    button.addEventListener("click", () => {
      currentIndex = index;
      currentTab = tabsFor(device)[0];
      render();
    });
    list.appendChild(button);
  });
}

function renderTabs(tabs) {
  const container = $("#tabs");
  container.innerHTML = "";
  tabs.forEach((tab) => {
    const button = document.createElement("button");
    button.className = `tab ${tab === currentTab ? "active" : ""}`;
    button.type = "button";
    button.textContent = tab;
    button.addEventListener("click", () => {
      currentTab = tab;
      render();
    });
    container.appendChild(button);
  });
}

function renderPanel(device) {
  const mount = $("#panelMount");
  if (currentTab === "Buttons") mount.innerHTML = renderButtons(device);
  if (currentTab === "Actions Ring") mount.innerHTML = renderRing(device);
  if (currentTab === "Keys") mount.innerHTML = renderKeys(device);
  if (currentTab === "Pointer") mount.innerHTML = renderPointer(device);
  if (currentTab === "Lighting") mount.innerHTML = renderLighting(device);
  if (currentTab === "Camera") mount.innerHTML = renderCamera(device);
  if (currentTab === "Light") mount.innerHTML = renderLight(device);
  if (currentTab === "Device") mount.innerHTML = renderDeviceInfo(device);
  bindPanelEvents(device);
}

function renderButtons(device) {
  return `
    <div class="panel-title">
      <div>
        <h2>Button mappings</h2>
        <p>Per-device bindings with short, long, gesture, and hold actions.</p>
      </div>
      <button class="ghost-button" data-command="profile">Default profile</button>
    </div>
    <div class="device-stage">
      <div class="visual">
        <div class="mouse-body"></div>
        <div class="mouse-split"></div>
        <div class="mouse-wheel"></div>
        ${Object.keys(device.bindings).map((id) => `<button class="button-hotspot" data-id="${escapeHtml(id)}">${escapeHtml(id.replace("Click", ""))}</button>`).join("")}
      </div>
      <div class="control-stack">
        ${Object.entries(device.bindings).map(([button, action]) => actionRow(button, action, `binding:${button}`)).join("")}
      </div>
    </div>
  `;
}

function renderRing(device) {
  const slots = ["Top", "TopRight", "Right", "BottomRight", "Bottom", "BottomLeft", "Left", "TopLeft"];
  return `
    <div class="panel-title">
      <div>
        <h2>Actions Ring</h2>
        <p>Eight-slot radial launcher, resolved against the active application profile.</p>
      </div>
      <label class="toggle"><input type="checkbox" checked /> Haptics</label>
    </div>
    <div class="ring-grid">
      ${slots.map((slot) => `<button class="ring-slot slot-${slot}" data-ring="${slot}">${escapeHtml(device.ring[slot])}<small>${slot}</small></button>`).join("")}
    </div>
  `;
}

function renderKeys(device) {
  const keys = Object.keys(device.keys);
  return `
    <div class="panel-title">
      <div>
        <h2>Function row</h2>
        <p>Global F-key remapping with keyboard-native Fn lock.</p>
      </div>
      <label class="toggle"><input data-field="fnLock" type="checkbox" ${device.fnLock ? "checked" : ""} /> Fn lock</label>
    </div>
    <div class="keyboard-row">
      ${keys.map((key) => `<button class="keycap" data-keycap="${key}">${key}<br>${escapeHtml(device.keys[key])}</button>`).join("")}
    </div>
  `;
}

function renderPointer(device) {
  return `
    <div class="panel-title">
      <div>
        <h2>Pointer</h2>
        <p>DPI, SmartShift, native scroll inversion, and wheel resolution.</p>
      </div>
      <button class="ghost-button" data-command="dpi-cycle">Cycle DPI</button>
    </div>
    <div class="control-stack">
      ${rangeRow("DPI", "Sensor resolution", "dpi", device.dpi, 200, 8000, 50)}
      ${selectRow("SmartShift", "Wheel mode", "smartshift", device.smartshift, ["ratchet", "free_spin"])}
      ${rangeRow("SmartShift torque", "Ratchet resistance", "torque", device.torque, 0, 100, 1)}
      ${selectRow("Wheel resolution", "HID++ hires wheel setting", "scrollResolution", device.scrollResolution, ["high", "low"])}
      ${checkboxRow("Invert scroll", "Device-native vertical inversion", "invertScroll", device.invertScroll)}
    </div>
  `;
}

function renderLighting(device) {
  const colors = ["#18a06f", "#255dc7", "#d04f43", "#f0b72f", "#ffffff"];
  return `
    <div class="panel-title">
      <div>
        <h2>Keyboard lighting</h2>
        <p>Static RGB color and brightness for supported HID++ keyboards.</p>
      </div>
    </div>
    <div class="control-stack">
      <div class="setting-row">
        <div><div class="setting-name">Color</div><div class="setting-hint">Solid effect</div></div>
        <div class="swatches">${colors.map((color) => `<button class="swatch" data-color="${color}" style="background:${color}"></button>`).join("")}</div>
      </div>
      ${rangeRow("Brightness", "Lighting intensity", "brightness", device.brightness, 0, 100, 1)}
    </div>
  `;
}

function renderCamera(device) {
  const c = device.camera;
  return `
    <div class="panel-title">
      <div>
        <h2>Camera</h2>
        <p>UVC preview and image controls written to the camera hardware.</p>
      </div>
      ${selectInput("camera.profile", c.profile, ["Default", "Streaming", "Video call"])}
    </div>
    <div class="device-stage">
      <div class="camera-preview"><div class="camera-lens">LIVE</div></div>
      <div class="control-stack">
        ${rangeRow("Zoom", "Digital zoom", "camera.zoom", c.zoom, 100, 500, 5)}
        ${checkboxRow("Auto focus", "Hardware auto focus", "camera.focusAuto", c.focusAuto)}
        ${checkboxRow("Auto exposure", "Hardware auto exposure", "camera.exposureAuto", c.exposureAuto)}
        ${rangeRow("Brightness", "Image brightness", "camera.brightness", c.brightness, 0, 100, 1)}
        ${rangeRow("Contrast", "Image contrast", "camera.contrast", c.contrast, 0, 100, 1)}
        ${rangeRow("White balance", "Color temperature", "camera.whiteBalance", c.whiteBalance, 2800, 6500, 100)}
      </div>
    </div>
  `;
}

function renderLight(device) {
  const light = device.light;
  return `
    <div class="panel-title">
      <div>
        <h2>Standalone light</h2>
        <p>Litra power, brightness, temperature, and camera-follow behavior.</p>
      </div>
      <label class="toggle"><input data-field="light.power" type="checkbox" ${light.power ? "checked" : ""} /> Power</label>
    </div>
    <div class="control-stack">
      ${rangeRow("Brightness", "Light intensity", "light.brightness", light.brightness, 0, 100, 1)}
      ${rangeRow("Temperature", "Warm to cool", "light.temperature", light.temperature, 2700, 6500, 100)}
      ${checkboxRow("Auto power", "Follow camera activity", "light.autoPower", Boolean(light.autoPower))}
    </div>
  `;
}

function renderDeviceInfo(device) {
  return `
    <div class="panel-title">
      <div>
        <h2>Device</h2>
        <p>Identity, route, capability gates, and management status.</p>
      </div>
    </div>
    <div class="control-stack">
      ${textRow("Custom name", "Shown in the device list", "name", device.name)}
      ${factRow("Config key", escapeHtml(device.key))}
      ${factRow("Route", escapeHtml(device.route))}
      ${factRow("Capabilities", escapeHtml(device.capabilities.join(", ")))}
      ${factRow("Online", device.online ? "yes" : "no")}
    </div>
  `;
}

function actionRow(label, value, field) {
  return `
    <div class="setting-row">
      <div><div class="setting-name">${label}</div><div class="setting-hint">Captured physical control</div></div>
      ${selectInput(field.replace("binding:", "bindings."), value, actions)}
    </div>
  `;
}

function rangeRow(label, hint, field, value, min, max, step) {
  return `
    <div class="setting-row">
      <div><label for="${field}">${label}</label><div class="setting-hint">${hint}: <strong>${value}</strong></div></div>
      <input id="${field}" data-field="${field}" type="range" min="${min}" max="${max}" step="${step}" value="${value}" />
    </div>
  `;
}

function selectRow(label, hint, field, value, values) {
  return `
    <div class="setting-row">
      <div><label>${label}</label><div class="setting-hint">${hint}</div></div>
      ${selectInput(field, value, values)}
    </div>
  `;
}

function checkboxRow(label, hint, field, value) {
  return `
    <div class="setting-row">
      <div><label>${label}</label><div class="setting-hint">${hint}</div></div>
      <label class="toggle"><input data-field="${field}" type="checkbox" ${value ? "checked" : ""} /> Enabled</label>
    </div>
  `;
}

function textRow(label, hint, field, value) {
  return `
    <div class="setting-row">
      <div><label>${label}</label><div class="setting-hint">${hint}</div></div>
      <input data-field="${field}" type="text" value="${escapeHtml(value)}" />
    </div>
  `;
}

function factRow(label, value) {
  return `
    <div class="setting-row">
      <div><div class="setting-name">${label}</div></div>
      <strong>${value}</strong>
    </div>
  `;
}

function selectInput(field, value, values) {
  return `
    <select data-field="${field}">
      ${values.map((item) => `<option value="${escapeHtml(item)}" ${item === value ? "selected" : ""}>${escapeHtml(item)}</option>`).join("")}
    </select>
  `;
}

function bindPanelEvents(device) {
  document.querySelectorAll("[data-field]").forEach((input) => {
    input.addEventListener("change", () => {
      writeDevice(device, input.dataset.field, readInput(input));
    });
  });

  document.querySelectorAll("[data-color]").forEach((button) => {
    button.addEventListener("click", () => {
      writeDevice(device, "lighting", button.dataset.color, "Lighting color updated");
    });
  });

  document.querySelectorAll("[data-ring]").forEach((button) => {
    button.addEventListener("click", () => {
      const slot = button.dataset.ring;
      const index = actions.indexOf(device.ring[slot]);
      const action = actions[(index + 1) % actions.length];
      writeDevice(device, `ring.${slot}`, action, `${slot} set to ${action}`);
    });
  });

  document.querySelectorAll("[data-keycap]").forEach((button) => {
    button.addEventListener("click", () => {
      const key = button.dataset.keycap;
      const index = actions.indexOf(device.keys[key]);
      const action = actions[(index + 1) % actions.length];
      writeDevice(device, `keys.${key}`, action, `${key} remapped`);
    });
  });

  document.querySelectorAll("[data-command]").forEach((button) => {
    button.addEventListener("click", () => {
      if (button.dataset.command === "dpi-cycle") {
        runCommand("cycle-dpi", { deviceKey: device.key }, "DPI preset changed");
      }
      if (button.dataset.command === "profile") showToast("Profile overlay: com.microsoft.VSCode");
    });
  });
}

function readInput(input) {
  if (input.type === "checkbox") return input.checked;
  if (input.type === "range" || input.type === "number") return Number(input.value);
  return input.value;
}

function renderInspector(device) {
  const native = snapshot.nativeAgent || {};
  $("#runtimeFacts").innerHTML = `
    <dt>Mode</dt><dd>${escapeHtml(snapshot.runtime?.mode || "demo")}</dd>
    <dt>Native ready</dt><dd>${snapshot.runtime?.nativeAvailable ? "yes" : "no"}</dd>
    <dt>Protocol</dt><dd>${snapshot.protocolVersion}</dd>
    <dt>Upstream baseline</dt><dd>${snapshot.upstreamBaselineProtocol}</dd>
    <dt>Revision</dt><dd>${snapshot.revision}</dd>
    <dt>Inventory</dt><dd>${escapeHtml(snapshot.inventoryStatus)}</dd>
    <dt>Native agent</dt><dd>${escapeHtml(native.status || "unavailable")}</dd>
    <dt>Native devices</dt><dd>${native.devices?.length ?? 0}</dd>
    <dt>Config apply</dt><dd>${escapeHtml(native.apply?.status || "unavailable")}</dd>
    <dt>Foreground</dt><dd>${escapeHtml(snapshot.foregroundApp.name)}</dd>
    <dt>Route</dt><dd>${escapeHtml(device.route)}</dd>
    <dt>Battery</dt><dd>${device.battery == null ? "n/a" : `${device.battery}%`}</dd>
  `;

  $("#appProfiles").innerHTML = recentApps
    .map((app, index) => `<div class="profile-pill"><span>${escapeHtml(app.name)}</span><strong>${index === 0 ? "active" : "seen"}</strong></div>`)
    .join("");

  renderDiagnostics();

  $("#configPreview").textContent = snapshot.configPreviews?.[device.key] || "Native configuration is managed by the agent.";
}

function renderDiagnostics() {
  const target = $("#diagnosticsFacts");
  if (!target) return;
  if (!diagnostics) {
    target.innerHTML = "<dd>loading...</dd>";
    return;
  }
  const nodes = diagnostics.host?.nodes || {};
  const nodeStatus = (node) => node?.status || "unavailable";
  const uinputStatus = (nodes.uinput || []).map(nodeStatus).join(" / ");
  target.innerHTML = `
    <dt>Native</dt><dd>${escapeHtml(diagnostics.nativeAgent?.status || "unavailable")}</dd>
    <dt>Inventory</dt><dd>${escapeHtml(diagnostics.nativeAgent?.inventoryStatus || "unavailable")} (${diagnostics.nativeAgent?.deviceCount ?? 0})</dd>
    <dt>hidraw</dt><dd>${escapeHtml(nodeStatus(nodes.hidraw))} (${nodes.hidraw?.accessibleCount ?? 0}/${nodes.hidraw?.count ?? 0} readable)</dd>
    <dt>Input events</dt><dd>${escapeHtml(nodeStatus(nodes.input))} (${nodes.input?.accessibleCount ?? 0}/${nodes.input?.count ?? 0} readable)</dd>
    <dt>uinput</dt><dd>${escapeHtml(uinputStatus || "unavailable")}</dd>
    <dt>Config</dt><dd>${escapeHtml(diagnostics.nativeAgent?.configStatus || "unavailable")}</dd>
    <dt>Apply</dt><dd>${escapeHtml(diagnostics.nativeAgent?.applyStatus || "unavailable")}</dd>
  `;
}

async function loadDiagnostics() {
  try {
    diagnostics = await api("/api/v1/diagnostics");
    renderDiagnostics();
  } catch {
    diagnostics = null;
    renderDiagnostics();
  }
}

function showToast(message, isError = false) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.toggle("error", isError);
  toast.classList.add("visible");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove("visible"), 1800);
}

$("#managedToggle").addEventListener("change", (event) => {
  writeDevice(devices[currentIndex], "managed", event.target.checked, "Management state changed");
});

$("#pairDevice").addEventListener("click", () => {
  runCommand("pair", {}, "Pairing flow completed in mock agent");
});

$("#syncAssets").addEventListener("click", () => runCommand("sync-assets", {}, "Asset catalog is current"));
$("#reloadConfig").addEventListener("click", () => runCommand("reload-config", {}, "Config reloaded"));
$("#runDiagnostics").addEventListener("click", () => loadDiagnostics());

$("#copyConfig").addEventListener("click", async () => {
  const text = $("#configPreview").textContent;
  try {
    await navigator.clipboard.writeText(text);
    showToast("Config copied");
  } catch {
    showToast("Clipboard unavailable");
  }
});

const events = new EventSource("/api/v1/events");
events.addEventListener("snapshot", (event) => {
  try {
    acceptSnapshot(JSON.parse(event.data));
  } catch (error) {
    setAgentStatus("offline");
    showToast(error.message, true);
  }
});
events.onerror = () => setAgentStatus("offline");

loadSnapshot();
loadDiagnostics();
