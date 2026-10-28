// Settings window: sidebar pages that apply each change immediately.
"use strict";

const $ = (id) => document.getElementById(id);
const root = $("settings");
const body = $("body");

const PAGES = [
  ["general", "General", "tune"],
  ["shortcut", "Shortcut", "keyboard"],
  ["history", "History", "history"],
  ["paste", "Paste behaviour", "content_paste"],
  ["about", "About", "info"],
];
const KEEP = [[0, "Forever"], [1, "1 day"], [7, "7 days"], [30, "30 days"]];
const IMAGE_SIZES_MB = [1, 5, 10, 20, 50, 100];
const MB = 1024 * 1024;

let settings = null;
let status = { helper_active: false, version: "" };
let page = "general";
// Shortcut page: "default" | "recording" | "conflict"
let shortcut = { mode: "default", pending: "", conflict: "" };
let addingApp = false;

const params = new URLSearchParams(location.search);
if (PAGES.some((p) => p[0] === params.get("page"))) page = params.get("page");

async function save(patch) {
  const next = { ...settings, ...patch };
  try {
    settings = await invoke("save_settings", { settings: next });
  } catch (err) {
    toast(root, String(err), 3000);
  }
  render();
}

// ---- building blocks ----

function switchEl(on, onToggle, label) {
  return el("button", {
    class: "switch", type: "button", role: "switch", "aria-checked": on ? "true" : "false", "aria-label": label,
    onclick: () => onToggle(!on),
  });
}

function segmented(options, current, pick, label) {
  return el("div", { class: "segmented", role: "radiogroup", "aria-label": label },
    options.map(([value, text]) => el("button", {
      type: "button", role: "radio", class: value === current ? "on" : "", "aria-checked": value === current ? "true" : "false",
      onclick: () => pick(value),
    }, text)));
}

function group(title, rows, { head, note } = {}) {
  return el("div", { class: "group" },
    head || el("div", { class: "group-title" }, title),
    el("div", { class: "boxed" }, rows),
    note && el("div", { class: "group-note" }, note));
}

function row(label, control, cls = "") {
  return el("div", { class: `row ${cls}`.trim() }, typeof label === "string" ? el("span", {}, label) : label, control);
}

function extensionControls() {
  const wrap = el("div", { class: "shortcut-controls" }, extensionPill(status));
  if (!status.helper_active && !status.extension_installed) {
    wrap.append(el("button", { class: "btn", type: "button", onclick: async () => {
      try {
        status = await installExtension();
      } catch (err) {
        toast(root, String(err), 3000);
      }
      render();
    } }, "Install"));
  }
  return wrap;
}

// ---- pages ----

function generalPage() {
  return [
    group("Startup", [
      row("Launch at login", switchEl(settings.autostart, (v) => save({ autostart: v }), "Launch at login")),
      row("Show tray icon", switchEl(settings.show_tray, (v) => save({ show_tray: v }), "Show tray icon")),
    ]),
    group("Appearance", [
      row("Theme", segmented([["system", "System"], ["light", "Light"], ["dark", "Dark"]], settings.theme, (v) => save({ theme: v }), "Theme"), "tight"),
      row("Language", el("div", { class: "muted-value" }, "English (System)")),
    ]),
  ];
}

function shortcutControls() {
  if (shortcut.mode === "recording") {
    return el("div", { class: "shortcut-controls" },
      el("div", { class: "recording" }, "Press a new shortcut…"),
      el("button", { class: "btn", type: "button", onclick: cancelRecording }, "Cancel"));
  }
  const accel = shortcut.mode === "conflict" ? shortcut.pending : settings.hotkey;
  return el("div", { class: "shortcut-controls" },
    accel ? keycaps(accel, shortcut.mode === "conflict") : el("span", { class: "muted-value" }, "Disabled"),
    el("button", { class: "btn", type: "button", onclick: startRecording }, "Change"));
}

function shortcutPage() {
  const rows = [row("Open clipboard history", shortcutControls(), "tall")];
  if (shortcut.mode === "conflict") {
    rows.push(el("div", { class: "conflict-bar" },
      icon("warning", "filled"),
      el("span", { class: "msg" }, `${accelParts(shortcut.pending).join("+")} is already used by ${shortcut.conflict}.`),
      el("button", { class: "btn", type: "button", onclick: () => applyShortcut(shortcut.pending) }, "Use anyway")));
  }
  return [
    el("div", { class: "group" },
      el("div", { class: "group-title" }, "Global shortcut"),
      el("div", { class: "boxed" }, rows),
      shortcut.mode === "recording" && el("div", { class: "group-note" }, "Press Esc to cancel or Backspace to remove the shortcut.")),
    group("GNOME extension", [
      row(el("div", {}, el("span", {}, "Co-Pa extension"), el("span", { class: "sub" }, extensionHint(status))), extensionControls(), "tall"),
    ]),
  ];
}

function startRecording() {
  shortcut = { mode: "recording", pending: "", conflict: "" };
  render();
}

function cancelRecording() {
  shortcut = { mode: "default", pending: "", conflict: "" };
  render();
}

async function applyShortcut(accel) {
  shortcut = { mode: "default", pending: "", conflict: "" };
  await save({ hotkey: accel });
}

async function onRecordKey(e) {
  if (shortcut.mode !== "recording") return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return cancelRecording();
  if (e.key === "Backspace" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return applyShortcut("");
  const accel = accelFromEvent(e);
  if (!accel) return;
  const conflict = await invoke("check_shortcut", { accel }).catch(() => null);
  if (conflict) {
    shortcut = { mode: "conflict", pending: accel, conflict };
    render();
  } else {
    applyShortcut(accel);
  }
}

function historyPage() {
  const imageMb = Math.round(settings.max_image_bytes / MB);
  const sizes = IMAGE_SIZES_MB.includes(imageMb) ? IMAGE_SIZES_MB : [...IMAGE_SIZES_MB, imageMb].sort((a, b) => a - b);
  const select = el("select", {
    "aria-label": "Maximum image size to store",
    onchange: (e) => save({ max_image_bytes: Number(e.target.value) * MB }),
  }, sizes.map((mb) => el("option", { value: mb, selected: mb === imageMb }, `${mb} MB`)));

  const stepper = el("div", { class: "stepper" },
    el("span", { class: "value" }, String(settings.max_items)),
    el("button", { class: "round-btn", type: "button", title: "Fewer", "aria-label": "Fewer items",
      onclick: () => save({ max_items: Math.max(50, Math.ceil(settings.max_items / 50) * 50 - 50) }) }, icon("remove")),
    el("button", { class: "round-btn", type: "button", title: "More", "aria-label": "More items",
      onclick: () => save({ max_items: Math.min(10000, Math.floor(settings.max_items / 50) * 50 + 50) }) }, icon("add")));

  const apps = settings.excluded_apps.map((name) =>
    row(name, el("button", { class: "remove-btn", type: "button", title: `Remove ${name}`,
      onclick: () => save({ excluded_apps: settings.excluded_apps.filter((a) => a !== name) }) }, icon("close")), "tight"));
  if (addingApp) {
    const input = el("input", {
      class: "add-input", type: "text", placeholder: "App name, e.g. Signal", "aria-label": "App name",
      onkeydown: (e) => {
        if (e.key === "Enter" && input.value.trim()) {
          addingApp = false;
          const name = input.value.trim();
          save({ excluded_apps: [...settings.excluded_apps.filter((a) => a !== name), name] });
        } else if (e.key === "Escape") {
          addingApp = false;
          render();
        }
      },
      onblur: () => { addingApp = false; render(); },
    });
    apps.push(el("div", { class: "row tight" }, input));
    setTimeout(() => input.focus());
  } else {
    apps.push(el("button", { class: "row tight add-row", type: "button", onclick: () => { addingApp = true; render(); } }, icon("add"), "Add app…"));
  }

  const needsExt = status.helper_active ? null : "Takes effect once the Co-Pa GNOME extension is enabled.";
  return [
    group("Storage", [
      row("Maximum items", stepper, "tight"),
      row("Keep history for", segmented(KEEP, settings.retention_days, (v) => save({ retention_days: v }), "Keep history for"), "tight"),
      row("Maximum image size to store", el("div", { class: "select-wrap" }, select, icon("expand_more")), "tight"),
    ]),
    group("Privacy", [
      row("Ignore passwords from password managers", switchEl(settings.ignore_passwords, (v) => save({ ignore_passwords: v }), "Ignore passwords"), "tight"),
    ]),
    group("Excluded apps", apps, {
      head: el("div", { class: "group-head" }, el("span", { class: "group-title" }, "Excluded apps"), el("span", { class: "hint" }, "Copies from these apps are not saved")),
      note: needsExt,
    }),
    el("div", {}, el("button", { class: "btn danger", type: "button", style: { height: "34px", padding: "0 16px" }, onclick: () => showDialog(true) }, "Clear history")),
  ];
}

function pastePage() {
  return [
    group("Pasting", [
      row(el("div", {}, el("span", {}, "Paste automatically after selecting"),
        !status.helper_active && el("span", { class: "sub" }, "Needs the Co-Pa GNOME extension; until then the entry is only copied.")),
        switchEl(settings.paste_after_select, (v) => save({ paste_after_select: v }), "Paste automatically")),
      row("Move pasted item to top", switchEl(settings.move_to_top, (v) => save({ move_to_top: v }), "Move pasted item to top")),
      row("Strip formatting by default", switchEl(settings.strip_formatting, (v) => save({ strip_formatting: v }), "Strip formatting by default")),
    ]),
  ];
}

function aboutPage() {
  const link = (label, which) => el("button", { class: "row tight link-row", type: "button", onclick: () => invoke("open_link", { which }) },
    el("span", {}, label), icon("open_in_new"));
  return [
    el("div", { class: "about-head" }, appIcon(96), el("div", { class: "about-name" }, "Co-Pa"), el("div", { class: "about-version" }, `Version ${status.version}`)),
    el("div", { class: "boxed" }, link("Website", "website"), link("Source code", "source"), link("Report an issue", "issues")),
  ];
}

// ---- render ----

function render() {
  if (!settings) return;
  $("pages").replaceChildren(...PAGES.map(([id, label, ic]) => el("button", {
    type: "button", role: "tab", class: id === page ? "on" : "", "aria-selected": id === page ? "true" : "false",
    onclick: () => { page = id; shortcut.mode = "default"; addingApp = false; render(); },
  }, icon(ic), el("span", {}, label))));
  $("page-title").textContent = PAGES.find((p) => p[0] === page)[1];
  const builders = { general: generalPage, shortcut: shortcutPage, history: historyPage, paste: pastePage, about: aboutPage };
  body.replaceChildren(...builders[page]());
}

function showDialog(open) {
  $("dialog").hidden = !open;
  if (open) $("dialog-cancel").focus();
}

$("dialog-cancel").addEventListener("click", () => showDialog(false));
$("dialog-confirm").addEventListener("click", async () => {
  showDialog(false);
  try {
    await invoke("clear_history");
    toast(root, "History cleared");
  } catch (err) {
    toast(root, String(err), 3000);
  }
});
$("close").addEventListener("click", () => invoke("close_window"));

document.addEventListener("keydown", (e) => {
  if (shortcut.mode === "recording") return onRecordKey(e);
  if (e.key === "Escape") {
    if (!$("dialog").hidden) showDialog(false);
    else if (!addingApp) invoke("close_window");
  }
}, true);

listen("open-page", (e) => {
  page = e.payload.page;
  render();
  if (e.payload.clear) showDialog(true);
});
listen("settings-changed", (e) => {
  settings = e.payload;
  if (shortcut.mode === "default" && !addingApp) render();
});

fillIcons();
initTheme().then((r) => {
  settings = r.settings;
  status = r.status || status;
  render();
  if (params.get("clear") === "true") showDialog(true);
});
// The extension can be enabled while the window is open.
setInterval(async () => {
  const s = await invoke("get_status").catch(() => null);
  if (s && (s.helper_active !== status.helper_active || s.extension_installed !== status.extension_installed)) {
    status = s;
    if (shortcut.mode === "default" && !addingApp) render();
  }
}, 5000);
