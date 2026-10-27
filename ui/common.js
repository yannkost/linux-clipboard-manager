// Shared helpers for the popup, settings and welcome pages.
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Material Symbols codepoints (the bundled font is subset to just these).
const ICONS = {
  search: "", expand_more: "", check: "", open_in_new: "",
  content_paste: "", search_off: "", close: "", push_pin: "",
  delete: "", content_copy: "", download: "", tune: "",
  keyboard: "", history: "", info: "", remove: "", add: "",
  warning: "", extension: "",
};

// Full-colour app icon (same drawing as src-tauri/icons/clips.svg).
const APP_ICON_SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><rect x="32" y="8" width="72" height="90" rx="10" fill="#99C1F1" transform="rotate(12 68 53)"/><rect x="28" y="10" width="72" height="90" rx="10" fill="#DEEBFB" transform="rotate(5 64 55)"/><rect x="18" y="28" width="88" height="94" rx="14" fill="#1A5FB4"/><rect x="18" y="23" width="88" height="94" rx="14" fill="#3584E4"/><rect x="29" y="36" width="66" height="72" rx="5" fill="#FFFFFF"/><rect x="39" y="51" width="46" height="5" rx="2.5" fill="#C0BFBC"/><rect x="39" y="63" width="38" height="5" rx="2.5" fill="#C0BFBC"/><rect x="39" y="75" width="42" height="5" rx="2.5" fill="#C0BFBC"/><rect x="39" y="87" width="26" height="5" rx="2.5" fill="#C0BFBC"/><rect x="44" y="16" width="36" height="18" rx="6" fill="#77767B"/><rect x="44" y="13" width="36" height="17" rx="6" fill="#DEDDDA"/><rect x="56" y="17" width="12" height="4" rx="2" fill="#9A9996"/></svg>';

/** The app icon as an <svg> element of the given size. */
function appIcon(size) {
  const wrap = document.createElement("div");
  wrap.innerHTML = APP_ICON_SVG;
  const svg = wrap.firstElementChild;
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.style.display = "block";
  return svg;
}

/** <span class="icon">…</span> for a Material Symbols icon name. */
function icon(name, extraClass = "") {
  const span = document.createElement("span");
  span.className = `icon ${extraClass}`.trim();
  span.textContent = ICONS[name] || "";
  span.setAttribute("aria-hidden", "true");
  return span;
}

/** Fill every [data-icon] placeholder in static markup. */
function fillIcons(root = document) {
  for (const el of root.querySelectorAll("[data-icon]")) {
    el.classList.add("icon");
    el.textContent = ICONS[el.dataset.icon] || "";
    el.setAttribute("aria-hidden", "true");
  }
}

/** Small element builder: el("div", {class: "x", onclick}, child, "text"). */
function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === "class") node.className = v;
    else if (k === "style" && typeof v === "object") Object.assign(node.style, v);
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

// ---- theme ----

let systemDark = false;

function applyTheme(theme) {
  const dark = theme === "dark" || (theme !== "light" && systemDark);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

async function initTheme() {
  try {
    const [settings, status] = await Promise.all([invoke("get_settings"), invoke("get_status")]);
    systemDark = status.prefers_dark;
    applyTheme(settings.theme);
    return { settings, status };
  } catch (err) {
    console.error("theme", err);
    return {};
  }
}

listen("settings-changed", (e) => applyTheme(e.payload.theme));

// ---- content types ----

const TYPE_LABEL = { text: "Text", code: "Text", link: "Link", file: "File", color: "Colour", image: "Image" };
const CODE_START = /^(\$ |sudo |ssh |git |cd |ls |cat |cargo |npm |npx |pnpm |yarn |docker |kubectl |flatpak |apt |dnf |pacman |curl |wget |python3? |pip3? |make |systemctl |journalctl |gsettings |export |#!)/;

function hexToRgb(hex) {
  let h = hex.slice(1);
  if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join("");
  const n = parseInt(h.slice(0, 6), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
}

function rgbToHex(r, g, b) {
  return "#" + [r, g, b].map((v) => Math.max(0, Math.min(255, v)).toString(16).padStart(2, "0")).join("").toUpperCase();
}

/** Guess how to display an entry: link, file path, colour, code or plain text. */
function classify(entry) {
  if (entry.kind === "image") return { type: "image" };
  const text = entry.text || "";
  const t = text.trim();
  const oneLine = !t.includes("\n");
  if (oneLine && /^https?:\/\/\S+$/i.test(t)) {
    let domain = "";
    try { domain = new URL(t).hostname.replace(/^www\./, ""); } catch { /* not a URL after all */ }
    return { type: "link", url: t, domain };
  }
  if (oneLine && t.length < 4096 && /^(\/|~\/|file:\/\/)[^\0]*$/.test(t) && !/\s{2,}/.test(t)) {
    const path = t.replace(/^file:\/\//, "");
    const trimmed = path.replace(/\/+$/, "");
    const cut = trimmed.lastIndexOf("/");
    return { type: "file", name: trimmed.slice(cut + 1) || "/", dir: trimmed.slice(0, cut + 1) };
  }
  if (oneLine && /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(t)) {
    return { type: "color", hex: t.toUpperCase(), css: t, rgb: hexToRgb(t) };
  }
  const rgb = oneLine && t.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(,\s*[\d.]+\s*)?\)$/i);
  if (rgb) {
    const hex = rgbToHex(+rgb[1], +rgb[2], +rgb[3]);
    return { type: "color", hex, css: t, rgb: `rgb(${+rgb[1]}, ${+rgb[2]}, ${+rgb[3]})` };
  }
  const lines = text.split("\n");
  const codeChars = (text.match(/[{};=<>()[\]]/g) || []).length;
  const indented = lines.filter((l) => /^( {2,}|\t)\S/.test(l)).length;
  if (
    (oneLine && CODE_START.test(t)) ||
    (lines.length > 1 && (indented >= 1 || codeChars / text.length > 0.04) && /[{};]|=>|::|\bfn\b|\bdef\b|\bfunction\b|\bconst\b|\blet\b/.test(text)) ||
    /^#\[[a-z_:]+.*\]$/m.test(lines[0])
  ) {
    return { type: "code" };
  }
  return { type: "text" };
}

// ---- time ----

/** "now", "5m", "3h", "Yesterday", "4d", "2w", "3mo", "1y" */
function relTime(ms) {
  const diff = Math.max(0, Date.now() - ms);
  const min = Math.floor(diff / 60000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  const hours = Math.floor(min / 60);
  const then = new Date(ms);
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (hours < 24 && ms >= startOfToday) return `${hours}h`;
  if (ms >= startOfToday - 86400000) return "Yesterday";
  const days = Math.floor((startOfToday - new Date(then.getFullYear(), then.getMonth(), then.getDate()).getTime()) / 86400000);
  if (days < 14) return `${days}d`;
  if (days < 30) return `${Math.floor(days / 7)}w`;
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}

function copiedAgo(ms) {
  const t = relTime(ms);
  if (t === "now") return "Copied just now";
  if (t === "Yesterday") return "Copied yesterday";
  return `Copied ${t} ago`;
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1).replace(/\.0$/, "")} MB`;
}

// ---- keyboard shortcuts (GTK accelerator syntax) ----

const MOD_LABEL = { control: "Ctrl", primary: "Ctrl", ctrl: "Ctrl", alt: "Alt", mod1: "Alt", shift: "Shift", super: "Super", mod4: "Super" };

/** "<Control><Alt>t" → ["Ctrl", "Alt", "T"] */
function accelParts(accel) {
  const parts = [];
  let rest = accel || "";
  let m;
  while ((m = rest.match(/^<([^>]+)>/))) {
    parts.push(MOD_LABEL[m[1].toLowerCase()] || m[1]);
    rest = rest.slice(m[0].length);
  }
  if (rest) parts.push(rest.length === 1 ? rest.toUpperCase() : rest[0].toUpperCase() + rest.slice(1));
  return parts;
}

const KEY_NAMES = { " ": "space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right", Enter: "Return", Escape: "Escape", Tab: "Tab", Backspace: "BackSpace", Delete: "Delete", Insert: "Insert", Home: "Home", End: "End", PageUp: "Page_Up", PageDown: "Page_Down" };

/** Build an accelerator from a keydown, or null for a lone modifier. */
function accelFromEvent(e) {
  if (["Control", "Alt", "Shift", "Meta", "Super", "OS", "AltGraph"].includes(e.key)) return null;
  let key;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3).toLowerCase();
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (/^F\d{1,2}$/.test(e.key)) key = e.key;
  else key = KEY_NAMES[e.key] || (e.key.length === 1 ? e.key.toLowerCase() : e.key);
  let mods = "";
  if (e.ctrlKey) mods += "<Control>";
  if (e.altKey) mods += "<Alt>";
  if (e.shiftKey) mods += "<Shift>";
  if (e.metaKey) mods += "<Super>";
  return mods + key;
}

/** Keycap elements for an accelerator. */
function keycaps(accel, conflict = false) {
  const wrap = el("div", { class: conflict ? "keys conflict" : "keys" });
  accelParts(accel).forEach((p, i) => {
    if (i) wrap.append(el("span", { class: "plus" }, "+"));
    wrap.append(el("span", { class: "kbd" }, p));
  });
  return wrap;
}

function toast(container, text, ms = 1400) {
  let t = container.querySelector(":scope > .toast");
  if (!t) {
    t = el("div", { class: "toast", role: "status" });
    container.append(t);
  }
  t.textContent = text;
  t.hidden = false;
  clearTimeout(t._timer);
  t._timer = setTimeout(() => { t.hidden = true; }, ms);
}
