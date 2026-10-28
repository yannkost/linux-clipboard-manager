// First-run dialog.
"use strict";
const $ = (id) => document.getElementById(id);
let settings = null;
let status = { helper_active: false };

function render() {
  if (!settings) return;
  const parts = accelParts(settings.hotkey);
  $("keys").replaceChildren(settings.hotkey ? keycaps(settings.hotkey) : "");
  $("shortcut-text").textContent = settings.hotkey
    ? `Press ${parts.join("+")} anywhere to open your history.`
    : "Set a shortcut in Settings to open your history.";
  $("ext-status").replaceChildren(extensionPill(status));
  $("ext-action").hidden = status.helper_active || status.extension_installed;
  $("ext-text").textContent = status.extension_installed && !status.helper_active
    ? "Installed. Log out and back in once to start it; until then Co-Pa works without auto-paste."
    : "A small GNOME extension needs to be enabled for auto-paste and reliable clipboard capture to work.";
  $("login").replaceChildren(el("button", {
    class: "switch", type: "button", role: "switch", "aria-label": "Launch at login",
    "aria-checked": settings.autostart ? "true" : "false",
    onclick: async () => {
      try {
        settings = await invoke("save_settings", { settings: { ...settings, autostart: !settings.autostart } });
      } catch (err) {
        console.error(err);
      }
      render();
    },
  }));
}

$("enable-ext").addEventListener("click", async () => {
  try {
    status = await installExtension();
    $("ext-error").hidden = true;
  } catch (err) {
    $("ext-error").textContent = String(err);
    $("ext-error").hidden = false;
  }
  render();
});
$("start").addEventListener("click", () => invoke("finish_welcome"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.target.closest("button")) invoke("finish_welcome");
});

fillIcons();
$("icon").append(appIcon(64));
initTheme().then((r) => {
  settings = r.settings;
  status = r.status || status;
  render();
});
setInterval(async () => {
  const s = await invoke("get_status").catch(() => null);
  if (s && (s.helper_active !== status.helper_active || s.extension_installed !== status.extension_installed)) { status = s; render(); }
}, 3000);
