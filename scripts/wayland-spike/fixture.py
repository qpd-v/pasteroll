"""Native GTK4 MIME provider/consumer for the isolated bridge integration suite."""
import json
import os
from pathlib import Path
import gi
gi.require_version('Gdk', '4.0')
gi.require_version('Gtk', '4.0')
from gi.repository import Gdk, Gio, GLib, Gtk

assert os.environ.get('PASTEROLL_WAYLAND_SPIKE') == '1'
assert os.environ['HOME'].startswith('/tmp/pasteroll-wayland-')
assert Path('/run/pasteroll-wayland-spike').is_file()
Gtk.init()
window = Gtk.Window(title='Pasteroll synthetic Wayland target')
window.set_default_size(940, 580)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
for side in ['top', 'bottom', 'start', 'end']:
    getattr(box, 'set_margin_' + side)(28)
box.append(Gtk.Label(label='Pasteroll native GTK4 Wayland integration fixture'))
entry = Gtk.TextView()
entry.set_vexpand(True)
entry.get_buffer().set_text('Native Wayland fixture')
box.append(entry)
window.set_child(box)
clipboard = Gdk.Display.get_default().get_clipboard()
XML = '''<node><interface name="com.pasteroll.WaylandFixture">
 <method name="State"><arg type="s" direction="out"/></method>
 <method name="Set"><arg type="s" direction="in"/><arg type="ay" direction="in"/><arg type="b" direction="in"/></method>
 <method name="Read"><arg type="s" direction="in"/><arg type="ay" direction="out"/></method>
</interface></node>'''

def error(invocation):
    invocation.return_dbus_error('com.pasteroll.WaylandFixture.Error', 'Synthetic clipboard transfer failed')

def method(connection, sender, path, interface, name, parameters, invocation):
    if name == 'State':
        invocation.return_value(GLib.Variant('(s)', (json.dumps({'display': Gdk.Display.get_default().__gtype__.name}),)))
    elif name == 'Set':
        mime, content, sensitive = parameters.unpack()
        providers = [Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(bytes(content)))]
        if sensitive:
            providers.append(Gdk.ContentProvider.new_for_bytes('x-kde-passwordManagerHint', GLib.Bytes.new(b'1')))
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

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
info = Gio.DBusNodeInfo.new_for_xml(XML)
bus.register_object('/com/pasteroll/WaylandFixture', info.interfaces[0], method, None, None)
Gio.bus_own_name_on_connection(bus, 'com.pasteroll.WaylandFixture', Gio.BusNameOwnerFlags.NONE, None, None)
window.present()
entry.grab_focus()
GLib.MainLoop().run()
