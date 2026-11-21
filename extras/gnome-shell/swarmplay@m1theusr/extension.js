// SwarmPlay's helper in GNOME Shell. On Wayland a program can't put its window above a game or
// take focus by itself; this does it for SwarmPlay, over D-Bus (org.swarmplay.Shell on the
// session bus). SwarmPlay installs it (Settings → Playing → In-game overlay); src/overlay.rs
// is the other side.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const OVERLAY = 'SwarmPlay Overlay';

const IFACE = `<node>
  <interface name="org.swarmplay.Shell">
    <method name="Version"><arg type="s" direction="out"/></method>
    <method name="Raise">
      <arg type="s" name="title" direction="in"/>
      <arg type="b" name="focus" direction="in"/>
      <arg type="s" name="shape" direction="in"/>
      <arg type="b" direction="out"/>
    </method>
    <method name="Focus">
      <arg type="s" name="title" direction="in"/>
      <arg type="b" direction="out"/>
    </method>
    <method name="FocusPids">
      <arg type="au" name="pids" direction="in"/>
      <arg type="b" direction="out"/>
    </method>
    <method name="Screenshot">
      <arg type="s" name="path" direction="in"/>
      <arg type="b" direction="out"/>
    </method>
  </interface>
</node>`;

export default class SwarmPlayHelper extends Extension {
    enable() {
        // The last window in front that isn't the overlay: the game, when one runs.
        this._last = null;
        this._focusId = global.display.connect('notify::focus-window', () => {
            const w = global.display.focus_window;
            if (w && w.get_title() !== OVERLAY)
                this._last = w;
        });
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, '/org/swarmplay/Shell');
        this._name = Gio.bus_own_name_on_connection(Gio.DBus.session, 'org.swarmplay.Shell',
            Gio.BusNameOwnerFlags.NONE, null, null);
    }

    disable() {
        if (this._focusId)
            global.display.disconnect(this._focusId);
        this._dbus?.unexport();
        if (this._name)
            Gio.bus_unown_name(this._name);
        this._focusId = 0;
        this._dbus = null;
        this._name = 0;
        this._last = null;
    }

    _windows() {
        return global.get_window_actors().map(a => a.meta_window).filter(w => w);
    }

    _byTitle(title) {
        return this._windows().find(w => w.get_title() === title) ?? null;
    }

    // The overlay's protocol version, from the code and not metadata.json: GNOME keeps an
    // extension's metadata from when the session started, and SwarmPlay compares this to say
    // whether the running helper does what it needs (src/overlay.rs).
    Version() {
        return '4';
    }

    // The overlay above everything, on the game's screen; with `focus` it also takes the keyboard
    // and mouse, without (a notification) the game keeps them.
    //
    // `shape` is 'screen' for the panel (a window the size of the screen, raised over the game as it
    // plays: SwarmPlay sees the game through it) or 'corner' for a notification's card (its own
    // little window, top right).
    Raise(title, focus, shape) {
        const w = this._byTitle(title);
        if (!w)
            return false;
        const game = this._last;
        const mon = game && game !== w ? game.get_monitor() : w.get_monitor();
        const geom = Main.layoutManager.monitors[mon] ?? Main.layoutManager.primaryMonitor;
        if (shape === 'screen') {
            // On the game's screen, the whole of it, but not in the full-screen state: Mutter (43
            // and later) paints an opaque black backdrop behind a full-screen Wayland window,
            // which would hide the game. Raised and focused, a window the size of the screen shows
            // the game playing behind it (SwarmPlay sizes itself to the monitor).
            try {
                if (mon !== w.get_monitor())
                    w.move_to_monitor(mon);
                if (w.is_fullscreen())
                    w.unmake_fullscreen();
            } catch (e) {
                logError(e, 'SwarmPlay overlay placement');
            }
        } else {
            const r = w.get_frame_rect();
            if (w.is_fullscreen())
                w.unmake_fullscreen();
            if (geom) {
                try {
                    w.move_resize_frame(true, geom.x + geom.width - r.width, geom.y, r.width, r.height);
                } catch (e) {
                    logError(e, 'SwarmPlay overlay placement');
                }
            }
        }
        w.make_above();
        const time = global.get_current_time();
        if (focus)
            Main.activateWindow(w, time);
        else if (game && global.display.focus_window === w)
            Main.activateWindow(game, time);
        return true;
    }

    Focus(title) {
        const w = this._byTitle(title);
        if (!w)
            return false;
        Main.activateWindow(w, global.get_current_time());
        return true;
    }

    // Back to the game: a window of one of its processes, else the last one in front.
    FocusPids(pids) {
        const w = this._windows().find(x => pids.includes(x.get_pid()) && x.get_title() !== OVERLAY) ?? this._last;
        if (!w)
            return false;
        Main.activateWindow(w, global.get_current_time());
        return true;
    }

    async ScreenshotAsync([path], invocation) {
        try {
            const file = Gio.File.new_for_path(path);
            const stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
            const shot = new Shell.Screenshot();
            await shot.screenshot(false, stream);
            stream.close(null);
            invocation.return_value(new GLib.Variant('(b)', [true]));
        } catch (e) {
            logError(e, 'SwarmPlay screenshot');
            invocation.return_value(new GLib.Variant('(b)', [false]));
        }
    }
}
