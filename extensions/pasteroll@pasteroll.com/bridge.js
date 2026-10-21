import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Graphene from 'gi://Graphene';
import Meta from 'gi://Meta';
import Mtk from 'gi://Mtk';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {createPanel} from './panel.js';
import {stageScaleForMonitor, ViewerTransfer} from './geometry.js';
import {InputHelper} from './input-helper.js';
import {showConfirmation, showSettings, showWelcome} from './settings-ui.js';

const NAME = 'com.pasteroll.Gnome';
const PATH = '/com/pasteroll/Gnome';
const IFACE = 'com.pasteroll.Gnome.Bridge2';
const VERSION = 3;

const CLIPBOARD = 1;
const OVERLAY = 2;
const INPUT = 4;
const KEYBOARD_ACTIVATION = 8;
const MOUSE_ACTIVATION = 16;
const PASTE = 32;
const VIEWER = 64;
const SETTINGS = 128;

const MAX_BYTES = 64 * 1024 * 1024;
const MAX_FRAME = 16 * 1024 * 1024;
const MAX_EVENTS = 256;
const MAX_APPS = 128;
const MAX_APP_NAME = 128;

const SHIFT = 1;
const CONTROL = 4;
const ALT = 8;
const MOD4 = 64;
const MOD5 = 128;
const BUTTON1 = 1 << 8;

const AUTH_MS = 3000;
const CLICK_MAX_MS = 700;
const CLICK_SETTLE_MS = 80;
const CLICK_DISTANCE = 14;
const CLICK_HOLD_MS = 500;

const TYPES = ['text/plain;charset=utf-8', 'text/plain', 'UTF8_STRING', 'image/png',
    'text/uri-list', 'x-special/gnome-copied-files'];
const SENSITIVE_TYPES = ['x-kde-passwordManagerHint', 'application/x-keepassxc'];

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
 <method name="Context"><arg type="t" direction="out"/><arg type="u" direction="out"/>
  <arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="b" direction="out"/>
  <arg type="i" direction="out"/><arg type="i" direction="out"/><arg type="u" direction="out"/>
  <arg type="(iiuu)" direction="out"/><arg type="(iiuu)" direction="out"/>
  <arg type="d" direction="out"/><arg type="d" direction="out"/></method>
 <method name="Current"><arg type="t" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="out"/></method>
 <method name="Privileged"><arg type="t" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="out"/></method>
 <method name="InputActive"><arg type="b" direction="in"/><arg type="t" direction="out"/></method>
 <method name="Modes"><arg type="t" direction="in"/><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
 <method name="ViewerShow"><arg type="t" direction="in"/><arg type="i" direction="in"/>
  <arg type="i" direction="in"/><arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="in"/></method>
 <method name="ViewerPresent"><arg type="t" direction="in"/><arg type="u" direction="in"/>
  <arg type="u" direction="in"/><arg type="ay" direction="in"/></method>
 <method name="ViewerBegin"><arg type="t" direction="in"/><arg type="u" direction="in"/>
  <arg type="u" direction="in"/><arg type="t" direction="out"/></method>
 <method name="ViewerStrip"><arg type="t" direction="in"/><arg type="t" direction="in"/>
  <arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="ay" direction="in"/></method>
 <method name="ViewerCommit"><arg type="t" direction="in"/><arg type="t" direction="in"/></method>
 <method name="ViewerHide"><arg type="t" direction="in"/></method>
 <method name="PollClick"><arg type="t" direction="in"/><arg type="b" direction="in"/>
  <arg type="u" direction="out"/><arg type="t" direction="out"/><arg type="u" direction="out"/>
  <arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="b" direction="out"/></method>
 <method name="SendPaste"><arg type="t" direction="in"/><arg type="t" direction="in"/>
  <arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="out"/></method>
 <method name="Suspend"><arg type="t" direction="in"/><arg type="b" direction="in"/></method>
 <method name="UiState"><arg type="b" direction="in"/><arg type="u" direction="in"/>
  <arg type="u" direction="in"/><arg type="t" direction="in"/><arg type="u" direction="in"/>
  <arg type="t" direction="in"/><arg type="u" direction="in"/><arg type="as" direction="in"/>
  <arg type="b" direction="in"/><arg type="as" direction="in"/><arg type="b" direction="in"/>
  <arg type="b" direction="in"/></method>
 <method name="OpenSettings"/>
 <method name="SettingsResult"><arg type="b" direction="out"/><arg type="b" direction="out"/>
  <arg type="b" direction="out"/><arg type="u" direction="out"/><arg type="u" direction="out"/>
  <arg type="t" direction="out"/><arg type="u" direction="out"/><arg type="t" direction="out"/>
  <arg type="u" direction="out"/><arg type="as" direction="out"/><arg type="b" direction="out"/>
  <arg type="as" direction="out"/><arg type="b" direction="out"/></method>
 <method name="ConfirmSettings"><arg type="u" direction="in"/><arg type="t" direction="in"/>
  <arg type="u" direction="in"/></method>
</interface></node>`;

// Mouse activation shares the versioned bridge geometry contract.
const MOUSE_IFACE = 'com.pasteroll.Gnome.Mouse1';
const MOUSE_XML = `<node><interface name="${MOUSE_IFACE}">
 <method name="Arm"><arg type="b" direction="in"/><arg type="u" direction="out"/></method>
 <method name="Begin"><arg type="t" direction="in"/><arg type="t" direction="out"/></method>
 <method name="Show"><arg type="i" direction="in"/><arg type="i" direction="in"/>
  <arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="t" direction="in"/>
  <arg type="t" direction="out"/><arg type="i" direction="out"/><arg type="i" direction="out"/><arg type="d" direction="out"/></method>
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

function inside(rect, x, y) {
    return rect && x >= rect.x && y >= rect.y &&
        x < rect.x + rect.width && y < rect.y + rect.height;
}

function clamp(value, low, high) {
    return Math.max(low, Math.min(value, high));
}

function lower(value) {
    return typeof value === 'string' ? value.toLowerCase() : '';
}

function validApp(value) {
    return typeof value === 'string' &&
        value.length > 0 &&
        value.length <= MAX_APP_NAME &&
        !/[\s/\\]/.test(value) &&
        ![...value].some(char => {
            const code = char.codePointAt(0);
            return code <= 0x1f || code === 0x7f;
        });
}

function validApps(values) {
    return Array.isArray(values) && values.length <= MAX_APPS && values.every(validApp);
}

function validFinite(value) {
    return typeof value === 'number' && Number.isFinite(value);
}

export class Bridge {
    constructor() {
        if (!Meta.is_wayland_compositor())
            throw new Error('Pasteroll GNOME bridge requires Wayland');

        this._alive = true;
        this._peer = null;
        this._epoch = Math.max(1, GLib.get_monotonic_time());
        this._generationSeq = 1;
        this._generation = this._epoch;
        this._revokedGeneration = 0;
        this._locked = false;
        this._sequence = 1;
        this._events = [];
        this._reads = new Set();
        this._inputActive = false;
        this._keyboardArmed = false;
        this._accelerator = 0;
        this._suspended = false;
        this._revoking = false;

        this._actor = null;
        this._image = null;
        this._width = 0;
        this._height = 0;
        this._scale = 1; // Device pixels per Shell stage unit.
        this._uiScale = 1; // Device pixels per UI DIP.
        this._monitor = null;
        this._work = null;
        this._overlayOnStage = false;
        this._heartbeat = this._nowMs();

        this._viewer = null;
        this._viewerImage = null;
        this._viewerTiles = null;
        this._pendingViewer = null;
        this._viewerWidth = 0;
        this._viewerHeight = 0;
        this._viewerContentWidth = 0;
        this._viewerContentHeight = 0;
        this._viewerRounded = false;
        this._viewerOnStage = false;
        this._viewerMode = false;
        this._modal = false;
        this._viewerLeaveId = 0;

        this._grab = null;
        this._grabActor = null;
        this._dialog = null;

        this._settingsState = null;
        this._pendingSettings = null;
        this._completedSettings = null;
        this._firstRunPending = false;
        this._firstRunAcknowledged = false;

        this._sessionTarget = null;
        this._pasteAuth = null;
        this._click = null;
        this._clickReplay = null;
        this._lastViewerClick = null;
        this._lastMods = 0;
        this._injecting = false;
        this._ignoreInputUntil = 0;
        this._swallowUntilRelease = false;
        this._swallowKind = 0;

        this._clipboardOwner = null;
        this._excluded = false;
        this._sourceApp = '';

        this._ownUid = Gio.Credentials.new().get_unix_user();
        this._ownPid = 0;
        try {
            this._ownPid = Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
                new GLib.Variant('(s)', ['org.gnome.Shell']), null, Gio.DBusCallFlags.NONE,
                1000, null).deep_unpack()[0];
        } catch (_) {
            this._ownPid = 0;
        }

        try {
            const seat = Clutter.get_default_backend().get_default_seat();
            this._pointerDevice = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
            this._keyboardDevice = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        } catch (_) {
            this._pointerDevice = null;
            this._keyboardDevice = null;
        }

        this._capabilities = CLIPBOARD | OVERLAY | INPUT | KEYBOARD_ACTIVATION | VIEWER | SETTINGS;
        if (this._pointerDevice && this._keyboardDevice)
            this._capabilities |= PASTE;

        this._mouseArmed = false;
        this._mouseTicket = null;
        this._mouseGeneration = 0;
        this._mouseSeq = 0;
        this._mouseEverReady = false;
        this._focusChangedAt = this._nowMs();
        this._focusId = global.display.connect('notify::focus-window', () => {
            this._focusChangedAt = this._nowMs();
            this._mouseTicket = null;
        });
        this._mouseObject = Gio.DBusExportedObject.wrapJSObject(MOUSE_XML, {
            ArmAsync: (args, invocation) => this._armMouse(args, invocation),
            BeginAsync: (args, invocation) => this._beginMouse(args, invocation),
            ShowAsync: (args, invocation) => this._showMouse(args, invocation),
        });
        this._mouseObject.export(Gio.DBus.session, `${PATH}/Mouse`);
        this._inputHelper = new InputHelper(() => this._mousePolicy(),
            (_sequence, at) => this._nativeActivation(at), ready => {
                if (ready) {
                    this._capabilities |= MOUSE_ACTIVATION;
                    this._mouseEverReady = true;
                } else {
                    this._capabilities &= ~MOUSE_ACTIVATION;
                    this._mouseTicket = null;
                    if (this._mouseArmed && this._mouseEverReady && !this._locked && this._peer) {
                        this._revokeSession('input helper unavailable');
                        this._enqueue(28, 0, 0, 0, 0);
                    }
                }
            });

        this._panel = createPanel(kind => this._panelAction(kind));

        this._selection = global.display.get_selection();
        this._selectionId = this._selection.connect('owner-changed', (_, type, owner) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
                return;
            this._sequence++;
            this._excluded = owner === this._clipboardOwner;
            this._sourceApp = this._sourceAppNow();
            // Wake the Rust ingest worker without transferring clipboard data.
            // Bursts need one pending notification; sequence checks select the
            // latest offer and still enforce pause, exclusions and own writes.
            if (this._peer && !this._locked && !this._events.some(e => e[0] === 26))
                this._enqueue(26, 0, 0, 0, 0);
        });

        this._filterId = Clutter.event_add_filter(null, event => this._input(event));
        this._acceleratorId = global.display.connect('accelerator-activated', (_, action) => {
            if (this._peer && action === this._accelerator && !this._inputActive) {
                this._heartbeat = this._nowMs();
                this._enqueue(0, 0, 0, 0, 0);
            }
        });
        this._nameSubscription = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus', null,
            Gio.DBusSignalFlags.NONE, (_, _sender, _path, _iface, _signal, parameters) => {
                const [name, , owner] = parameters.deep_unpack();
                if (name === this._peer && !owner)
                    this._dropPeer();
            });
        this._sessionModeId = Main.sessionMode.connect('updated', () => this._sessionModeChanged());
        this._watchdog = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            if (!this._alive)
                return GLib.SOURCE_REMOVE;
            if (!this._suspended && (this._actor || this._viewer || this._dialog) &&
                this._peer && this._nowMs() - this._heartbeat > 3000) {
                const generation = this._generation;
                this._revokeSession('watchdog');
                this._events = [[7, generation, 0, 0, 0]];
            }
            return GLib.SOURCE_CONTINUE;
        });

        this._object = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._object.export(Gio.DBus.session, PATH);
        this._nameId = Gio.bus_own_name_on_connection(Gio.DBus.session, NAME,
            Gio.BusNameOwnerFlags.NONE, null, null);
        this._sessionModeChanged();
    }

    _nowMs() {
        return GLib.get_monotonic_time() / 1000;
    }

    _mousePolicy() {
        const enabled = Boolean(this._alive && this._peer && this._mouseArmed &&
            !this._locked && this._nowMs() - this._heartbeat < 3000);
        const accepting = Boolean(enabled && !this._inputActive && !this._dialog &&
            !this._suspended && (!this._mouseTicket || this._nowMs() > this._mouseTicket.expires) &&
            !Main.overview.visible && Main.actionMode === Shell.ActionMode.NORMAL);
        return [enabled, accepting];
    }

    _nativeActivation(at) {
        const age = GLib.get_monotonic_time() - at;
        if (!this._mousePolicy()[1] || !this._inputHelper.ready || age < 0 || age > 150000 ||
            this._focusChangedAt * 1000 > at)
            return;
        const target = this._foregroundInfo();
        if (!target || (!target.desktop && this._privileged(target.id, target.pid)))
            return;
        const ticket = this._epoch + ++this._mouseSeq;
        this._mouseTicket = {ticket, target, expires: this._nowMs() + 500, peer: this._peer};
        this._enqueue(27, 0, 0, 0, ticket);
    }

    _armMouse([enabled], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (enabled && this._keyboardArmed) {
            fail(invocation, 'Protocol');
            return;
        }
        this._mouseArmed = Boolean(enabled);
        this._heartbeat = this._nowMs();
        this._mouseTicket = null;
        if (!enabled) {
            this._inputHelper.revoke();
            invocation.return_value(new GLib.Variant('(u)', [this._capabilities & ~MOUSE_ACTIVATION]));
            return;
        }
        const peer = this._peer;
        const expires = this._nowMs() + 1600;
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
            if (!this._alive || this._peer !== peer || !this._mouseArmed || this._locked) {
                fail(invocation, 'Stale');
                return GLib.SOURCE_REMOVE;
            }
            if (this._inputHelper.ready) {
                invocation.return_value(new GLib.Variant('(u)', [this._capabilities]));
                return GLib.SOURCE_REMOVE;
            }
            if (this._nowMs() >= expires) {
                this._mouseArmed = false;
                this._inputHelper.revoke();
                fail(invocation, 'Unavailable');
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    _beginMouse([ticket], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        const auth = this._mouseTicket;
        this._mouseTicket = null;
        const current = this._foregroundInfo();
        if (!auth || auth.ticket !== ticket || auth.peer !== this._peer ||
            this._nowMs() > auth.expires || !this._inputHelper.ready ||
            !this._mousePolicy()[1] || !current || current.id !== auth.target.id ||
            current.pid !== auth.target.pid || current.desktop !== auth.target.desktop) {
            fail(invocation, 'Stale');
            return;
        }
        this._setInputActive(true);
        this._mouseGeneration = this._generation;
        invocation.return_value(new GLib.Variant('(t)', [this._generation]));
    }

    _showMouse([x, y, width, height, generation], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._mouseArmed || !this._inputHelper.ready || !this._inputActive ||
            generation !== this._mouseGeneration || !this._currentGeneration(generation)) {
            fail(invocation, 'Stale');
            return;
        }
        this._showFrame(x, y, width, height, invocation);
    }

    _newGeneration() {
        this._generation = this._epoch + this._generationSeq++;
        this._revokedGeneration = 0;
        return this._generation;
    }

    _verifyPeer(sender, callback) {
        const failed = () => callback(false);
        const call = (method, next) => {
            try {
                Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                    'org.freedesktop.DBus', method, new GLib.Variant('(s)', [sender]), null,
                    Gio.DBusCallFlags.NONE, 2000, null, (connection, result) => {
                        try {
                            const value = connection.call_finish(result).deep_unpack()[0];
                            next(value);
                        } catch (_) {
                            failed();
                        }
                    });
            } catch (_) {
                failed();
            }
        };
        call('GetConnectionUnixUser', uid => {
            if (uid !== this._ownUid) {
                failed();
                return;
            }
            call('GetConnectionUnixProcessID', pid => {
                let executable;
                try {
                    executable = GLib.file_read_link(`/proc/${pid}/exe`);
                } catch (_) {
                    failed();
                    return;
                }
                const glibHome = GLib.get_home_dir();
                const environmentHome = GLib.getenv('HOME');
                const paths = ['/usr/bin/pasteroll', '/usr/local/bin/pasteroll',
                    GLib.build_filenamev([glibHome, '.local', 'bin', 'pasteroll'])];
                if (environmentHome && environmentHome !== glibHome)
                    paths.push(GLib.build_filenamev([environmentHome, '.local', 'bin', 'pasteroll']));
                let match = false;
                try {
                    match = paths.some(path => canonical(path) === executable);
                } catch (_) {}
                callback(match);
            });
        });
    }

    _authorize(invocation) {
        if (!this._alive || !this._peer || invocation.get_sender() !== this._peer) {
            fail(invocation, 'AccessDenied');
            return false;
        }
        return true;
    }

    _secure(invocation) {
        if (this._locked) {
            fail(invocation, 'Stale');
            return false;
        }
        return true;
    }

    HandshakeAsync([version], invocation) {
        if (version !== VERSION) {
            fail(invocation, 'Version');
            return;
        }
        const sender = invocation.get_sender();
        if (this._peer && this._peer !== sender) {
            fail(invocation, 'AccessDenied');
            return;
        }
        this._verifyPeer(sender, verified => {
            if (!this._alive || !verified || (this._peer && this._peer !== sender)) {
                fail(invocation, 'AccessDenied');
                return;
            }
            try {
                const changedPeer = this._peer !== sender;
                this._peer = sender;
                if (changedPeer) {
                    this._events = [];
                    this._firstRunAcknowledged = false;
                    this._settingsState = null;
                    this._pendingSettings = null;
                    this._completedSettings = null;
                    this._newGeneration();
                }
                invocation.return_value(new GLib.Variant('(uutuu)',
                    [VERSION, this._capabilities, this._epoch, MAX_BYTES, MAX_FRAME]));
            } catch (_) {
                fail(invocation, 'AccessDenied');
            }
        });
    }

    _sourceAppNow() {
        const focus = global.display.focus_window;
        return focus ? this._exeForWindow(focus) : '';
    }

    _snapshot() {
        const types = St.Clipboard.get_default().get_mimetypes(St.ClipboardType.CLIPBOARD) ?? [];
        if (types.length > 128 || types.some(t => t.length > 256))
            return [this._sequence, [], '', true];
        // Hardened clients can hide /proc/PID/exe. If identification also fails
        // through Shell's desktop-app mapping, configured exclusions must not
        // silently admit a copy from an unidentified source.
        const unidentified = !this._sourceApp && (this._settingsState?.excludeApps.length ?? 0) > 0;
        const excluded = this._excluded || unidentified || types.some(t => SENSITIVE_TYPES.includes(t));
        return [this._sequence, types, this._sourceApp, excluded];
    }

    SnapshotAsync(_args, invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        invocation.return_value(new GLib.Variant('(tassb)', this._snapshot()));
    }

    ReadAsync([sequence, mime, limit], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (sequence !== this._sequence) {
            fail(invocation, 'Stale');
            return;
        }
        if (!TYPES.includes(mime) || this._snapshot()[3]) {
            fail(invocation, 'AccessDenied');
            return;
        }
        if (limit < 1 || limit > MAX_BYTES || this._reads.size >= 2) {
            fail(invocation, 'Bounds');
            return;
        }
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
                    if (!this._alive || this._locked || invocation.get_sender() !== this._peer ||
                        sequence !== this._sequence) {
                        fail(invocation, 'Stale');
                        return;
                    }
                    if (stream.get_data_size() > limit) {
                        fail(invocation, 'Bounds');
                        return;
                    }
                    const data = stream.steal_as_bytes().toArray();
                    invocation.return_value(new GLib.Variant('(ay)', [data]));
                    data.fill(0);
                } catch (_) {
                    fail(invocation, 'Stale');
                } finally {
                    this._reads.delete(cancellable);
                    const source = GLib.MainContext.default().find_source_by_id(timer);
                    if (source)
                        source.destroy();
                    if (!stream.is_closed())
                        stream.close(null);
                }
            });
    }

    WriteAsync([mime, data], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!TYPES.includes(mime) || data.length > MAX_BYTES) {
            fail(invocation, 'Bounds');
            return;
        }
        try {
            const owner = Meta.SelectionSourceMemory.new(mime, GLib.Bytes.new(data));
            this._clipboardOwner = owner;
            this._selection.set_owner(Meta.SelectionType.SELECTION_CLIPBOARD, owner);
            this._excluded = true;
            this._sequence++;
            invocation.return_value(new GLib.Variant('(t)', [this._sequence]));
        } catch (_) {
            fail(invocation, 'Protocol');
        }
    }

    ArmKeyboardAsync([enabled], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (enabled) {
            this._keyboardArmed = true;
            if (!this._armKeybinding()) {
                this._keyboardArmed = false;
                fail(invocation, 'Unavailable');
                return;
            }
        } else {
            this._keyboardArmed = false;
            this._releaseKeybinding();
        }
        invocation.return_value(null);
    }

    _armKeybinding() {
        if (this._accelerator)
            return true;
        try {
            const accelerator = global.display.grab_accelerator('<Control><Shift>space',
                Meta.KeyBindingFlags.NONE);
            if (!accelerator)
                return false;
            this._accelerator = accelerator;
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(accelerator),
                Shell.ActionMode.NORMAL);
            return true;
        } catch (_) {
            this._accelerator = 0;
            return false;
        }
    }

    _releaseKeybinding() {
        if (!this._accelerator)
            return;
        try {
            global.display.ungrab_accelerator(this._accelerator);
        } catch (_) {}
        this._accelerator = 0;
    }

    _windowById(id) {
        if (!id)
            return null;
        for (const actor of global.get_window_actors()) {
            const window = actor.get_meta_window?.();
            if (window?.get_id?.() === id)
                return window;
        }
        return null;
    }

    _windowActorAt(x, y) {
        let actor = null;
        try {
            actor = global.stage.get_actor_at_pos(Clutter.PickMode.ALL, Math.round(x), Math.round(y));
        } catch (_) {
            return null;
        }
        while (actor) {
            const window = actor.get_meta_window?.();
            if (window)
                return window;
            actor = actor.get_parent?.();
        }
        return null;
    }

    _exeForPid(pid) {
        if (!(pid > 0))
            return '';
        try {
            return lower(GLib.path_get_basename(GLib.file_read_link(`/proc/${pid}/exe`)));
        } catch (_) {
            return '';
        }
    }

    _exeForWindow(window) {
        const name = this._exeForPid(window.get_pid?.() ?? 0);
        if (name)
            return name;
        try {
            const app = Shell.WindowTracker.get_default().get_window_app(window);
            const executable = app?.get_app_info()?.get_executable();
            return executable ? lower(GLib.path_get_basename(executable)) : '';
        } catch (_) {
            return '';
        }
    }

    _targetFromWindow(window) {
        if (!window || !window.get_compositor_private?.())
            return null;
        let type;
        let frameType;
        try {
            type = window.get_window_type();
            frameType = window.get_frame_type?.();
        } catch (_) {
            return null;
        }
        if (type === Meta.WindowType.DESKTOP)
            return {id: 0, pid: 0, exe: '', class: '', desktop: true};
        if ([Meta.WindowType.DOCK, Meta.WindowType.MENU, Meta.WindowType.DROPDOWN_MENU,
            Meta.WindowType.POPUP_MENU, Meta.WindowType.TOOLTIP, Meta.WindowType.NOTIFICATION,
            Meta.WindowType.COMBO, Meta.WindowType.DND, Meta.WindowType.OVERRIDE_OTHER]
            .includes(type)) {
            return null;
        }
        if (frameType === Meta.FrameType.MENU)
            return null;
        try {
            if (window.is_override_redirect?.() || window.is_skip_taskbar?.() || window.is_remote?.())
                return null;
        } catch (_) {
            return null;
        }
        const pid = window.get_pid?.() ?? 0;
        if (!(pid > 0) || pid === this._ownPid)
            return null;
        const id = window.get_id?.() ?? 0;
        if (!id)
            return null;
        const executable = this._exeForWindow(window);
        const klass = lower(window.get_wm_class?.() ?? window.get_wm_class_instance?.() ??
            window.get_sandboxed_app_id?.() ?? '');
        if (klass.length > 256 || executable.length > 256)
            return null;
        return {id, pid, exe: executable, class: klass, desktop: false};
    }

    _desktopInfo() {
        return {id: 0, pid: 0, exe: '', class: '', desktop: true};
    }

    _emptyInfo() {
        return {id: 0, pid: 0, exe: '', class: '', desktop: false};
    }

    _panelHit(x, y) {
        const panel = Main.panel;
        if (!panel?.visible)
            return false;
        try {
            const [px, py] = panel.get_transformed_position();
            const [pw, ph] = panel.get_transformed_size();
            return x >= px && y >= py && x < px + pw && y < py + ph;
        } catch (_) {
            return false;
        }
    }

    _desktopAtPointer(x, y) {
        if (this._panelHit(x, y) || Main.overview.visible)
            return false;
        const window = this._windowActorAt(x, y);
        if (window)
            return window.get_window_type?.() === Meta.WindowType.DESKTOP;
        // A Shell popup or another extension's actor is not the desktop.
        // Require an actual background ancestor rather than treating every
        // unrecognized non-window actor as a place to save a file.
        let actor;
        try {
            actor = global.stage.get_actor_at_pos(Clutter.PickMode.ALL, Math.round(x), Math.round(y));
        } catch (_) {
            return false;
        }
        while (actor) {
            if (actor instanceof Meta.BackgroundActor)
                return true;
            actor = actor.get_parent?.();
        }
        return false;
    }

    _foregroundInfo() {
        const focus = global.display.focus_window;
        if (focus) {
            const target = this._targetFromWindow(focus);
            if (target)
                return target;
            return this._emptyInfo();
        }
        const [x, y] = this._pointerCoords();
        return this._desktopAtPointer(x, y) ? this._desktopInfo() : this._emptyInfo();
    }

    _pointerCoords() {
        const pointer = global.get_pointer();
        return [Math.round(pointer[0]), Math.round(pointer[1])];
    }

    _pointerModifiers() {
        const modifiers = global.get_pointer()[2] ?? 0;
        this._lastMods = modifiers >>> 0;
        return this._lastMods;
    }

    _monitorAt(x, y) {
        const monitors = Main.layoutManager.monitors;
        for (const monitor of monitors) {
            if (x >= monitor.x && y >= monitor.y &&
                x < monitor.x + monitor.width && y < monitor.y + monitor.height)
                return monitor;
        }
        return Main.layoutManager.primaryMonitor ?? monitors[0] ?? null;
    }

    _monitorScale(monitor) {
        try {
            const scale = global.display.get_monitor_scale(monitor.index);
            if (Number.isFinite(scale) && scale > 0 && scale <= 64)
                return scale;
        } catch (_) {}
        return Number.isFinite(monitor.geometry_scale) && monitor.geometry_scale > 0
            ? monitor.geometry_scale : 1;
    }

    _stageScale(monitor) {
        const views = global.stage.peek_stage_views().map(view => {
            const rect = new Mtk.Rectangle();
            view.get_layout(rect);
            return {x: rect.x, y: rect.y, width: rect.width, height: rect.height,
                scale: view.get_scale()};
        });
        return stageScaleForMonitor(monitor, views);
    }

    _workFor(monitor) {
        try {
            const work = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
            return {x: work.x, y: work.y, width: work.width, height: work.height};
        } catch (_) {
            return {x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height};
        }
    }

    _context() {
        const [x, y] = this._pointerCoords();
        const monitor = this._monitorAt(x, y);
        if (!monitor)
            throw new Error('no monitor');
        const work = this._workFor(monitor);
        const scale = this._monitorScale(monitor);
        const target = this._foregroundInfo();
        return {
            target,
            pointer: [x, y],
            modifiers: this._pointerModifiers(),
            monitor: [monitor.x, monitor.y, monitor.width, monitor.height],
            work: [work.x, work.y, work.width, work.height],
            scale,
            stageScale: this._stageScale(monitor),
        };
    }

    ContextAsync(_args, invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        try {
            const context = this._context();
            const target = context.target;
            invocation.return_value(new GLib.Variant('(tussbiiu(iiuu)(iiuu)dd)', [
                target.id, target.pid, target.exe, target.class, target.desktop,
                context.pointer[0], context.pointer[1], context.modifiers,
                context.monitor, context.work, context.scale, context.stageScale,
            ]));
        } catch (error) {
            console.error(`Pasteroll context unavailable: ${error.message}`);
            fail(invocation, 'Protocol');
        }
    }

    _current(id, pid) {
        if (this._locked || this._revokedGeneration === this._generation)
            return false;
        const window = this._windowById(id);
        if (!window || !(pid > 0) || window.get_pid?.() !== pid ||
            !window.get_compositor_private?.())
            return false;
        const focus = global.display.focus_window;
        return Boolean(focus && focus.get_id?.() === id && focus.get_pid?.() === pid);
    }

    CurrentAsync([id, pid], invocation) {
        if (!this._authorize(invocation))
            return;
        let current = false;
        try {
            current = !this._locked && this._current(id, pid);
        } catch (_) {
            current = false;
        }
        invocation.return_value(new GLib.Variant('(b)', [current]));
    }

    _pidUid(pid) {
        if (!(pid > 0))
            return null;
        try {
            const [ok, bytes] = GLib.file_get_contents(`/proc/${pid}/status`);
            if (!ok)
                return null;
            const text = new TextDecoder().decode(bytes);
            const line = text.split('\n').find(value => value.startsWith('Uid:'));
            if (!line)
                return null;
            const fields = line.trim().split(/\s+/);
            const effective = Number(fields[2]);
            return Number.isInteger(effective) ? effective : null;
        } catch (_) {
            return null;
        }
    }

    _privileged(id, pid) {
        if (!(id > 0) && pid === 0)
            return false;
        const window = this._windowById(id);
        if (!window || !(pid > 0) || window.get_pid?.() !== pid)
            return true;
        const uid = this._pidUid(pid);
        return uid === null || uid !== this._ownUid;
    }

    PrivilegedAsync([id, pid], invocation) {
        if (!this._authorize(invocation))
            return;
        let privileged = true;
        try {
            privileged = this._locked || this._privileged(id, pid);
        } catch (_) {
            privileged = true;
        }
        invocation.return_value(new GLib.Variant('(b)', [privileged]));
    }

    InputActiveAsync([on], invocation) {
        if (!this._authorize(invocation))
            return;
        if (this._locked) {
            fail(invocation, 'Stale');
            return;
        }
        if (on && this._mouseArmed) {
            fail(invocation, 'Unavailable');
            return;
        }
        this._setInputActive(on);
        invocation.return_value(new GLib.Variant('(t)', [this._generation]));
    }

    _setInputActive(on) {
        if (on) {
            this._newGeneration();
            this._inputActive = true;
            this._events = [];
            this._click = null;
            this._clickReplay = null;
            this._lastViewerClick = null;
            this._viewerMode = false;
            this._modal = false;
            this._sessionTarget = this._foregroundInfo();
            this._pasteAuth = this._sessionTarget && !this._sessionTarget.desktop
                ? {generation: this._generation, target: this._sessionTarget, expires: null}
                : null;
            this._heartbeat = this._nowMs();
        } else {
            this._inputActive = false;
            if (this._actor)
                this._actor.reactive = false;
            this._viewerMode = false;
            this._modal = false;
            this._releaseGrab();
            this._extendAuth();
        }
    }

    ModesAsync([generation, viewer, modal], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._currentGeneration(generation)) {
            fail(invocation, 'Stale');
            return;
        }
        this._viewerMode = Boolean(viewer);
        this._modal = Boolean(modal);
        this._syncGrab();
        invocation.return_value(null);
    }

    _currentGeneration(generation) {
        return generation !== 0 && generation === this._generation &&
            this._revokedGeneration !== this._generation && !this._locked;
    }

    _createViewer() {
        const image = new Clutter.Image();
        const actor = new St.Widget({
            name: 'pasteroll-wayland-viewer',
            reactive: true,
            clip_to_allocation: true,
            style_class: 'pasteroll-viewer',
        });
        actor.set_content(image);
        this._viewerLeaveId = actor.connect('leave-event', () => {
            if (this._viewerMode && this._alive)
                this._enqueue(13, 0, 0, this._pointerModifiers());
            return Clutter.EVENT_PROPAGATE;
        });
        this._viewer = actor;
        this._viewerImage = image;
        this._viewerWidth = 0;
        this._viewerHeight = 0;
        this._viewerContentWidth = 0;
        this._viewerContentHeight = 0;
    }

    _setViewerClip() {
        if (this._viewerTiles) {
            this._layoutViewerTiles(this._viewerTiles);
            return;
        }
        if (!this._viewerImage)
            return;
        try {
            const width = this._viewerContentWidth || this._viewerWidth;
            const height = this._viewerContentHeight || this._viewerHeight;
            this._viewerImage.rounded_clip_radius = this._viewerRounded
                ? Math.round(14 * this._uiScale) : 0;
            const rect = new Graphene.Rect();
            rect.origin.x = 0;
            rect.origin.y = 0;
            rect.size.width = width;
            rect.size.height = height;
            this._viewerImage.set_rounded_clip_bounds(rect);
        } catch (_) {}
    }

    ViewerShowAsync([generation, x, y, width, height, rounded], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._currentGeneration(generation)) {
            fail(invocation, 'Stale');
            return;
        }
        if (!width || !height || width > 4096 || height > 4096 ||
            !Number.isSafeInteger(x) || !Number.isSafeInteger(y)) {
            fail(invocation, 'Bounds');
            return;
        }
        const scale = this._scale > 0 ? this._scale : 1;
        try {
            if (!this._viewer)
                this._createViewer();
            this._viewerWidth = width;
            this._viewerHeight = height;
            this._viewerRounded = Boolean(rounded);
            this._viewer.set_size(width / scale, height / scale);
            this._viewer.set_position(x, y);
            this._viewer.reactive = true;
            this._setViewerClip();
            this._addViewerToStage();
            this._syncGrab();
            this._heartbeat = this._nowMs();
            invocation.return_value(null);
        } catch (_) {
            fail(invocation, 'Protocol');
        }
    }

    ViewerPresentAsync([generation, width, height, pixels], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._currentGeneration(generation) || !this._viewer) {
            fail(invocation, 'Stale');
            return;
        }
        if (!width || !height || width > 4096 || height > 4096 ||
            pixels.length !== width * height * 4 || pixels.length > MAX_FRAME) {
            fail(invocation, 'Bounds');
            return;
        }
        try {
            this._discardPendingViewer();
            const image = this._viewerImage ?? new Clutter.Image();
            image.set_data(pixels, Cogl.PixelFormat.BGRA_8888, width, height, width * 4);
            this._viewerTiles?.actor.destroy();
            this._viewerTiles = null;
            this._viewerImage = image;
            this._viewer.set_content(image);
            this._viewerContentWidth = width;
            this._viewerContentHeight = height;
            this._setViewerClip();
            this._heartbeat = this._nowMs();
            invocation.return_value(null);
        } catch (_) {
            fail(invocation, 'Protocol');
        }
    }

    _discardPendingViewer() {
        this._pendingViewer?.actor.destroy();
        this._pendingViewer = null;
    }

    _layoutViewerTiles(tiles) {
        const width = this._viewerWidth / this._scale;
        const height = this._viewerHeight / this._scale;
        tiles.actor.set_size(width, height);
        for (const tile of tiles.tiles) {
            tile.actor.set_position(0, tile.y * height / tiles.transfer.height);
            tile.actor.set_size(width, tile.height * height / tiles.transfer.height);
        }
    }

    ViewerBeginAsync([generation, width, height], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._currentGeneration(generation) || !this._viewer) {
            fail(invocation, 'Stale');
            return;
        }
        let transfer;
        try {
            transfer = new ViewerTransfer(width, height, MAX_FRAME);
        } catch (_) {
            fail(invocation, 'Bounds');
            return;
        }
        this._discardPendingViewer();
        const frame = this._nextViewerFrame = (this._nextViewerFrame ?? 0) + 1;
        const actor = new St.Widget({visible: false, reactive: false});
        this._viewer.add_child(actor);
        this._pendingViewer = {frame, actor, transfer, tiles: []};
        this._heartbeat = this._nowMs();
        invocation.return_value(new GLib.Variant('(t)', [frame]));
    }

    ViewerStripAsync([generation, frame, y, height, pixels], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        const pending = this._pendingViewer;
        if (!this._currentGeneration(generation) || !pending || pending.frame !== frame) {
            fail(invocation, 'Stale');
            return;
        }
        try {
            pending.transfer.accept(y, height, pixels.length);
            const image = new Clutter.Image();
            image.set_data(pixels, Cogl.PixelFormat.BGRA_8888,
                pending.transfer.width, height, pending.transfer.width * 4);
            const actor = new St.Widget({reactive: false});
            actor.set_content(image);
            pending.actor.add_child(actor);
            pending.tiles.push({actor, y, height});
            this._heartbeat = this._nowMs();
            invocation.return_value(null);
        } catch (_) {
            this._discardPendingViewer();
            fail(invocation, 'Bounds');
        }
    }

    ViewerCommitAsync([generation, frame], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        const pending = this._pendingViewer;
        if (!this._currentGeneration(generation) || !this._viewer ||
            !pending || pending.frame !== frame) {
            fail(invocation, 'Stale');
            return;
        }
        if (!pending.transfer.complete) {
            this._discardPendingViewer();
            fail(invocation, 'Bounds');
            return;
        }
        this._layoutViewerTiles(pending);
        this._viewer.set_content(null);
        this._viewerImage = null;
        this._viewerTiles?.actor.destroy();
        this._viewerTiles = pending;
        this._pendingViewer = null;
        this._viewerContentWidth = pending.transfer.width;
        this._viewerContentHeight = pending.transfer.height;
        pending.actor.show();
        this._heartbeat = this._nowMs();
        invocation.return_value(null);
    }

    ViewerHideAsync([generation], invocation) {
        if (!this._authorize(invocation))
            return;
        if (generation !== this._generation) {
            fail(invocation, 'Stale');
            return;
        }
        this._destroyViewer();
        this._viewerMode = false;
        this._modal = false;
        this._syncGrab();
        invocation.return_value(null);
    }

    _createOverlay() {
        const image = new Clutter.Image();
        const actor = new St.Widget({
            name: 'pasteroll-wayland-overlay',
            reactive: true,
            style_class: 'pasteroll-overlay',
        });
        actor.set_content(image);
        this._actor = actor;
        this._image = image;
    }

    ShowAsync([x, y, width, height, keyboardOptIn], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!keyboardOptIn) {
            fail(invocation, 'Unavailable');
            return;
        }
        if (this._mouseArmed) {
            fail(invocation, 'Unavailable');
            return;
        }
        this._showFrame(x, y, width, height, invocation);
    }

    _showFrame(x, y, width, height, invocation) {
        if (!width || !height || width > 4096 || height > 4096 || width * height * 4 > MAX_FRAME) {
            fail(invocation, 'Bounds');
            return;
        }
        try {
            const monitor = this._monitorAt(x, y);
            if (!monitor) {
                fail(invocation, 'Unavailable');
                return;
            }
            const work = this._workFor(monitor);
            const uiScale = this._monitorScale(monitor);
            const scale = this._stageScale(monitor);
            const logicalWidth = width / scale;
            const logicalHeight = height / scale;
            if (logicalWidth > monitor.width || logicalHeight > monitor.height) {
                fail(invocation, 'Bounds');
                return;
            }
            if (!this._inputActive) {
                this._newGeneration();
                this._click = null;
                this._clickReplay = null;
                this._pasteAuth = null;
                this._sessionTarget = null;
            }
            if (this._actor && (this._width !== width || this._height !== height))
                this._destroyOverlay();
            if (!this._actor)
                this._createOverlay();
            this._width = width;
            this._height = height;
            this._scale = scale;
            this._uiScale = uiScale;
            this._monitor = monitor;
            this._work = work;
            this._actor.set_size(Math.max(1, logicalWidth), Math.max(1, logicalHeight));
            this._actor.reactive = this._inputActive;

            const margin = 14;
            let ox = Math.round(x + margin);
            let oy = Math.round(y + margin);
            if (ox + logicalWidth > work.x + work.width)
                ox = Math.round(x - margin - logicalWidth);
            if (oy + logicalHeight > work.y + work.height)
                oy = Math.round(y - margin - logicalHeight);
            const maxX = Math.max(work.x, Math.floor(work.x + work.width - logicalWidth));
            const maxY = Math.max(work.y, Math.floor(work.y + work.height - logicalHeight));
            ox = clamp(ox, work.x, maxX);
            oy = clamp(oy, work.y, maxY);
            this._actor.set_position(ox, oy);
            this._addOverlayToStage();
            this._syncGrab();
            this._heartbeat = this._nowMs();
            invocation.return_value(new GLib.Variant('(tiid)',
                [this._generation, ox, oy, uiScale]));
        } catch (_) {
            fail(invocation, 'Protocol');
        }
    }

    PresentAsync([generation, width, height, pixels], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._actor || generation !== this._generation || !this._image) {
            fail(invocation, 'Stale');
            return;
        }
        if (width !== this._width || height !== this._height ||
            pixels.length !== width * height * 4 || pixels.length > MAX_FRAME) {
            fail(invocation, 'Bounds');
            return;
        }
        try {
            this._image.set_data(pixels, Cogl.PixelFormat.BGRA_8888, width, height, width * 4);
            this._heartbeat = this._nowMs();
            invocation.return_value(null);
        } catch (_) {
            fail(invocation, 'Protocol');
        }
    }

    HideAsync([generation], invocation) {
        if (!this._authorize(invocation))
            return;
        if (generation !== this._generation) {
            fail(invocation, 'Stale');
            return;
        }
        this._destroyOverlay();
        this._inputActive = false;
        this._extendAuth();
        this._syncGrab();
        invocation.return_value(null);
    }

    _addOverlayToStage() {
        if (!this._actor || this._overlayOnStage || this._dialog || this._suspended || !this._alive)
            return;
        Main.layoutManager.addChrome(this._actor);
        this._overlayOnStage = true;
        const parent = this._actor.get_parent();
        if (this._viewerOnStage && this._viewer?.get_parent() === parent)
            parent.set_child_below_sibling(this._actor, this._viewer);
    }

    _removeOverlayFromStage() {
        if (!this._actor || !this._overlayOnStage)
            return;
        Main.layoutManager.removeChrome(this._actor);
        this._overlayOnStage = false;
    }

    _addViewerToStage() {
        if (!this._viewer || this._viewerOnStage || this._dialog || this._suspended || !this._alive)
            return;
        Main.layoutManager.addChrome(this._viewer);
        this._viewerOnStage = true;
        const parent = this._viewer.get_parent();
        if (this._overlayOnStage && this._actor?.get_parent() === parent)
            parent.set_child_above_sibling(this._viewer, this._actor);
    }

    _removeViewerFromStage() {
        if (!this._viewer || !this._viewerOnStage)
            return;
        Main.layoutManager.removeChrome(this._viewer);
        this._viewerOnStage = false;
    }

    _destroyOverlay() {
        if (this._grabActor === this._actor)
            this._releaseGrab();
        this._removeOverlayFromStage();
        if (this._actor) {
            this._actor.set_content(null);
            this._actor.destroy();
        }
        this._actor = null;
        this._image = null;
        this._width = 0;
        this._height = 0;
    }

    _destroyViewer() {
        this._discardPendingViewer();
        this._viewerTiles = null;
        if (this._grabActor === this._viewer)
            this._releaseGrab();
        this._removeViewerFromStage();
        if (this._viewer && this._viewerLeaveId) {
            try {
                this._viewer.disconnect(this._viewerLeaveId);
            } catch (_) {}
        }
        this._viewerLeaveId = 0;
        if (this._viewer) {
            this._viewer.set_content(null);
            this._viewer.destroy();
        }
        this._viewer = null;
        this._viewerImage = null;
        this._viewerWidth = 0;
        this._viewerHeight = 0;
        this._viewerContentWidth = 0;
        this._viewerContentHeight = 0;
    }

    _rejectedClick() {
        return {status: 1, id: 0, pid: 0, exe: '', class: '', desktop: false};
    }

    _pendingClick() {
        return {status: 0, id: 0, pid: 0, exe: '', class: '', desktop: false};
    }

    _desktopClick() {
        return {status: 2, id: 0, pid: 0, exe: '', class: '', desktop: true};
    }

    _targetClick(target) {
        return {status: 3, id: target.id, pid: target.pid, exe: target.exe,
            class: target.class, desktop: false};
    }

    _clientPoint(window, x, y) {
        let rect;
        try {
            rect = window.get_buffer_rect();
        } catch (_) {
            return false;
        }
        if (!inside(rect, x, y))
            return false;
        // Client-side decorations put the titlebar in the client buffer. Refuse a
        // conservative top band instead of guessing that a drag on its buttons is content.
        if (window.is_client_decorated?.() && !window.is_fullscreen?.()) {
            const band = Math.min(48, Math.max(24, Math.round(rect.height * 0.06)));
            if (y < rect.y + band)
                return false;
        }
        return true;
    }

    _clickCandidate(x, y) {
        if (this._panelHit(x, y) || Main.overview.visible || this._dialog)
            return {kind: 'rejected'};
        const monitor = this._monitorAt(x, y);
        if (!monitor || !inside(this._workFor(monitor), x, y))
            return {kind: 'rejected'};
        const window = this._windowActorAt(x, y);
        if (!window)
            return {kind: this._desktopAtPointer(x, y) ? 'desktop' : 'rejected'};
        if (window.get_window_type?.() === Meta.WindowType.DESKTOP)
            return {kind: 'desktop'};
        const target = this._targetFromWindow(window);
        if (!target || target.desktop || !this._clientPoint(window, x, y))
            return {kind: 'rejected'};
        return {kind: 'target', target, window};
    }

    _replayClick(click) {
        if (!this._pointerDevice)
            return false;
        try {
            const now = GLib.get_monotonic_time();
            this._injecting = true;
            this._ignoreInputUntil = this._nowMs() + 250;
            this._pointerDevice.notify_absolute_motion(now, click.x, click.y);
            this._pointerDevice.notify_button(now, 1, Clutter.ButtonState.PRESSED);
            this._pointerDevice.notify_button(now, 1, Clutter.ButtonState.RELEASED);
            return true;
        } catch (_) {
            return false;
        } finally {
            this._injecting = false;
        }
    }

    _startClickReplay(click) {
        if (!click.complete) {
            if (this._nowMs() - click.pressAt > CLICK_HOLD_MS)
                return this._rejectedClick();
            return this._pendingClick();
        }
        if (!this._currentGeneration(click.generation) || this._locked)
            return this._rejectedClick();
        const candidate = this._clickCandidate(click.x, click.y);
        if (candidate.kind === 'rejected')
            return this._rejectedClick();
        if (candidate.kind === 'target' &&
            this._privileged(candidate.target.id, candidate.target.pid))
            return this._rejectedClick();

        // The original click is still blocked by the modal grab. Hide the
        // carousel and release the grab before replaying it once.
        this._destroyOverlay();
        this._releaseGrab();
        if (!this._replayClick(click))
            return this._rejectedClick();
        click.replayed = true;
        this._clickReplay = {
            generation: click.generation,
            start: this._nowMs(),
            candidate,
            seenKey: '',
            seenAt: 0,
            status: 0,
            result: null,
        };
        return this._pendingClick();
    }

    _verifyClickReplay() {
        const state = this._clickReplay;
        if (!state || state.status === 1)
            return this._rejectedClick();
        if (state.status === 2)
            return this._desktopClick();
        if (state.status === 3 && state.result)
            return this._targetClick(state.result);
        const elapsed = this._nowMs() - state.start;
        if (elapsed > CLICK_MAX_MS) {
            state.status = 1;
            return this._rejectedClick();
        }
        if (elapsed < CLICK_SETTLE_MS)
            return this._pendingClick();
        const click = this._click;
        if (!click || click.generation !== state.generation)
            return this._rejectedClick();
        const window = this._windowActorAt(click.x, click.y);
        const focus = global.display.focus_window;

        if (state.candidate.kind === 'desktop') {
            if (!this._desktopAtPointer(click.x, click.y) || this._dialog)
                return this._rejectedClick();
            state.status = 2;
            return this._desktopClick();
        }

        if (!window || !focus)
            return this._pendingClick();
        const actual = this._targetFromWindow(window);
        const focused = this._targetFromWindow(focus);
        if (!actual || actual.desktop || !focused || focused.desktop)
            return this._rejectedClick();
        const expected = state.candidate.target;
        if (actual.id !== expected.id || actual.pid !== expected.pid ||
            focused.id !== expected.id || focused.pid !== expected.pid)
            return this._rejectedClick();
        if (!this._clientPoint(window, click.x, click.y) || this._privileged(actual.id, actual.pid))
            return this._rejectedClick();
        const key = `${actual.id}:${actual.pid}`;
        const now = this._nowMs();
        if (state.seenKey !== key) {
            state.seenKey = key;
            state.seenAt = now;
            return this._pendingClick();
        }
        if (now - state.seenAt < CLICK_SETTLE_MS)
            return this._pendingClick();
        state.status = 3;
        state.result = actual;
        this._sessionTarget = actual;
        this._pasteAuth = {generation: state.generation, target: actual, expires: null};
        return this._targetClick(actual);
    }

    _pollClick(generation, first) {
        if (!this._currentGeneration(generation) || this._locked)
            return this._rejectedClick();
        const cached = this._clickReplay;
        if (cached && cached.generation === generation) {
            if (cached.status === 3 && cached.result)
                return this._targetClick(cached.result);
            if (cached.status === 2)
                return this._desktopClick();
            if (cached.status === 1)
                return this._rejectedClick();
            return this._verifyClickReplay();
        }
        const click = this._click;
        if (!click || click.generation !== generation || click.button !== 1)
            return this._rejectedClick();
        if (first || !click.replayed)
            return this._startClickReplay(click);
        return this._verifyClickReplay();
    }

    PollClickAsync([generation, first], invocation) {
        if (!this._authorize(invocation))
            return;
        if (generation !== this._generation) {
            fail(invocation, 'Stale');
            return;
        }
        let result;
        try {
            result = this._pollClick(generation, Boolean(first));
        } catch (_) {
            result = this._rejectedClick();
        }
        invocation.return_value(new GLib.Variant('(utussb)', [
            result.status, result.id, result.pid, result.exe, result.class, result.desktop,
        ]));
    }

    _extendAuth() {
        if (this._pasteAuth && this._pasteAuth.expires === null)
            this._pasteAuth.expires = this._nowMs() + AUTH_MS;
    }

    _sendPaste(generation, id, pid, chord) {
        const auth = this._pasteAuth;
        if (!auth || auth.generation !== generation || auth.target.id !== id ||
            auth.target.pid !== pid || auth.target.desktop || chord > 2)
            return false;
        // Burn the authorization even when the final checks fail: retrying a
        // rejected paste would be a second delivery attempt for the same click.
        this._pasteAuth = null;
        if (this._locked || this._revokedGeneration === this._generation || this._viewer)
            return false;
        if (auth.expires !== null && this._nowMs() > auth.expires)
            return false;
        if (!this._current(id, pid) || this._privileged(id, pid))
            return false;
        if (!this._clipboardOwner || !this._excluded)
            return false;
        const modifiers = this._pointerModifiers();
        if (modifiers & (ALT | MOD4 | MOD5 | BUTTON1))
            return false;
        const heldCtrl = Boolean(modifiers & CONTROL);
        const heldShift = Boolean(modifiers & SHIFT);
        const needCtrl = chord === 0 || chord === 2;
        const needShift = chord === 1 || chord === 2;
        if ((heldCtrl && !needCtrl) || (heldShift && !needShift))
            return false;
        if (!this._keyboardDevice)
            return false;

        this._releaseGrab();
        const pressed = [];
        const press = key => {
            this._keyboardDevice.notify_keyval(GLib.get_monotonic_time(), key,
                Clutter.KeyState.PRESSED);
            pressed.push(key);
        };
        const release = key => {
            this._keyboardDevice.notify_keyval(GLib.get_monotonic_time(), key,
                Clutter.KeyState.RELEASED);
        };
        this._injecting = true;
        try {
            if (needCtrl && !heldCtrl)
                press(Clutter.KEY_Control_L);
            if (needShift && !heldShift)
                press(Clutter.KEY_Shift_L);
            const main = chord === 1 ? Clutter.KEY_Insert : Clutter.KEY_v;
            press(main);
            release(main);
            for (const key of pressed.slice(0, -1).reverse())
                release(key);
            this._inputActive = false;
            this._sessionTarget = null;
            this._click = null;
            this._clickReplay = null;
            return true;
        } catch (_) {
            for (const key of pressed.reverse()) {
                try {
                    release(key);
                } catch (_) {}
            }
            return false;
        } finally {
            this._injecting = false;
        }
    }

    SendPasteAsync([generation, id, pid, chord], invocation) {
        if (!this._authorize(invocation))
            return;
        let sent = false;
        try {
            sent = this._sendPaste(generation, id, pid, chord);
        } catch (_) {
            sent = false;
        }
        invocation.return_value(new GLib.Variant('(b)', [sent]));
    }

    SuspendAsync([generation, on], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._currentGeneration(generation)) {
            fail(invocation, 'Stale');
            return;
        }
        this._heartbeat = this._nowMs();
        if (on && !this._suspended) {
            this._suspended = true;
            this._releaseGrab();
            this._removeOverlayFromStage();
            this._removeViewerFromStage();
        } else if (!on && this._suspended) {
            this._suspended = false;
            this._addOverlayToStage();
            if (this._viewer)
                this._addViewerToStage();
            this._syncGrab();
        }
        invocation.return_value(null);
    }

    _actorRect(actor) {
        if (!actor)
            return null;
        try {
            const [x, y] = actor.get_transformed_position();
            const [width, height] = actor.get_transformed_size();
            return {x, y, width, height};
        } catch (_) {
            return null;
        }
    }

    _surfacePoint(actor, x, y, scale) {
        const rect = this._actorRect(actor);
        if (!inside(rect, x, y))
            return null;
        return [Math.round((x - rect.x) * scale), Math.round((y - rect.y) * scale)];
    }

    _viewerPoint(x, y) {
        if (!this._viewer || !this._viewerMode)
            return null;
        return this._surfacePoint(this._viewer, x, y, this._scale > 0 ? this._scale : 1);
    }

    _overlayPoint(x, y) {
        if (!this._actor)
            return null;
        return this._surfacePoint(this._actor, x, y, this._scale > 0 ? this._scale : 1);
    }

    _scrollAmount(event) {
        const direction = event.get_scroll_direction();
        if (direction === Clutter.ScrollDirection.UP)
            return [120, 0];
        if (direction === Clutter.ScrollDirection.DOWN)
            return [-120, 0];
        if (direction === Clutter.ScrollDirection.LEFT)
            return [-120, 1];
        if (direction === Clutter.ScrollDirection.RIGHT)
            return [120, 1];
        return [0, 0];
    }

    _captureClick(event, x, y, mods) {
        if (event.get_button() !== 1)
            return false;
        this._click = {
            generation: this._generation,
            button: 1,
            pressX: x,
            pressY: y,
            x,
            y,
            pressAt: this._nowMs(),
            mods,
            complete: false,
            replayed: false,
        };
        return true;
    }

    _captureClickRelease(event, x, y) {
        const click = this._click;
        if (!click || event.get_button() !== 1)
            return false;
        click.x = x;
        click.y = y;
        const moved = Math.hypot(x - click.pressX, y - click.pressY);
        const elapsed = this._nowMs() - click.pressAt;
        click.complete = moved <= CLICK_DISTANCE && elapsed <= CLICK_HOLD_MS;
        this._enqueue(click.complete ? 6 : 7, 0, 0, click.mods);
        return true;
    }

    _viewerClickCount(event, x, y) {
        let count = 0;
        try {
            count = event.get_click_count?.() ?? 0;
        } catch (_) {}
        const now = this._nowMs();
        const last = this._lastViewerClick;
        if (!count) {
            count = last && now - last.time <= 400 &&
                Math.hypot(x - last.x, y - last.y) <= 8 ? last.count + 1 : 1;
        }
        this._lastViewerClick = {time: now, x, y, count};
        return count;
    }

    _input(event) {
        if (!this._alive)
            return Clutter.EVENT_PROPAGATE;
        const type = event.type();

        if (this._dialog) {
            if (type === Clutter.EventType.KEY_PRESS || type === Clutter.EventType.KEY_RELEASE)
                return Clutter.EVENT_PROPAGATE;
            const [x, y] = event.get_coords();
            const dialog = this._actorRect(this._dialog.dialog);
            if (inside(dialog, x, y))
                return Clutter.EVENT_PROPAGATE;
            return Clutter.EVENT_STOP;
        }

        if (this._injecting)
            return Clutter.EVENT_PROPAGATE;
        if (this._nowMs() < this._ignoreInputUntil)
            return Clutter.EVENT_PROPAGATE;
        if (this._locked || this._suspended)
            return Clutter.EVENT_PROPAGATE;
        if (this._swallowUntilRelease && type === Clutter.EventType.BUTTON_RELEASE) {
            this._swallowUntilRelease = false;
            const kind = this._swallowKind;
            this._swallowKind = 0;
            if (kind)
                this._enqueue(kind, 0, 0, this._lastMods);
            return Clutter.EVENT_STOP;
        }
        if (!this._inputActive && !this._viewerMode)
            return Clutter.EVENT_PROPAGATE;

        let mods = this._lastMods;
        try {
            mods = (event.get_state?.() ?? this._pointerModifiers()) >>> 0;
        } catch (_) {}
        this._lastMods = mods;
        const [x, y] = event.get_coords();

        if (type === Clutter.EventType.KEY_PRESS || type === Clutter.EventType.KEY_RELEASE) {
            if (type === Clutter.EventType.KEY_PRESS) {
                let symbol = 0;
                let unicode = 0;
                try {
                    symbol = event.get_key_symbol() | 0;
                    const character = event.get_key_unicode();
                    // GJS represents gunichar as a string on GNOME 46.
                    unicode = typeof character === 'string'
                        ? (character.codePointAt(0) ?? 0) : (character >>> 0);
                } catch (_) {}
                this._enqueue(2, symbol, unicode, mods);
            }
            return Clutter.EVENT_STOP;
        }

        if (type === Clutter.EventType.SCROLL) {
            const [delta, horizontal] = this._scrollAmount(event);
            if (!delta)
                return Clutter.EVENT_STOP;
            if (this._viewerMode) {
                if (this._viewerPoint(x, y))
                    this._enqueue(8, delta, horizontal, mods);
            } else if (this._inputActive && this._actor && !horizontal) {
                this._enqueue(1, delta > 0 ? -1 : 1, 0, mods);
            }
            return Clutter.EVENT_STOP;
        }

        if (type === Clutter.EventType.MOTION) {
            if (this._viewerMode) {
                const point = this._viewerPoint(x, y);
                if (point)
                    this._enqueueMotion(11, point[0], point[1], mods);
            } else {
                const point = this._overlayPoint(x, y);
                if (point)
                    this._enqueueMotion(5, point[0], point[1], mods);
            }
            return Clutter.EVENT_STOP;
        }

        if (type !== Clutter.EventType.BUTTON_PRESS && type !== Clutter.EventType.BUTTON_RELEASE)
            return Clutter.EVENT_PROPAGATE;

        if (this._viewerMode) {
            const point = this._viewerPoint(x, y);
            if (point) {
                if (event.get_button() === 1) {
                    if (type === Clutter.EventType.BUTTON_PRESS) {
                        const count = this._viewerClickCount(event, x, y);
                        this._enqueue(count === 2 ? 12 : 9, point[0], point[1], mods);
                    } else {
                        this._enqueue(10, point[0], point[1], mods);
                    }
                }
                return Clutter.EVENT_STOP;
            }
            if (type === Clutter.EventType.BUTTON_PRESS) {
                this._swallowUntilRelease = true;
                this._swallowKind = 0;
                this._enqueue(14, 0, 0, mods);
            } else if (type === Clutter.EventType.BUTTON_RELEASE) {
                this._swallowUntilRelease = false;
                this._swallowKind = 0;
            }
            return Clutter.EVENT_STOP;
        }

        if (this._overlayPoint(x, y)) {
            if (event.get_button() === 1) {
                const point = this._overlayPoint(x, y);
                this._enqueue(type === Clutter.EventType.BUTTON_PRESS ? 3 : 4,
                    point[0], point[1], mods);
            }
            return Clutter.EVENT_STOP;
        }

        if (type === Clutter.EventType.BUTTON_PRESS) {
            if (event.get_button() === 1)
                this._captureClick(event, x, y, mods);
            else {
                this._swallowUntilRelease = true;
                this._swallowKind = 7;
            }
        } else if (type === Clutter.EventType.BUTTON_RELEASE) {
            if (event.get_button() === 1)
                this._captureClickRelease(event, x, y);
            else if (!this._swallowUntilRelease)
                this._enqueue(7, 0, 0, mods);
        }
        return Clutter.EVENT_STOP;
    }

    _enqueue(kind, a = 0, b = 0, mods = 0, generation = this._generation) {
        if (this._events.length >= MAX_EVENTS) {
            const current = this._generation;
            this._revokeSession('event queue overflow');
            this._events = [[7, current, 0, 0, 0]];
            return;
        }
        this._events.push([kind, generation, Math.round(a), Math.round(b), mods >>> 0]);
    }

    _enqueueMotion(kind, a, b, mods) {
        const last = this._events[this._events.length - 1];
        if (last && last[0] === kind && last[1] === this._generation) {
            last[2] = Math.round(a);
            last[3] = Math.round(b);
            last[4] = mods >>> 0;
            return;
        }
        this._enqueue(kind, a, b, mods);
    }

    UiStateAsync([paused, size, clips, totalBytes, ageDays, clipBytes, pins,
        excludeApps, warnMultiline, terminalApps, autostart, firstRun], invocation) {
        if (!this._authorize(invocation))
            return;
        if (this._locked) {
            fail(invocation, 'Stale');
            return;
        }
        if (!Array.isArray(excludeApps) || !validApps(excludeApps) ||
            !Array.isArray(terminalApps) || !validApps(terminalApps) ||
            ![size, clips, totalBytes, ageDays, clipBytes, pins].every(validFinite)) {
            fail(invocation, 'Protocol');
            return;
        }
        this._settingsState = {
            paused: Boolean(paused),
            size,
            clips,
            totalBytes,
            ageDays,
            clipBytes,
            pins,
            excludeApps: [...excludeApps],
            warnMultiline: Boolean(warnMultiline),
            terminalApps: [...terminalApps],
            autostart: Boolean(autostart),
            firstRun: Boolean(firstRun),
        };
        this._firstRunPending = Boolean(firstRun);
        if (!firstRun)
            this._firstRunAcknowledged = true;
        this._panel.setPaused(Boolean(paused));
        invocation.return_value(null);
    }

    _dialogWork() {
        const [x, y] = this._pointerCoords();
        const monitor = this._monitorAt(x, y) ?? Main.layoutManager.primaryMonitor;
        return monitor ? this._workFor(monitor) : {x: 0, y: 0, width: 1024, height: 768};
    }

    _prepareDialog() {
        if (this._dialog)
            return false;
        this._releaseGrab();
        this._removeOverlayFromStage();
        this._removeViewerFromStage();
        return true;
    }

    _dialogClosed() {
        this._dialog = null;
        if (this._revoking || !this._alive)
            return;
        this._addOverlayToStage();
        this._addViewerToStage();
        this._syncGrab();
    }

    _openWelcome() {
        if (!this._prepareDialog())
            return true;
        const work = this._dialogWork();
        this._dialog = showWelcome({
            work,
            onAcknowledge: () => {
                if (!this._firstRunPending && this._firstRunAcknowledged)
                    return;
                this._firstRunPending = false;
                this._firstRunAcknowledged = true;
                this._enqueue(25, 0, 0, 0, 0);
            },
            onClose: () => this._dialogClosed(),
        });
        return true;
    }

    _openSettingsDialog() {
        if (!this._settingsState || !this._prepareDialog())
            return false;
        const work = this._dialogWork();
        this._dialog = showSettings({
            work,
            state: this._settingsState,
            onApply: result => {
                this._pendingSettings = {...result};
                this._completedSettings = {...result, changed: true, confirmed: false};
            },
            onCancel: () => {
                this._pendingSettings = null;
                this._completedSettings = null;
            },
            onClose: () => this._dialogClosed(),
        });
        return true;
    }

    OpenSettingsAsync(_args, invocation) {
        if (!this._authorize(invocation))
            return;
        if (this._locked) {
            fail(invocation, 'Stale');
            return;
        }
        try {
            if (this._firstRunPending && !this._firstRunAcknowledged) {
                this._openWelcome();
            } else if (!this._openSettingsDialog()) {
                fail(invocation, 'Unavailable');
                return;
            }
            invocation.return_value(null);
        } catch (error) {
            this._dialog = null;
            console.error(`Pasteroll could not open settings: ${error.message}`);
            fail(invocation, 'Protocol');
        }
    }

    _settingsTuple(changed, confirmed, state) {
        return new GLib.Variant('(bbbuututuasbasb)', [
            changed,
            confirmed,
            Boolean(state.paused),
            Math.max(0, Math.round(state.size)),
            Math.max(0, Math.round(state.clips)),
            Math.max(0, Math.round(state.totalBytes)),
            Math.max(0, Math.round(state.ageDays)),
            Math.max(0, Math.round(state.clipBytes)),
            Math.max(0, Math.round(state.pins)),
            [...state.excludeApps],
            Boolean(state.warnMultiline),
            [...state.terminalApps],
            Boolean(state.autostart),
        ]);
    }

    SettingsResultAsync(_args, invocation) {
        if (!this._authorize(invocation))
            return;
        if (this._locked) {
            this._pendingSettings = null;
            this._completedSettings = null;
        }
        const completed = this._completedSettings;
        this._completedSettings = null;
        if (completed) {
            invocation.return_value(this._settingsTuple(true, completed.confirmed, completed));
            return;
        }
        if (this._pendingSettings) {
            // A confirmation dialog may remain open across several polls. Keep
            // returning the proposal so the caller does not discard its pending
            // prune plan while waiting for the person's decision.
            invocation.return_value(this._settingsTuple(true, false, this._pendingSettings));
            return;
        }
        const state = this._settingsState ?? {
            paused: false, size: 100, clips: 0, totalBytes: 0, ageDays: 0,
            clipBytes: 0, pins: 0, excludeApps: [], warnMultiline: true,
            terminalApps: [], autostart: false,
        };
        invocation.return_value(this._settingsTuple(false, false, state));
    }

    _openConfirmation(clips, bytes, byAge) {
        if (!this._pendingSettings || !this._prepareDialog())
            return false;
        const pending = this._pendingSettings;
        const work = this._dialogWork();
        this._confirmationSettled = false;
        this._dialog = showConfirmation({
            work,
            clips,
            bytes,
            byAge,
            onConfirm: () => {
                this._confirmationSettled = true;
                this._completedSettings = {...pending, changed: true, confirmed: true};
                this._pendingSettings = null;
            },
            onCancel: () => {
                this._confirmationSettled = true;
                this._pendingSettings = null;
                this._completedSettings = null;
            },
            onClose: () => {
                if (!this._confirmationSettled) {
                    this._pendingSettings = null;
                    this._completedSettings = null;
                }
                this._confirmationSettled = false;
                this._dialogClosed();
            },
        });
        return true;
    }

    ConfirmSettingsAsync([clips, bytes, byAge], invocation) {
        if (!this._authorize(invocation) || !this._secure(invocation))
            return;
        if (!this._pendingSettings || !validFinite(clips) || !validFinite(bytes) ||
            !validFinite(byAge)) {
            fail(invocation, 'Protocol');
            return;
        }
        try {
            if (!this._openConfirmation(clips, bytes, byAge)) {
                fail(invocation, 'Unavailable');
                return;
            }
            invocation.return_value(null);
        } catch (_) {
            this._dialog = null;
            fail(invocation, 'Protocol');
        }
    }

    _panelAction(kind) {
        if (!this._alive || !this._peer || this._locked)
            return;
        this._heartbeat = this._nowMs();
        if (kind === 20 && this._mouseArmed) {
            this._nativeActivation(GLib.get_monotonic_time());
            return;
        }
        this._enqueue(kind, 0, 0, 0, 0);
    }

    PollAsync(_args, invocation) {
        if (!this._authorize(invocation))
            return;
        this._heartbeat = this._nowMs();
        invocation.return_value(new GLib.Variant('(a(utiiu))', [this._events.splice(0)]));
    }

    _releaseGrab() {
        if (!this._grab)
            return;
        const grab = this._grab;
        this._grab = null;
        this._grabActor = null;
        try {
            Main.popModal(grab);
        } catch (_) {}
    }

    _syncGrab() {
        if (!this._alive || this._dialog || this._suspended || this._locked) {
            this._releaseGrab();
            return;
        }
        const actor = this._viewerOnStage ? this._viewer
            : (this._inputActive && this._overlayOnStage ? this._actor : null);
        if (!actor) {
            this._releaseGrab();
            return;
        }
        if (this._grabActor === actor && this._grab)
            return;
        this._releaseGrab();
        try {
            const grab = Main.pushModal(actor, {actionMode: Shell.ActionMode.POPUP});
            if (grab) {
                this._grab = grab;
                this._grabActor = actor;
            }
        } catch (_) {
            this._grab = null;
            this._grabActor = null;
        }
    }

    _sessionModeChanged() {
        const locked = Boolean(Main.sessionMode?.isLocked || Main.sessionMode?.isGreeter);
        if (locked === this._locked)
            return;
        this._locked = locked;
        if (locked) {
            this._mouseTicket = null;
            this._inputHelper.revoke();
            this._releaseKeybinding();
            const generation = this._generation;
            this._revokeSession('session lock');
            if (generation)
                this._events = [[7, generation, 0, 0, 0]];
        } else if (this._keyboardArmed && this._peer) {
            this._armKeybinding();
        }
    }

    _revokeSession(_reason) {
        if (this._revoking)
            return;
        this._revoking = true;
        if (this._generation)
            this._revokedGeneration = this._generation;
        this._inputActive = false;
        this._viewerMode = false;
        this._modal = false;
        this._suspended = false;
        this._pasteAuth = null;
        this._sessionTarget = null;
        this._mouseGeneration = 0;
        this._mouseTicket = null;
        this._click = null;
        this._clickReplay = null;
        this._swallowUntilRelease = false;
        this._swallowKind = 0;
        if (this._dialog) {
            const dialog = this._dialog;
            this._dialog = null;
            try {
                dialog.close();
            } catch (_) {}
        }
        this._releaseGrab();
        this._destroyViewer();
        this._destroyOverlay();
        this._events = [];
        this._revoking = false;
    }

    _dropPeer() {
        const generation = this._generation;
        this._revokeSession('owner lost');
        this._peer = null;
        this._mouseArmed = false;
        this._mouseEverReady = false;
        this._inputHelper.revoke();
        this._keyboardArmed = false;
        this._releaseKeybinding();
        this._events = [];
        this._settingsState = null;
        this._pendingSettings = null;
        this._completedSettings = null;
        this._firstRunAcknowledged = false;
        this._firstRunPending = false;
        if (generation)
            this._newGeneration();
    }

    destroy() {
        if (!this._alive)
            return;
        this._alive = false;
        this._mouseArmed = false;
        this._inputHelper?.destroy();
        this._mouseObject?.unexport();
        if (this._focusId)
            global.display.disconnect(this._focusId);
        this._focusId = 0;
        this._revokeSession('disable');
        this._peer = null;
        this._keyboardArmed = false;
        this._releaseKeybinding();
        this._panel?.destroy();
        this._panel = null;
        if (this._selection && this._selectionId) {
            try {
                this._selection.disconnect(this._selectionId);
            } catch (_) {}
        }
        this._selectionId = 0;
        if (this._filterId) {
            try {
                Clutter.event_remove_filter(this._filterId);
            } catch (_) {}
        }
        this._filterId = 0;
        if (this._acceleratorId) {
            try {
                global.display.disconnect(this._acceleratorId);
            } catch (_) {}
        }
        this._acceleratorId = 0;
        if (this._sessionModeId) {
            try {
                Main.sessionMode.disconnect(this._sessionModeId);
            } catch (_) {}
        }
        this._sessionModeId = 0;
        if (this._nameSubscription) {
            try {
                Gio.DBus.session.signal_unsubscribe(this._nameSubscription);
            } catch (_) {}
        }
        this._nameSubscription = 0;
        if (this._watchdog) {
            try {
                GLib.source_remove(this._watchdog);
            } catch (_) {}
        }
        this._watchdog = 0;
        try {
            this._object?.unexport();
        } catch (_) {}
        try {
            if (this._nameId)
                Gio.bus_unown_name(this._nameId);
        } catch (_) {}
        this._nameId = 0;
        this._clipboardOwner = null;
        this._pointerDevice = null;
        this._keyboardDevice = null;
        this._events = [];
    }
}
