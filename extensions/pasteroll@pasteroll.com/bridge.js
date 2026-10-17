import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const NAME = 'com.pasteroll.Gnome';
const PATH = '/com/pasteroll/Gnome';
const IFACE = 'com.pasteroll.Gnome.Bridge1';
const MAX_BYTES = 64 * 1024 * 1024;
const MAX_FRAME = 16 * 1024 * 1024;
const TYPES = ['text/plain;charset=utf-8', 'text/plain', 'UTF8_STRING', 'image/png',
    'text/uri-list', 'x-special/gnome-copied-files'];
const XML = `<node><interface name="${IFACE}">
 <method name="Handshake"><arg type="u" direction="in"/>
  <arg type="u" direction="out"/><arg type="u" direction="out"/><arg type="t" direction="out"/>
  <arg type="u" direction="out"/><arg type="u" direction="out"/></method>
 <method name="Snapshot"><arg type="t" direction="out"/><arg type="as" direction="out"/>
  <arg type="s" direction="out"/><arg type="b" direction="out"/></method>
 <method name="Read"><arg type="t" direction="in"/><arg type="s" direction="in"/>
  <arg type="u" direction="in"/><arg type="ay" direction="out"/></method>
 <method name="Write"><arg type="s" direction="in"/><arg type="ay" direction="in"/><arg type="t" direction="out"/></method>
 <method name="Show"><arg type="i" direction="in"/><arg type="i" direction="in"/>
  <arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="in"/>
  <arg type="t" direction="out"/><arg type="i" direction="out"/><arg type="i" direction="out"/><arg type="d" direction="out"/></method>
 <method name="Present"><arg type="t" direction="in"/><arg type="u" direction="in"/>
  <arg type="u" direction="in"/><arg type="ay" direction="in"/></method>
 <method name="Poll"><arg type="a(utiiu)" direction="out"/></method>
 <method name="Hide"><arg type="t" direction="in"/></method>
 <method name="ArmKeyboard"><arg type="b" direction="in"/></method>
</interface></node>`;

function fail(invocation, name) {
    invocation.return_dbus_error(`${IFACE}.${name}`, `Pasteroll bridge request rejected: ${name}`);
}

function canonical(path) {
    for (let depth = 0; depth < 12; depth++) {
        if (!GLib.file_test(path, GLib.FileTest.IS_SYMLINK))
            return GLib.canonicalize_filename(path, null);
        const target = GLib.file_read_link(path);
        path = GLib.canonicalize_filename(target, GLib.path_get_dirname(path));
    }
    return null;
}

export class Bridge {
    constructor() {
        if (!Meta.is_wayland_compositor())
            throw new Error('Pasteroll GNOME bridge requires Wayland');
        this._alive = true;
        this._peer = null;
        this._epoch = GLib.get_monotonic_time();
        this._generation = this._epoch;
        this._sequence = 1;
        this._events = [];
        this._reads = new Set();
        this._actor = null;
        this._grab = null;
        this._accelerator = 0;
        this._clipboardOwner = null;
        this._excluded = false;
        this._sourceApp = '';
        this._selection = global.display.get_selection();
        this._selectionId = this._selection.connect('owner-changed', (_, type, owner) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
                return;
            this._sequence++;
            this._excluded = owner === this._clipboardOwner;
            this._sourceApp = '';
            const pid = global.display.focus_window?.get_pid();
            if (pid > 0) {
                try { this._sourceApp = GLib.path_get_basename(GLib.file_read_link(`/proc/${pid}/exe`)).toLowerCase(); }
                catch (_) { /* Unknown source stays unknown. */ }
            }
        });
        this._filterId = Clutter.event_add_filter(null, event => this._input(event));
        this._acceleratorId = global.display.connect('accelerator-activated', (_, action) => {
            if (this._peer && action === this._accelerator && !this._actor)
                this._enqueue(0, 0, 0, 0);
        });
        this._nameSubscription = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus', null,
            Gio.DBusSignalFlags.NONE, (_, _sender, _path, _iface, _signal, parameters) => {
                const [name, , owner] = parameters.deep_unpack();
                if (name === this._peer && !owner)
                    this._dropPeer();
            });
        this._watchdog = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            if (this._actor && GLib.get_monotonic_time() - this._heartbeat > 3_000_000) {
                this._hide();
                this._events = [[7, this._generation, 0, 0, 0]];
            }
            return GLib.SOURCE_CONTINUE;
        });
        this._object = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._object.export(Gio.DBus.session, PATH);
        this._nameId = Gio.bus_own_name_on_connection(Gio.DBus.session, NAME, Gio.BusNameOwnerFlags.NONE, null, null);
    }

    _credentials(sender) {
        const call = method => Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', method, new GLib.Variant('(s)', [sender]), null,
            Gio.DBusCallFlags.NONE, 1000, null).deep_unpack()[0];
        const uid = call('GetConnectionUnixUser');
        if (uid !== Gio.Credentials.new().get_unix_user())
            return false;
        const executable = GLib.file_read_link(`/proc/${call('GetConnectionUnixProcessID')}/exe`);
        const paths = ['/usr/bin/pasteroll', '/usr/local/bin/pasteroll',
            GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', 'pasteroll'])];
        return paths.some(path => canonical(path) === executable);
    }

    _authorize(invocation) {
        if (!this._alive || !this._peer || invocation.get_sender() !== this._peer) {
            fail(invocation, 'AccessDenied');
            return false;
        }
        return true;
    }

    HandshakeAsync([version], invocation) {
        if (version !== 1) { fail(invocation, 'Version'); return; }
        const sender = invocation.get_sender();
        try {
            if ((this._peer && this._peer !== sender) || !this._credentials(sender)) {
                fail(invocation, 'AccessDenied'); return;
            }
            this._peer = sender;
            invocation.return_value(new GLib.Variant('(uutuu)', [1, 15, this._epoch, MAX_BYTES, MAX_FRAME]));
        } catch (_) { fail(invocation, 'AccessDenied'); }
    }

    _snapshot() {
        const types = St.Clipboard.get_default().get_mimetypes(St.ClipboardType.CLIPBOARD) ?? [];
        if (types.length > 128 || types.some(t => t.length > 256))
            return [this._sequence, [], '', true];
        const excluded = this._excluded || types.includes('x-kde-passwordManagerHint') || types.includes('application/x-keepassxc');
        return [this._sequence, types, this._sourceApp, excluded];
    }

    SnapshotAsync(_args, invocation) {
        if (this._authorize(invocation))
            invocation.return_value(new GLib.Variant('(tassb)', this._snapshot()));
    }

    ReadAsync([sequence, mime, limit], invocation) {
        if (!this._authorize(invocation))
            return;
        if (sequence !== this._sequence) { fail(invocation, 'Stale'); return; }
        if (!TYPES.includes(mime) || this._snapshot()[3]) { fail(invocation, 'AccessDenied'); return; }
        if (limit < 1 || limit > MAX_BYTES || this._reads.size >= 2) { fail(invocation, 'Bounds'); return; }
        const cancellable = new Gio.Cancellable();
        const stream = Gio.MemoryOutputStream.new_resizable();
        this._reads.add(cancellable);
        const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
            cancellable.cancel();
            return GLib.SOURCE_REMOVE;
        });
        this._selection.transfer_async(Meta.SelectionType.SELECTION_CLIPBOARD, mime, limit + 1,
            stream, cancellable, (selection, result) => {
                try {
                    selection.transfer_finish(result);
                    stream.close(null);
                    if (!this._alive || invocation.get_sender() !== this._peer || sequence !== this._sequence) {
                        fail(invocation, 'Stale'); return;
                    }
                    if (stream.get_data_size() > limit) { fail(invocation, 'Bounds'); return; }
                    const data = stream.steal_as_bytes().toArray();
                    invocation.return_value(new GLib.Variant('(ay)', [data]));
                    data.fill(0);
                } catch (_) { fail(invocation, 'Stale'); }
                finally {
                    this._reads.delete(cancellable);
                    // The timer may already have fired. Remove only a still-existing source.
                    const source = GLib.MainContext.default().find_source_by_id(timer);
                    if (source) source.destroy();
                    if (!stream.is_closed()) stream.close(null);
                }
            });
    }

    WriteAsync([mime, data], invocation) {
        if (!this._authorize(invocation))
            return;
        if (!TYPES.includes(mime) || data.length > MAX_BYTES) { fail(invocation, 'Bounds'); return; }
        const owner = Meta.SelectionSourceMemory.new(mime, GLib.Bytes.new(data));
        this._clipboardOwner = owner;
        this._selection.set_owner(Meta.SelectionType.SELECTION_CLIPBOARD, owner);
        this._excluded = true;
        invocation.return_value(new GLib.Variant('(t)', [this._sequence]));
    }

    ArmKeyboardAsync([enabled], invocation) {
        if (!this._authorize(invocation))
            return;
        if (this._accelerator) global.display.ungrab_accelerator(this._accelerator);
        this._accelerator = enabled ? global.display.grab_accelerator('<Control><Shift>space', Meta.KeyBindingFlags.NONE) : 0;
        if (enabled && !this._accelerator) { fail(invocation, 'Unavailable'); return; }
        if (enabled) Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._accelerator), Shell.ActionMode.NORMAL);
        invocation.return_value(null);
    }

    ShowAsync([x, y, width, height, keyboardOptIn], invocation) {
        if (!this._authorize(invocation))
            return;
        if (!keyboardOptIn) { fail(invocation, 'Unavailable'); return; }
        if (!width || !height || width > 4096 || height > 4096 || width * height * 4 > MAX_FRAME) {
            fail(invocation, 'Bounds'); return;
        }
        const monitor = Main.layoutManager.monitors.find(m => x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height)
            ?? Main.layoutManager.primaryMonitor;
        if (!monitor) { fail(invocation, 'Unavailable'); return; }
        const scale = global.display.get_monitor_scale(monitor.index);
        if (width / scale > monitor.width || height / scale > monitor.height) { fail(invocation, 'Bounds'); return; }
        this._hide();
        this._generation++;
        this._events = [];
        this._width = width;
        this._height = height;
        this._actor = new St.Widget({name: 'pasteroll-wayland-overlay', reactive: true, width: width / scale, height: height / scale});
        this._image = new Clutter.Image();
        this._actor.set_content(this._image);
        const ox = Math.round(Math.max(monitor.x, Math.min(x + 14, monitor.x + monitor.width - width / scale)));
        const oy = Math.round(Math.max(monitor.y + 32, Math.min(y + 14, monitor.y + monitor.height - height / scale)));
        this._actor.set_position(ox, oy);
        Main.layoutManager.addChrome(this._actor);
        this._grab = Main.pushModal(this._actor, {actionMode: Shell.ActionMode.POPUP});
        this._heartbeat = GLib.get_monotonic_time();
        invocation.return_value(new GLib.Variant('(tiid)', [this._generation, ox, oy, scale]));
    }

    PresentAsync([generation, width, height, pixels], invocation) {
        if (!this._authorize(invocation))
            return;
        if (!this._actor || generation !== this._generation) { fail(invocation, 'Stale'); return; }
        if (width !== this._width || height !== this._height || pixels.length !== width * height * 4 || pixels.length > MAX_FRAME) {
            fail(invocation, 'Bounds'); return;
        }
        try {
            this._image.set_data(pixels, Cogl.PixelFormat.BGRA_8888, width, height, width * 4);
            this._heartbeat = GLib.get_monotonic_time();
            invocation.return_value(null);
        } catch (_) { fail(invocation, 'Protocol'); }
    }

    HideAsync([generation], invocation) {
        if (!this._authorize(invocation)) return;
        if (generation !== this._generation) { fail(invocation, 'Stale'); return; }
        this._hide();
        invocation.return_value(null);
    }

    PollAsync(_args, invocation) {
        if (!this._authorize(invocation)) return;
        this._heartbeat = GLib.get_monotonic_time();
        invocation.return_value(new GLib.Variant('(a(utiiu))', [this._events.splice(0)]));
    }

    _enqueue(kind, a, b, mods) {
        if (this._events.length >= 256) {
            this._hide();
            this._events = [[7, this._generation, 0, 0, 0]];
            return;
        }
        this._events.push([kind, this._generation, Math.round(a), Math.round(b), mods >>> 0]);
    }

    _input(event) {
        if (!this._actor)
            return Clutter.EVENT_PROPAGATE;
        const type = event.type();
        const mods = event.get_state();
        const [x, y] = event.get_coords();
        const inside = x >= this._actor.x && x < this._actor.x + this._actor.width && y >= this._actor.y && y < this._actor.y + this._actor.height;
        if (type === Clutter.EventType.KEY_PRESS) {
            this._enqueue(2, event.get_key_symbol(), event.get_key_unicode(), mods);
        } else if (type === Clutter.EventType.SCROLL) {
            const direction = event.get_scroll_direction();
            if (direction === Clutter.ScrollDirection.DOWN || direction === Clutter.ScrollDirection.UP)
                this._enqueue(1, direction === Clutter.ScrollDirection.DOWN ? 1 : -1, 0, mods);
        } else if (type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.BUTTON_RELEASE) {
            if (inside && event.get_button() === 1)
                this._enqueue(type === Clutter.EventType.BUTTON_PRESS ? 3 : 4, x - this._actor.x, y - this._actor.y, mods);
            else if (type === Clutter.EventType.BUTTON_RELEASE)
                this._enqueue(event.get_button() === 1 ? 6 : 7, x, y, mods);
        } else if (type === Clutter.EventType.MOTION && inside)
            this._enqueue(5, x - this._actor.x, y - this._actor.y, mods);
        return Clutter.EVENT_STOP;
    }

    _hide() {
        if (this._grab) { Main.popModal(this._grab); this._grab = null; }
        if (this._actor) {
            Main.layoutManager.removeChrome(this._actor);
            this._actor.set_content(null);
            this._actor.destroy();
        }
        this._actor = null;
        this._image = null;
    }

    _dropPeer() {
        this._hide();
        for (const read of this._reads) read.cancel();
        if (this._accelerator) global.display.ungrab_accelerator(this._accelerator);
        this._accelerator = 0;
        this._peer = null;
        this._events = [];
        this._generation++;
    }

    destroy() {
        this._alive = false;
        this._dropPeer();
        this._selection.disconnect(this._selectionId);
        Clutter.event_remove_filter(this._filterId);
        global.display.disconnect(this._acceleratorId);
        Gio.DBus.session.signal_unsubscribe(this._nameSubscription);
        GLib.source_remove(this._watchdog);
        this._object.unexport();
        Gio.bus_unown_name(this._nameId);
        this._clipboardOwner = null;
    }
}
