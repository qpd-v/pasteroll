// Only installed by run.py in a disposable, network-isolated GNOME session.
// The private test bus exposes input injection to the harness. Never ship this API.
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const XML = `<node><interface name="com.pasteroll.WaylandSpike">
  <method name="State"><arg type="s" direction="out"/></method>
  <method name="Input"><arg type="s" direction="in"/></method>
  <method name="FocusTarget"/>
  <method name="FocusWindow"><arg type="s" direction="in"/></method>
  <method name="Snapshot"><arg type="s" direction="in"/><arg type="b" direction="out"/></method>
</interface></node>`;

function actor_rect(actor) {
    try {
        const [x, y] = actor.get_transformed_position();
        return [Math.round(x), Math.round(y), Math.round(actor.width), Math.round(actor.height)];
    } catch (_) {
        return [0, 0, 0, 0];
    }
}

function actor_text(actor) {
    for (const key of ['text', 'label']) {
        try {
            if (typeof actor[key] === 'string' && actor[key])
                return actor[key].slice(0, 300);
        } catch (_) { /* Not every actor exposes every property. */ }
    }
    try {
        return actor.get_accessible_name?.() || '';
    } catch (_) {
        return '';
    }
}

function actor_accessible(actor) {
    try {
        return actor.get_accessible_name?.() || '';
    } catch (_) {
        return '';
    }
}

function actor_style(actor) {
    try {
        return actor.style_class ?? '';
    } catch (_) {
        return '';
    }
}

function actor_tree(root, depth = 0, limit = 10) {
    if (!root || depth > limit)
        return [];
    const out = [];
    let child = null;
    try { child = root.get_first_child(); } catch (_) { child = null; }
    while (child && out.length < 1200) {
        const text = actor_text(child);
        if (child.name || text || child.visible)
            out.push({name: child.name ?? '', text, rect: actor_rect(child),
                style: actor_style(child), accessible: actor_accessible(child),
                visible: child.visible, mapped: child.mapped,
                children: actor_tree(child, depth + 1, limit)});
        try { child = child.get_next_sibling(); } catch (_) { child = null; }
    }
    return out;
}

export default class WaylandSpike extends Extension {
    enable() {
        if (GLib.getenv('PASTEROLL_WAYLAND_SPIKE') !== '1' ||
            !GLib.get_home_dir().startsWith('/tmp/pasteroll-wayland-'))
            throw new Error('This experiment requires its isolated launcher');
        this._events = [];
        this._history = [];
        this._selected = 0;
        this._captureGeneration = 0;
        this._overlay = null;
        this._target = null;
        this._timers = new Set();
        this._grab = null;
        this._alive = true;
        const seat = Clutter.get_default_backend().get_default_seat();
        this._pointer = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        this._selection = global.display.get_selection();
        this._selectionId = this._selection.connect('owner-changed', (_, type) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
                return;
            const generation = ++this._captureGeneration;
            St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (_, text) => {
                if (!this._alive || generation !== this._captureGeneration || !text || text === this._history[0])
                    return;
                this._history = [text, ...this._history.filter(t => t !== text)].slice(0, 8);
                this._event('clipboard', {count: this._history.length, length: text.length});
            });
        });
        this._inputId = Clutter.event_add_filter(null, (event, _actor) => this._input(event));
        this._accelerator = (GLib.getenv('PASTEROLL_PHASE2') === '1' ||
            GLib.getenv('PASTEROLL_PHASE3') === '1') ? 0 :
            global.display.grab_accelerator('<Control><Shift>space', Meta.KeyBindingFlags.NONE);
        if (this._accelerator)
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._accelerator), Shell.ActionMode.NORMAL);
        this._acceleratorId = global.display.connect('accelerator-activated', (_, action) => {
            if (action === this._accelerator && !this._overlay) {
                this._event('diagnostic-keyboard-activation');
                this._show();
            }
        });
        this._dbus = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._dbus.export(Gio.DBus.session, '/com/pasteroll/WaylandSpike');
        this._busId = Gio.bus_own_name_on_connection(Gio.DBus.session,
            'com.pasteroll.WaylandSpike', Gio.BusNameOwnerFlags.NONE, null, null);
        this._event('ready', {wayland: Meta.is_wayland_compositor()});
    }

    _event(kind, data = {}) {
        this._events.push({kind, at: GLib.get_monotonic_time(), ...data});
        this._events = this._events.slice(-200);
    }

    _later(ms, callback) {
        const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
            this._timers.delete(id);
            if (this._alive)
                callback();
            return GLib.SOURCE_REMOVE;
        });
        this._timers.add(id);
    }

    State() {
        const companion = Main.extensionManager.lookup('pasteroll@pasteroll.com')?.stateObj?._bridge;
        const focus = global.display.focus_window;
        const focusActor = focus ? global.get_window_actors().find(a => a.meta_window === focus) : null;
        const monitor = Main.layoutManager.primaryMonitor;
        const work = Main.layoutManager.getWorkAreaForMonitor(monitor?.index ?? 0);
        let monitorScale = 1;
        try {
            if (monitor)
                monitorScale = global.display.get_monitor_scale(monitor.index) || 1;
        } catch (_) {
            monitorScale = monitor?.geometry_scale || 1;
        }
        const panel = {};
        for (const [key, actor] of Object.entries(Main.panel.statusArea ?? {})) {
            let menu = null;
            try { menu = actor.menu ?? null; } catch (_) { menu = null; }
            panel[key] = {name: actor.name ?? '', text: actor_text(actor), rect: actor_rect(actor),
                visible: actor.visible, menuOpen: menu?.isOpen ?? false,
                children: actor_tree(actor, 0, 10)};
        }
        return JSON.stringify({wayland: Meta.is_wayland_compositor(),
            companion: companion ? {snapshot: companion._snapshot(),
                mouseReady: companion._inputHelper?.ready ?? false,
                mouseArmed: companion._mouseArmed ?? false,
                mouseTicket: companion._mouseTicket?.ticket ?? 0,
                pendingReads: companion._reads.size, inputActive: companion._inputActive,
                generation: companion._generation, revoked: companion._revokedGeneration,
                peer: companion._peer !== null} : null,
            history: this._history.length, selected: this._selected,
            historyLengths: this._history.map(t => t.length),
            visible: this._overlay !== null, events: this._events,
            bridgeVisible: Main.uiGroup.get_children().some(actor => actor.name === 'pasteroll-wayland-overlay'),
            pointer: global.get_pointer().slice(0, 2),
            modifiers: global.get_pointer()[2],
            focus: focus ? {title: focus.get_title() ?? '', pid: focus.get_pid(),
                class: focus.get_wm_class() ?? '', rect: focusActor ? actor_rect(focusActor) : null} : null,
            monitor: monitor ? [monitor.x, monitor.y, monitor.width, monitor.height, monitorScale] : null,
            workArea: work ? [work.x, work.y, work.width, work.height] : null,
            namedActors: Main.uiGroup.get_children().filter(a => a.name).map(a =>
                ({name: a.name, rect: actor_rect(a), visible: a.visible, mapped: a.mapped})).slice(0, 100),
            stage: Main.uiGroup.get_children().filter(a => a.visible).slice(0, 30).map(a =>
                ({name: a.name ?? '', text: actor_text(a), rect: actor_rect(a),
                    children: actor_tree(a, 0, 10)})),
            panel});
    }

    // Explicit synthetic input only, on the test compositor's private seat.
    Input(json) {
        const input = JSON.parse(json);
        const now = GLib.get_monotonic_time();
        if (input.kind === 'move')
            this._pointer.notify_absolute_motion(now, input.x, input.y);
        else if (input.kind === 'button')
            this._pointer.notify_button(now, input.button,
                input.down ? Clutter.ButtonState.PRESSED : Clutter.ButtonState.RELEASED);
        else if (input.kind === 'key')
            this._keyboard.notify_keyval(now, input.key,
                input.down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
        else if (input.kind === 'scroll')
            this._pointer.notify_discrete_scroll(now,
                input.down ? Clutter.ScrollDirection.DOWN : Clutter.ScrollDirection.UP,
                Clutter.ScrollSource.WHEEL);
        else
            throw new Error('Unsupported test input');
    }

    FocusTarget() {
        const target = global.get_window_actors().map(a => a.meta_window)
            .find(w => w.get_title() === 'Pasteroll synthetic Wayland target');
        if (!target)
            throw new Error('Native test target is absent');
        Main.overview.hide();
        target.move_resize_frame(false, 70, 85, 1000, 650);
        target.activate(global.get_current_time());
        this._target = target;
    }

    FocusWindow(title) {
        const target = global.get_window_actors().map(a => a.meta_window)
            .find(w => (w.get_title() ?? '') === title);
        if (!target)
            throw new Error('Native test target is absent');
        Main.overview.hide();
        target.move_resize_frame(false, 70, 85, 720, 430);
        target.activate(global.get_current_time());
        this._target = target;
    }

    async SnapshotAsync([name], invocation) {
        if (!/^(frame-[0-9]{4}|snap-[a-z0-9-]+)\.png$/.test(name)) {
            invocation.return_dbus_error('com.pasteroll.WaylandSpike.InvalidName', 'Invalid frame name');
            return;
        }
        let stream = null;
        try {
            stream = Gio.File.new_for_path(`${GLib.getenv('PASTEROLL_SPIKE_OUTPUT')}/${name}`)
                .replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
            await new Shell.Screenshot().screenshot(true, stream);
            invocation.return_value(new GLib.Variant('(b)', [true]));
        } catch (error) {
            invocation.return_dbus_error('com.pasteroll.WaylandSpike.ScreenshotFailed', error.message);
        } finally {
            stream?.close(null);
        }
    }

    _show() {
        if (!this._history.length)
            return;
        this._target = global.display.focus_window;
        this._selected = 0;
        const [px, py] = global.get_pointer();
        this._overlay = new St.BoxLayout({vertical: true, reactive: true,
            width: 390, height: 240,
            style: 'background-color: #20242e; border: 2px solid #62dab4; border-radius: 14px; padding: 20px; color: #f4f0e7;'});
        this._overlay.add_child(new St.Label({text: 'PASTEROLL · WAYLAND PROTOTYPE',
            style: 'font-size: 13px; color: #62dab4; margin-bottom: 20px;'}));
        this._label = new St.Label({text: '', style: 'font-size: 22px;'});
        this._overlay.add_child(this._label);
        this._overlay.add_child(new St.Label({text: 'Scroll to choose · click the target to paste · Esc cancels',
            style: 'font-size: 12px; margin-top: 24px;'}));
        Main.layoutManager.addChrome(this._overlay);
        this._overlay.set_position(Math.max(0, Math.min(px + 14, global.stage.width - 400)),
            Math.max(32, Math.min(py + 14, global.stage.height - 260)));
        this._refresh();
        this._grab = Main.pushModal(this._overlay, {actionMode: Shell.ActionMode.POPUP});
        this._event('opened', {pointer: [px, py],
            origin: [this._overlay.x, this._overlay.y],
            target: this._target?.get_title() ?? null});
    }

    _refresh() {
        this._label.text = `${this._selected + 1} / ${this._history.length}\n\n${this._history[this._selected]}`;
    }

    _hide() {
        if (!this._overlay)
            return;
        if (this._grab) {
            Main.popModal(this._grab);
            this._grab = null;
        }
        Main.layoutManager.removeChrome(this._overlay);
        this._overlay.destroy();
        this._overlay = null;
        this._label = null;
    }

    _input(event) {
        const type = event.type();
        if (type === Clutter.EventType.BUTTON_PRESS) {
            const state = event.get_state();
            const button = event.get_button();
            this._event('button-observed', {button, state});
            if (!this._overlay && button === 2 &&
                (state & Clutter.ModifierType.CONTROL_MASK) &&
                (state & Clutter.ModifierType.SHIFT_MASK)) {
                this._show();
                return Clutter.EVENT_STOP;
            }
            if (this._overlay && button === 1) {
                const [x, y] = event.get_coords();
                const box = this._overlay;
                if (x >= box.x && x < box.x + box.width && y >= box.y && y < box.y + box.height)
                    return Clutter.EVENT_STOP;
                const selected = this._history[this._selected];
                this._hide();
                St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, selected);
                // The shell's modal grab owns the original click. Replay it after
                // releasing the grab, then verify focus before sending paste.
                this._pointer.notify_absolute_motion(GLib.get_monotonic_time(), x, y);
                this._later(140, () => {
                    this._pointer.notify_button(GLib.get_monotonic_time(), 1, Clutter.ButtonState.PRESSED);
                    this._pointer.notify_button(GLib.get_monotonic_time(), 1, Clutter.ButtonState.RELEASED);
                    this._later(100, () => this._paste(selected));
                });
                return Clutter.EVENT_STOP;
            }
        }
        if (this._overlay && type === Clutter.EventType.SCROLL) {
            const direction = event.get_scroll_direction();
            // Mutter emits a smooth companion event for a discrete wheel step.
            // This spike proves wheel steps only; do not count both forms.
            if (direction !== Clutter.ScrollDirection.DOWN && direction !== Clutter.ScrollDirection.UP)
                return Clutter.EVENT_STOP;
            const delta = direction === Clutter.ScrollDirection.DOWN ? 1 : -1;
            this._selected = Math.max(0, Math.min(this._history.length - 1, this._selected + delta));
            this._refresh();
            this._event('scrolled', {selected: this._selected});
            return Clutter.EVENT_STOP;
        }
        if (this._overlay && type === Clutter.EventType.KEY_PRESS && event.get_key_symbol() === Clutter.KEY_Escape) {
            this._hide();
            this._event('cancelled');
            return Clutter.EVENT_STOP;
        }
        return Clutter.EVENT_PROPAGATE;
    }

    _paste(selected) {
        if (!this._target || global.display.focus_window !== this._target) {
            this._event('paste-rejected', {reason: 'focus-changed'});
            return;
        }
        const mods = global.get_pointer()[2];
        if (mods & (Clutter.ModifierType.CONTROL_MASK | Clutter.ModifierType.SHIFT_MASK |
            Clutter.ModifierType.MOD1_MASK | Clutter.ModifierType.SUPER_MASK)) {
            this._event('paste-rejected', {reason: 'held-modifier'});
            return;
        }
        const now = GLib.get_monotonic_time();
        for (const [key, down] of [[Clutter.KEY_Control_L, true], [Clutter.KEY_v, true],
            [Clutter.KEY_v, false], [Clutter.KEY_Control_L, false]])
            this._keyboard.notify_keyval(now, key, down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
        this._event('paste-sent', {length: selected.length});
    }

    disable() {
        this._alive = false;
        for (const id of this._timers ?? [])
            GLib.source_remove(id);
        this._timers?.clear();
        this._hide();
        if (this._selectionId)
            this._selection.disconnect(this._selectionId);
        if (this._inputId)
            Clutter.event_remove_filter(this._inputId);
        if (this._acceleratorId)
            global.display.disconnect(this._acceleratorId);
        if (this._accelerator)
            global.display.ungrab_accelerator(this._accelerator);
        this._dbus?.unexport();
        if (this._busId)
            Gio.bus_unown_name(this._busId);
        this._history = [];
        this._events = [];
        this._pointer = null;
        this._keyboard = null;
        this._target = null;
    }
}
