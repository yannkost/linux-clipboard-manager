// Co-Pa helper: the parts of a clipboard manager that Wayland only allows
// inside GNOME Shell.
//
// - Reports every clipboard change to the Co-Pa app (AddTextFrom /
//   AddImageFrom on io.github.yannkost.ClipboardManager), with the app it was
//   copied from and whether a password manager marked it as secret.
// - Exports io.github.yannkost.ClipboardManager.Shell on GNOME Shell's bus
//   connection so Co-Pa can set the clipboard and paste into the window that
//   was focused before its popup opened.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const VERSION = 1;

const SHELL_PATH = '/io/github/yannkost/ClipboardManager/Shell';
const SHELL_IFACE = `
<node>
  <interface name="io.github.yannkost.ClipboardManager.Shell">
    <method name="SetText"><arg type="s" direction="in" name="text"/></method>
    <method name="SetImage"><arg type="ay" direction="in" name="png"/></method>
    <method name="Paste"><arg type="b" direction="in" name="plain"/></method>
    <method name="Version"><arg type="u" direction="out" name="version"/></method>
  </interface>
</node>`;

const APP_BUS = 'io.github.yannkost.ClipboardManager';
const APP_PATH = '/io/github/yannkost/ClipboardManager';
const APP_IFACE = 'io.github.yannkost.ClipboardManager';

const TEXT_TYPES = ['text/plain;charset=utf-8', 'UTF8_STRING', 'text/plain', 'STRING'];
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/bmp', 'image/gif', 'image/webp'];
// Set by KeePassXC, KDE and others on copied passwords.
const PASSWORD_HINT = 'x-kde-passwordManagerHint';
const MAX_IMAGE_BYTES = 64 * 1024 * 1024;
// Terminals paste with Ctrl+Shift+V; Ctrl+V would type ^V.
const TERMINAL_RE = /terminal|kgx|ptyxis|console|kitty|alacritty|tilix|wezterm|konsole|xterm|foot|blackbox|terminator|guake|tilda/i;
// Content we set ourselves shows up as a clipboard change; ignore it for this long.
const OWN_CHANGE_MS = 1500;

export default class CoPaHelperExtension extends Extension {
    enable() {
        this._suppressUntil = 0;
        this._lastWindow = null;
        this._clipsPid = 0;
        // Created up front: a brand-new virtual device drops the first events.
        const seat = Clutter.get_default_backend().get_default_seat();
        this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);

        this._dbus = Gio.DBusExportedObject.wrapJSObject(SHELL_IFACE, {
            SetText: text => this._setText(text),
            SetImage: png => this._setImage(png),
            Paste: plain => this._paste(plain),
            Version: () => VERSION,
        });
        this._dbus.export(Gio.DBus.session, SHELL_PATH);

        this._selection = global.display.get_selection();
        this._ownerChangedId = this._selection.connect('owner-changed', (_sel, type) => {
            if (type === Meta.SelectionType.SELECTION_CLIPBOARD)
                this._onClipboardChanged();
        });

        this._focusId = global.display.connect('notify::focus-window', () => {
            const win = global.display.focus_window;
            if (win && !this._isCoPaWindow(win))
                this._lastWindow = win;
        });

        // Learn the Co-Pa process id so its own windows are never paste
        // targets and copies made inside Co-Pa are not recorded again.
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, APP_BUS, Gio.BusNameWatcherFlags.NONE,
            (conn, _name, owner) => this._lookupPid(conn, owner),
            () => {
                this._clipsPid = 0;
            });
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
        if (this._ownerChangedId)
            this._selection.disconnect(this._ownerChangedId);
        this._ownerChangedId = 0;
        this._selection = null;
        if (this._focusId)
            global.display.disconnect(this._focusId);
        this._focusId = 0;
        if (this._watchId)
            Gio.bus_unwatch_name(this._watchId);
        this._watchId = 0;
        if (this._pasteTimeoutId)
            GLib.source_remove(this._pasteTimeoutId);
        this._pasteTimeoutId = 0;
        this._keyboard?.run_dispose();
        this._keyboard = null;
        this._lastWindow = null;
    }

    _lookupPid(conn, owner) {
        conn.call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [owner]), new GLib.VariantType('(u)'),
            Gio.DBusCallFlags.NONE, -1, null, (c, res) => {
                try {
                    [this._clipsPid] = c.call_finish(res).deepUnpack();
                } catch (e) {
                    this._clipsPid = 0;
                }
            });
    }

    _isCoPaWindow(win) {
        if (this._clipsPid && win.get_pid() === this._clipsPid)
            return true;
        return /linux-clipboard-manager/i.test(win.get_wm_class() ?? '');
    }

    // ---- recording ----

    _onClipboardChanged() {
        if (Date.now() < this._suppressUntil)
            return;
        const focus = global.display.focus_window;
        if (focus && this._isCoPaWindow(focus))
            return;

        const types = this._selection.get_mimetypes(Meta.SelectionType.SELECTION_CLIPBOARD);
        const password = types.includes(PASSWORD_HINT);
        const app = Shell.WindowTracker.get_default().focus_app;
        const appName = app?.get_name() ?? '';
        const appId = app?.get_id() ?? '';
        const clipboard = St.Clipboard.get_default();

        if (TEXT_TYPES.some(t => types.includes(t))) {
            clipboard.get_text(St.ClipboardType.CLIPBOARD, (_cb, text) => {
                if (text)
                    this._callApp('AddTextFrom', new GLib.Variant('(sssb)', [text, appName, appId, password]));
            });
            return;
        }
        const mime = IMAGE_TYPES.find(t => types.includes(t));
        if (!mime || password)
            return;
        clipboard.get_content(St.ClipboardType.CLIPBOARD, mime, (_cb, bytes) => {
            if (!bytes || bytes.get_size() === 0 || bytes.get_size() > MAX_IMAGE_BYTES)
                return;
            const args = GLib.Variant.new_tuple([
                GLib.Variant.new_from_bytes(new GLib.VariantType('ay'), bytes, true),
                GLib.Variant.new_string(appName),
                GLib.Variant.new_string(appId),
            ]);
            this._callApp('AddImageFrom', args);
        });
    }

    _callApp(method, args) {
        // NO_AUTO_START: if Co-Pa is not running, the copy is simply not recorded.
        Gio.DBus.session.call(APP_BUS, APP_PATH, APP_IFACE, method, args, null,
            Gio.DBusCallFlags.NO_AUTO_START, 10000, null, (conn, res) => {
                try {
                    conn.call_finish(res);
                } catch (e) {
                    // Co-Pa not running, or it rejected the content (too big, excluded…).
                }
            });
    }

    // ---- methods called by Co-Pa ----

    _setText(text) {
        this._suppressUntil = Date.now() + OWN_CHANGE_MS;
        St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, text);
    }

    _setImage(png) {
        this._suppressUntil = Date.now() + OWN_CHANGE_MS;
        St.Clipboard.get_default().set_content(St.ClipboardType.CLIPBOARD, 'image/png', new GLib.Bytes(png));
    }

    _paste(_plain) {
        // Co-Pa puts plain text on the clipboard either way, so `plain` needs
        // no special keystroke; terminals always need Shift.
        // A window that has since closed has no actor any more.
        const target = this._lastWindow?.get_compositor_private() ? this._lastWindow : null;
        if (target && global.display.focus_window !== target && !target.minimized)
            target.activate(global.get_current_time());

        // Wait (up to 600 ms) for the target to have focus, give it a moment
        // to process the focus change, then press the keys.
        let waited = 0;
        let settled = false;
        if (this._pasteTimeoutId)
            GLib.source_remove(this._pasteTimeoutId);
        this._pasteTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 30, () => {
            waited += 30;
            const focus = global.display.focus_window;
            const ready = focus && !this._isCoPaWindow(focus) && (!target || focus === target);
            if (!ready && waited < 600)
                return GLib.SOURCE_CONTINUE;
            if (ready && !settled) {
                settled = true;
                return GLib.SOURCE_CONTINUE;
            }
            this._pasteTimeoutId = 0;
            if (focus && !this._isCoPaWindow(focus))
                this._sendPasteKeys(TERMINAL_RE.test(focus.get_wm_class() ?? ''));
            return GLib.SOURCE_REMOVE;
        });
    }

    _sendPasteKeys(shift) {
        const kb = this._keyboard;
        let time = GLib.get_monotonic_time();
        const key = (keyval, state) => kb.notify_keyval(time++, keyval, state);
        key(Clutter.KEY_Control_L, Clutter.KeyState.PRESSED);
        if (shift)
            key(Clutter.KEY_Shift_L, Clutter.KeyState.PRESSED);
        key(Clutter.KEY_v, Clutter.KeyState.PRESSED);
        key(Clutter.KEY_v, Clutter.KeyState.RELEASED);
        if (shift)
            key(Clutter.KEY_Shift_L, Clutter.KeyState.RELEASED);
        key(Clutter.KEY_Control_L, Clutter.KeyState.RELEASED);
    }
}
