// Fixed, root-owned host service. No raw input events cross this connection.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const NAME = 'com.pasteroll.Input';
const PATH = '/com/pasteroll/Input';
const IFACE = 'com.pasteroll.Input1';

export class InputHelper {
    constructor(policy, activated, changed) {
        this._policy = policy;
        this._activated = activated;
        this._changed = changed;
        this._alive = true;
        this._ready = false;
        this._owner = null;
        this._last = 0;
        this._request = 0;
        this._instance = GLib.get_monotonic_time();
        this._discovery = 0;
        this._pending = false;
        this._cancel = new Gio.Cancellable();
        this._watch = Gio.bus_watch_name(Gio.BusType.SYSTEM, NAME,
            Gio.BusNameWatcherFlags.NONE,
            (bus, _name, owner) => {
                const discovery = ++this._discovery;
                this._owner = null;
                this._setReady(false);
                bus.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                    'org.freedesktop.DBus', 'GetConnectionUnixUser',
                    new GLib.Variant('(s)', [owner]), new GLib.VariantType('(u)'),
                    Gio.DBusCallFlags.NONE, 200, this._cancel, (connection, result) => {
                        try {
                            const [uid] = connection.call_finish(result).deep_unpack();
                            if (this._alive && discovery === this._discovery && uid === 0) {
                                this._bus = bus;
                                this._owner = owner;
                                this._last = 0;
                                this._poll();
                            }
                        } catch (_) {}
                    });
            }, () => { ++this._discovery; this._owner = null; this._setReady(false); });
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 40, () => {
            if (!this._alive)
                return GLib.SOURCE_REMOVE;
            this._poll();
            return GLib.SOURCE_CONTINUE;
        });
    }

    get ready() { return this._ready; }

    _setReady(value) {
        if (value === this._ready)
            return;
        this._ready = value;
        this._changed(value);
    }

    _poll() {
        if (!this._alive || !this._owner || this._pending)
            return;
        const owner = this._owner;
        const request = ++this._request;
        const [enabled, accepting] = this._policy();
        this._pending = true;
        this._bus.call(owner, PATH, IFACE, 'Poll',
            new GLib.Variant('(bbtt)', [enabled, accepting, this._instance, request]),
            new GLib.VariantType('(uutt)'), Gio.DBusCallFlags.NONE,
            500, this._cancel, (bus, result) => {
                this._pending = false;
                try {
                    const [version, status, sequence, at] = bus.call_finish(result).deep_unpack();
                    if (!this._alive || owner !== this._owner || request !== this._request)
                        return;
                    const [currentEnabled, currentAccepting] = this._policy();
                    const ready = version === 1 && status === 1 && currentEnabled;
                    this._setReady(ready);
                    if (ready && currentAccepting && Number.isSafeInteger(sequence) &&
                        sequence > this._last && Number.isSafeInteger(at)) {
                        this._last = sequence;
                        this._activated(sequence, at);
                    }
                } catch (_) {
                    this._setReady(false);
                }
            });
    }

    revoke() {
        this._setReady(false);
        // A pending reply cannot renew an enabled lease after this explicit revocation.
        if (this._owner && this._bus) {
            this._bus.call(this._owner, PATH, IFACE, 'Poll',
                new GLib.Variant('(bbtt)', [false, false, this._instance, ++this._request]), null,
                Gio.DBusCallFlags.NONE, 200, null, (bus, result) => {
                    try { bus.call_finish(result); } catch (_) {}
                });
        }
    }

    destroy() {
        if (!this._alive)
            return;
        this._alive = false;
        this.revoke();
        this._cancel.cancel();
        if (this._timer)
            GLib.source_remove(this._timer);
        if (this._watch)
            Gio.bus_unwatch_name(this._watch);
        this._timer = this._watch = 0;
        this._owner = null;
    }
}
