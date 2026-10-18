"""Native GTK4 clipboard provider/consumer and two real paste targets.

Only synthetic public fixture strings are used. This process is started inside
the disposable launcher and never sees the live desktop or clipboard.
"""
import json
import os
from pathlib import Path
import traceback
import gi

gi.require_version('Gdk', '4.0')
gi.require_version('Gtk', '4.0')
from gi.repository import Gdk, Gio, GLib, Gtk

assert os.environ.get('PASTEROLL_WAYLAND_SPIKE') == '1'
assert os.environ['HOME'].startswith('/tmp/pasteroll-wayland-')
assert Path('/run/pasteroll-wayland-spike').is_file()

Gtk.init()
TITLES = ['Pasteroll synthetic Wayland target',
          'Pasteroll synthetic Wayland target B']
windows = []

for title in TITLES:
    window = Gtk.Window(title=title)
    window.set_default_size(720, 430)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
    for side in ['top', 'bottom', 'start', 'end']:
        getattr(box, 'set_margin_' + side)(28)
    box.append(Gtk.Label(label='Pasteroll native GTK4 Wayland integration fixture'))
    entry = Gtk.TextView()
    entry.set_vexpand(True)
    entry.get_buffer().set_text('Native Wayland fixture')
    box.append(entry)
    window.set_child(box)
    counts = {'button': 0, 'scroll': 0, 'key': 0}

    def count_release(_gesture, _press, _x, _y, counts=counts):
        counts['button'] += 1

    clicks = Gtk.GestureClick.new()
    clicks.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    clicks.connect('released', count_release)
    window.add_controller(clicks)

    def count_scroll(_, _dx, _dy, counts=counts):
        counts['scroll'] += 1
        return False

    scroll_events = Gtk.EventControllerScroll.new(Gtk.EventControllerScrollFlags.VERTICAL)
    scroll_events.connect('scroll', count_scroll)
    window.add_controller(scroll_events)
    windows.append({'window': window, 'entry': entry, 'counts': counts})

windows[1]['window'].present()
windows[0]['window'].present()
windows[0]['entry'].grab_focus()
clipboard = Gdk.Display.get_default().get_clipboard()
XML = '''<node><interface name="com.pasteroll.WaylandFixture">
 <method name="State"><arg type="s" direction="out"/></method>
 <method name="Set"><arg type="s" direction="in"/><arg type="ay" direction="in"/><arg type="b" direction="in"/></method>
 <method name="Read"><arg type="s" direction="in"/><arg type="ay" direction="out"/></method>
 <method name="SetText"><arg type="u" direction="in"/><arg type="s" direction="in"/></method>
 <method name="Clear"><arg type="u" direction="in"/></method>
 <method name="Harden"/>
</interface></node>'''


def error(invocation):
    invocation.return_dbus_error('com.pasteroll.WaylandFixture.Error',
                                 'Synthetic clipboard transfer failed')


def text_of(item):
    buf = item['entry'].get_buffer()
    return buf.get_text(buf.get_start_iter(), buf.get_end_iter(), False)


def index(value):
    if value >= len(windows):
        raise ValueError('fixture window')
    return int(value)


def method(connection, sender, path, interface, name, parameters, invocation):
    try:
        if name == 'State':
            focus_title = ''
            for item in windows:
                if item['window'].get_focus() is not None:
                    focus_title = item['window'].get_title()
                    break
            result = {
                'display': Gdk.Display.get_default().__gtype__.name,
                'focus': focus_title,
                'windows': [{'title': item['window'].get_title(), 'text': text_of(item),
                             'events': item['counts'], 'visible': item['window'].get_visible()}
                            for item in windows],
            }
            invocation.return_value(GLib.Variant('(s)', (json.dumps(result),)))
        elif name == 'Set':
            mime, content, sensitive = parameters.unpack()
            providers = [Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(bytes(content)))]
            if sensitive:
                providers.append(Gdk.ContentProvider.new_for_bytes(
                    'x-kde-passwordManagerHint', GLib.Bytes.new(b'1')))
            provider = Gdk.ContentProvider.new_union(providers) if sensitive else providers[0]
            if not clipboard.set_content(provider):
                error(invocation)
            else:
                invocation.return_value(None)
        elif name == 'Read':
            mime = parameters.unpack()[0]
            data = bytearray()

            def part(stream, result):
                try:
                    chunk = stream.read_bytes_finish(result).get_data()
                    if chunk:
                        data.extend(chunk)
                        if len(data) > 2 * 1024 * 1024:
                            raise ValueError('fixture limit')
                        stream.read_bytes_async(65536, GLib.PRIORITY_DEFAULT, None, part)
                    else:
                        stream.close(None)
                        invocation.return_value(GLib.Variant('(ay)', (data,)))
                except Exception:
                    error(invocation)

            def ready(clip, result):
                try:
                    stream, _ = clip.read_finish(result)
                    stream.read_bytes_async(65536, GLib.PRIORITY_DEFAULT, None, part)
                except Exception:
                    error(invocation)

            clipboard.read_async([mime], GLib.PRIORITY_DEFAULT, None, ready)
        elif name == 'SetText':
            which, value = parameters.unpack()
            item = windows[index(which)]
            item['entry'].get_buffer().set_text(value)
            item['window'].present()
            item['entry'].grab_focus()
            invocation.return_value(None)
        elif name == 'Clear':
            which = index(parameters.unpack()[0])
            windows[which]['entry'].get_buffer().set_text('')
            invocation.return_value(None)
        elif name == 'Harden':
            import ctypes
            if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:  # PR_SET_DUMPABLE
                raise RuntimeError('fixture hardening failed')
            invocation.return_value(None)
        else:
            error(invocation)
    except Exception:
        traceback.print_exc()
        error(invocation)


bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
info = Gio.DBusNodeInfo.new_for_xml(XML)
bus.register_object('/com/pasteroll/WaylandFixture', info.interfaces[0], method, None, None)
Gio.bus_own_name_on_connection(bus, 'com.pasteroll.WaylandFixture',
                              Gio.BusNameOwnerFlags.NONE, None, None)
GLib.MainLoop().run()
