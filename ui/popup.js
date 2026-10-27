// Main popup: search, Recent/Pinned tabs, filter menu, multi-select, preview.
"use strict";

const $ = (id) => document.getElementById(id);
const popup = $("popup");
const search = $("search");
const list = $("list");
const scroll = $("scroll");

const state = {
  query: "",
  tab: "recent",
  filter: "all",
  entries: [],
  counts: { recent: 0, pinned: 0 },
  sel: 0,
  multi: new Set(),
  preview: false,
  menu: false,
  settings: { strip_formatting: false },
};
const FILTER_LABEL = { all: "All", text: "Text", images: "Images" };
const fullCache = new Map(); // id → full entry for the preview
const imageCache = new Map(); // id → data URL

function current() {
  return state.entries[state.sel];
}

function call(cmd, args) {
  return invoke(cmd, args).catch((err) => {
    toast(popup, String(err), 2500);
    throw err;
  });
}

// ---- rendering ----

function renderRow(entry, index) {
  const info = classify(entry);
  const sel = index === state.sel;
  const multi = state.multi.has(entry.id);
  const row = el("div", {
    class: `row${sel ? " sel" : ""}${multi ? " multi" : ""}`,
    role: "option",
    "aria-selected": sel || multi ? "true" : "false",
    "data-index": index,
  });
  row.append(el("div", { class: "row-bg" }));

  const slot = el("div", { class: "badge-slot" });
  if (multi) slot.append(el("div", { class: "check" }, icon("check")));
  else if (index < 9) slot.append(el("div", { class: "badge" }, String(index + 1)));
  row.append(slot);

  const content = el("div", { class: "content" });
  const text = entry.text || "";
  switch (info.type) {
    case "code":
      for (const line of text.split("\n").filter((l) => l.trim()).slice(0, 2)) {
        content.append(el("div", { class: "c-code" }, line));
      }
      break;
    case "link":
      content.append(el("div", { class: "c-line" }, info.url), el("div", { class: "c-sub" }, info.domain));
      break;
    case "file":
      content.append(el("div", { class: "c-line" }, info.name), el("div", { class: "c-path" }, info.dir));
      break;
    case "color":
      content.append(
        el("div", { class: "c-color" },
          el("div", { class: "swatch", style: { background: info.css } }),
          el("span", { class: "c-hex" }, info.hex),
          el("span", { class: "c-sub" }, info.rgb)),
      );
      break;
    case "image": {
      const ratio = entry.width && entry.height ? entry.width / entry.height : 1;
      const width = Math.max(32, Math.min(100, Math.round(56 * ratio)));
      content.append(
        el("div", { class: "c-image" },
          el("img", { class: "thumb", src: entry.thumbnail || "", alt: "", style: { width: `${width}px` } }),
          el("div", { class: "meta" },
            el("span", { class: "c-line" }, "Image"),
            el("span", { class: "c-sub" }, `${entry.width}×${entry.height} · PNG`))),
      );
      break;
    }
    default:
      content.append(el("div", { class: "c-text" }, text.trim()));
  }
  row.append(content);

  const trail = el("div", { class: "trail" });
  if (info.type === "link") {
    trail.append(el("button", {
      class: "open-btn", type: "button", title: "Open link",
      onclick: (e) => { e.stopPropagation(); call("open_entry", { id: entry.id }); },
    }, icon("open_in_new")));
  }
  if (entry.pinned && state.tab === "recent") trail.append(icon("push_pin", "pin-mark filled"));
  trail.append(el("span", { class: "time" }, relTime(entry.last_used_at)));
  row.append(trail);

  row.addEventListener("click", (e) => {
    if (e.ctrlKey || e.metaKey) {
      toggleMulti(entry.id);
      search.focus();
    } else {
      // One click uses the entry: copy it (and paste if enabled), close the popup.
      state.multi.clear();
      state.sel = index;
      paste(e.shiftKey);
    }
  });
  return row;
}

function renderHints() {
  const hints = $("hints");
  hints.replaceChildren();
  const b = (t) => el("b", {}, t);
  if (state.multi.size) {
    hints.append(b("↵"), " Paste all · ", b("Del"), " Remove all · ", b("Esc"), " Clear selection");
    $("count-label").textContent = `${state.multi.size} selected`;
  } else {
    const alt = state.settings.strip_formatting ? "Paste with formatting" : "Paste as plain text";
    hints.append(b("↵"), " Paste · ", b("Shift+↵"), ` ${alt} · `, b("Ctrl+P"), " Pin · ", b("Del"), " Remove · ", b("Esc"), " Close");
    const n = state.entries.length;
    $("count-label").textContent = `${n} ${n === 1 ? "item" : "items"}`;
  }
}

function render() {
  if (state.sel >= state.entries.length) state.sel = Math.max(0, state.entries.length - 1);
  list.replaceChildren(...state.entries.map(renderRow));

  const total = state.counts.recent + state.counts.pinned;
  const nothingAtAll = !state.query && state.filter === "all" && total === 0;
  $("empty").hidden = !(nothingAtAll && state.tab === "recent");
  $("no-results").hidden = nothingAtAll || state.entries.length > 0 || !state.query;
  $("no-results-query").textContent = state.query;

  $("recent-count").textContent = state.counts.recent;
  $("pinned-count").textContent = state.counts.pinned;
  for (const t of document.querySelectorAll("[data-tab]")) {
    const on = t.dataset.tab === state.tab;
    t.classList.toggle("on", on);
    t.setAttribute("aria-selected", on);
  }
  $("filter-label").textContent = FILTER_LABEL[state.filter];
  renderHints();
  list.querySelector(".row.sel")?.scrollIntoView({ block: "nearest" });
  renderPreview();
}

async function renderPreview() {
  const pane = $("preview");
  const entry = current();
  const open = state.preview && !!entry;
  pane.hidden = !open;
  if (!open) return;

  const info = classify(entry);
  $("pv-title").textContent = TYPE_LABEL[info.type];
  $("pv-sub").textContent = copiedAgo(entry.last_used_at);
  const pin = $("pv-pin");
  pin.classList.toggle("pinned", entry.pinned);
  pin.title = entry.pinned ? "Unpin" : "Pin";

  const isImage = entry.kind === "image";
  $("pv-text-wrap").hidden = isImage;
  $("pv-image-wrap").hidden = !isImage;
  const id = entry.id;
  if (isImage) {
    $("pv-dims").textContent = `${entry.width}×${entry.height} · PNG`;
    $("pv-size").textContent = formatBytes(entry.size);
    const img = $("pv-img");
    img.src = imageCache.get(id) || entry.thumbnail || "";
    if (!imageCache.has(id)) {
      const url = await invoke("get_image", { id }).catch(() => null);
      if (url) imageCache.set(id, url);
      if (url && current()?.id === id) img.src = url;
    }
    return;
  }

  let full = fullCache.get(id);
  if (!full) {
    full = await invoke("get_entry", { id }).catch(() => entry);
    fullCache.set(id, full);
    if (current()?.id !== id) return;
  }
  const raw = full.text || "";
  const lines = raw.split("\n");
  const numbered = lines.length > 1;
  $("pv-lines").replaceChildren(...lines.map((t, i) =>
    el("div", { class: "ln" }, el("span", { class: "n" }, numbered ? String(i + 1) : ""), el("span", { class: "t" }, t))));
  const chars = [...raw].length;
  $("pv-count").textContent = `${chars} ${chars === 1 ? "character" : "characters"} · ${lines.length} ${lines.length === 1 ? "line" : "lines"}`;
}

// ---- data ----

async function refresh({ keep = true } = {}) {
  const keepId = keep ? current()?.id : undefined;
  let result;
  try {
    result = await invoke("list_entries", { query: state.query, tab: state.tab, filter: state.filter });
  } catch (err) {
    console.error("list_entries", err);
    return;
  }
  state.entries = result.entries;
  state.counts = { recent: result.recent_count, pinned: result.pinned_count };
  const ids = new Set(state.entries.map((e) => e.id));
  for (const id of [...state.multi]) if (!ids.has(id)) state.multi.delete(id);
  const found = state.entries.findIndex((e) => e.id === keepId);
  state.sel = found >= 0 ? found : keep ? Math.min(state.sel, state.entries.length - 1) : 0;
  render();
}

// ---- actions ----

async function paste(shift) {
  const plain = state.settings.strip_formatting ? !shift : shift;
  if (state.multi.size) {
    const ids = state.entries.filter((e) => state.multi.has(e.id)).map((e) => e.id);
    await call("paste_entries", { ids, plain });
    state.multi.clear();
    return;
  }
  const entry = current();
  if (entry) await call("paste_entry", { id: entry.id, plain });
}

async function togglePin() {
  const entry = current();
  if (!entry) return;
  await call("toggle_pin", { id: entry.id });
  await refresh();
}

async function remove() {
  if (state.multi.size) {
    await call("delete_entries", { ids: [...state.multi] });
    state.multi.clear();
  } else {
    const entry = current();
    if (!entry) return;
    await call("delete_entry", { id: entry.id });
  }
  await refresh();
}

function toggleMulti(id) {
  if (state.multi.has(id)) state.multi.delete(id);
  else state.multi.add(id);
  render();
}

function setPreview(open) {
  if (state.preview === open) return;
  state.preview = open;
  invoke("set_preview", { open }).catch(() => {});
  render();
}

function setTab(tab) {
  state.tab = tab;
  state.sel = 0;
  state.multi.clear();
  refresh({ keep: false });
}

function setMenu(open) {
  state.menu = open;
  $("filter-menu").hidden = !open;
  $("filter-chip").setAttribute("aria-expanded", open);
  for (const b of document.querySelectorAll("[data-filter]")) {
    b.setAttribute("aria-checked", b.dataset.filter === state.filter);
  }
}

function setFilter(filter) {
  state.filter = filter;
  setMenu(false);
  state.sel = 0;
  refresh({ keep: false });
  search.focus();
}

function move(delta, extend) {
  const n = state.entries.length;
  if (!n) return;
  const next = Math.max(0, Math.min(n - 1, state.sel + delta));
  if (extend) {
    state.multi.add(state.entries[state.sel].id);
    state.multi.add(state.entries[next].id);
  }
  state.sel = next;
  render();
}

// ---- events ----

document.addEventListener("keydown", (e) => {
  const k = e.key;
  const ctrl = e.ctrlKey || e.metaKey;
  const inSearch = document.activeElement === search;
  const caretAtEnd = !inSearch || search.selectionStart === search.value.length;

  if (state.menu) {
    if (k === "Escape") { e.preventDefault(); setMenu(false); }
    return;
  }
  if (ctrl && k === ",") { e.preventDefault(); invoke("open_settings_window", { page: null }); return; }

  if (k === "ArrowDown") { e.preventDefault(); move(1, e.shiftKey); }
  else if (k === "ArrowUp") { e.preventDefault(); move(-1, e.shiftKey); }
  else if (k === "PageDown") { e.preventDefault(); move(8, false); }
  else if (k === "PageUp") { e.preventDefault(); move(-8, false); }
  else if (k === "Enter") { e.preventDefault(); paste(e.shiftKey); }
  else if (ctrl && k.toLowerCase() === "p") { e.preventDefault(); togglePin(); }
  else if (ctrl && /^[1-9]$/.test(k)) {
    e.preventDefault();
    const i = Number(k) - 1;
    if (state.entries[i]) { state.sel = i; state.multi.clear(); paste(e.shiftKey); }
  }
  else if (k === "Tab") { e.preventDefault(); setTab(state.tab === "recent" ? "pinned" : "recent"); }
  else if (k === "Delete" && (!state.query || caretAtEnd)) { e.preventDefault(); remove(); }
  else if (k === "Escape") {
    e.preventDefault();
    if (state.preview) setPreview(false);
    else if (state.multi.size) { state.multi.clear(); render(); }
    else if (state.query) { search.value = state.query = ""; refresh({ keep: false }); }
    else invoke("hide_popup");
  }
  else if (k === "ArrowRight" && caretAtEnd && !state.preview) { e.preventDefault(); setPreview(true); }
  else if (k === "ArrowLeft" && state.preview && (!state.query || search.selectionStart === 0)) { e.preventDefault(); setPreview(false); }
  else if (k === " " && !state.query) { e.preventDefault(); setPreview(!state.preview); }
  else return;
});

let searchTimer;
search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    state.query = search.value;
    state.sel = 0;
    refresh({ keep: false });
  }, 60);
});

for (const t of document.querySelectorAll("[data-tab]")) {
  t.addEventListener("click", () => { setTab(t.dataset.tab); search.focus(); });
}
$("filter-chip").addEventListener("click", (e) => { e.stopPropagation(); setMenu(!state.menu); });
for (const b of document.querySelectorAll("[data-filter]")) {
  b.addEventListener("click", () => setFilter(b.dataset.filter));
}
document.addEventListener("click", (e) => {
  if (state.menu && !$("filter-menu").contains(e.target)) setMenu(false);
});

$("pv-close").addEventListener("click", () => { setPreview(false); search.focus(); });
$("pv-paste").addEventListener("click", () => paste(state.settings.strip_formatting));
$("pv-paste-plain").addEventListener("click", () => paste(!state.settings.strip_formatting));
$("pv-pin").addEventListener("click", togglePin);
$("pv-delete").addEventListener("click", remove);
$("pv-copy").addEventListener("click", async () => {
  const entry = current();
  if (!entry) return;
  await call("copy_entry", { id: entry.id });
  toast(popup, "Copied");
});
$("pv-save").addEventListener("click", async () => {
  const entry = current();
  if (!entry) return;
  const path = await call("save_image_as", { id: entry.id });
  if (path) toast(popup, `Saved to ${path.split("/").pop()}`);
});
$("pv-open").addEventListener("click", () => {
  const entry = current();
  if (entry) call("open_entry", { id: entry.id });
});

listen("popup-shown", () => {
  search.value = state.query = "";
  state.tab = "recent";
  state.filter = "all";
  state.multi.clear();
  state.preview = false;
  setMenu(false);
  refresh({ keep: false });
  search.focus();
});
listen("history-changed", () => {
  fullCache.clear();
  refresh();
});
listen("settings-changed", (e) => {
  state.settings = e.payload;
  renderHints();
});

// Keep focus in the search box so typing always filters.
document.addEventListener("mouseup", () => {
  if (!state.menu && !window.getSelection()?.toString()) search.focus();
});

fillIcons();
initTheme().then(({ settings }) => {
  if (settings) state.settings = settings;
  refresh({ keep: false });
});
search.focus();
