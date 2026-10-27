// Placeholder popup UI. The real design comes later.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const search = document.getElementById("search");
const list = document.getElementById("list");
const empty = document.getElementById("empty");

let entries = [];
let selected = 0;
let searchTimer = null;

function describeImage(e) {
  const kb = Math.max(1, Math.round(e.size / 1024));
  return `${e.width}×${e.height} · ${kb} KB`;
}

function render() {
  list.replaceChildren();
  empty.hidden = entries.length > 0;
  entries.forEach((e, i) => {
    const li = document.createElement("li");
    li.setAttribute("role", "option");
    li.dataset.index = i;

    const index = document.createElement("span");
    index.className = "index";
    index.textContent = i < 9 ? String(i + 1) : "";
    li.append(index);

    if (e.kind === "image") {
      const img = document.createElement("img");
      img.src = e.thumbnail || "";
      img.alt = "Image";
      li.append(img);
      const meta = document.createElement("span");
      meta.className = "meta";
      meta.textContent = describeImage(e);
      li.append(meta);
    } else {
      const text = document.createElement("span");
      text.className = "text";
      text.textContent = e.text;
      text.title = e.text;
      li.append(text);
    }
    if (e.pinned) {
      const pin = document.createElement("span");
      pin.className = "pin";
      pin.textContent = "📌";
      pin.title = "Pinned";
      li.append(pin);
    }
    li.addEventListener("click", () => { selected = i; paste(false); });
    li.addEventListener("mousemove", () => { if (selected !== i) { selected = i; updateSelection(); } });
    list.append(li);
  });
  updateSelection();
}

function updateSelection() {
  if (selected >= entries.length) selected = Math.max(0, entries.length - 1);
  for (const li of list.children) {
    const on = Number(li.dataset.index) === selected;
    li.classList.toggle("selected", on);
    li.setAttribute("aria-selected", on);
    if (on) li.scrollIntoView({ block: "nearest" });
  }
}

async function refresh(keepSelection = true) {
  try {
    const id = keepSelection ? entries[selected]?.id : undefined;
    entries = await invoke("list_entries", { query: search.value });
    const found = entries.findIndex((e) => e.id === id);
    selected = found >= 0 ? found : 0;
    render();
  } catch (err) {
    console.error("list_entries failed", err);
  }
}

async function paste(plain) {
  const e = entries[selected];
  if (!e) return;
  try {
    await invoke("paste_entry", { id: e.id, plain });
  } catch (err) {
    console.error("paste_entry failed", err);
  }
}

async function act(command) {
  const e = entries[selected];
  if (!e) return;
  try {
    await invoke(command, { id: e.id });
  } catch (err) {
    console.error(`${command} failed`, err);
  }
  await refresh();
}

document.addEventListener("keydown", (ev) => {
  switch (ev.key) {
    case "ArrowDown":
      selected = Math.min(selected + 1, entries.length - 1);
      updateSelection();
      break;
    case "ArrowUp":
      selected = Math.max(selected - 1, 0);
      updateSelection();
      break;
    case "PageDown":
      selected = Math.min(selected + 8, entries.length - 1);
      updateSelection();
      break;
    case "PageUp":
      selected = Math.max(selected - 8, 0);
      updateSelection();
      break;
    case "Enter":
      paste(ev.shiftKey);
      break;
    case "Delete":
      act("delete_entry");
      break;
    case "Escape":
      if (search.value) {
        search.value = "";
        refresh(false);
      } else {
        invoke("hide_popup");
      }
      break;
    default:
      if (ev.ctrlKey && ev.key.toLowerCase() === "p") {
        act("toggle_pin");
      } else if (ev.ctrlKey && /^[1-9]$/.test(ev.key)) {
        selected = Number(ev.key) - 1;
        paste(false);
      } else {
        return;
      }
  }
  ev.preventDefault();
});

search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => refresh(false), 80);
});

listen("popup-shown", () => {
  search.value = "";
  selected = 0;
  refresh(false);
  search.focus();
});
listen("history-changed", () => refresh());

refresh(false);
search.focus();
