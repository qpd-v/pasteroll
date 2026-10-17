"""Native GTK Wayland target containing only public synthetic fixture strings."""
import json
import os
from pathlib import Path
import gi

gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gio, GLib, Gtk

assert os.environ.get('PASTEROLL_WAYLAND_SPIKE') == '1'
assert os.environ['HOME'].startswith('/tmp/pasteroll-wayland-')
assert Path('/run/pasteroll-wayland-spike').is_file()
FIXTURES = ['A brighter idea.', 'Keep the useful thought.', 'One scroll brings it back.']
XML = '''<node><interface name="com.pasteroll.WaylandTarget">
<method name="Copy"><arg type="u" direction="in"/></method>
<method name="State"><arg type="s" direction="out"/></method>
<method name="Clear"/>
</interface></node>'''
window = Gtk.Window(title='Pasteroll synthetic Wayland target')
window.set_default_size(940, 580)
window.connect('destroy', Gtk.main_quit)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
box.set_border_width(28)
box.pack_start(Gtk.Label(label='PASTEROLL · GNOME WAYLAND FEASIBILITY TEST'), False, False, 0)
box.pack_start(Gtk.Label(label='Synthetic Notes — choose an earlier clip and click below to paste.'), False, False, 0)
entry = Gtk.TextView()
entry.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
scroll = Gtk.ScrolledWindow()
scroll.add(entry)
box.pack_start(scroll, True, True, 0)
window.add(box)
counts = {'button': 0, 'scroll': 0, 'key': 0, 'middle': 0}
def count_event(_, event):
    if event.type == Gdk.EventType.BUTTON_PRESS:
        counts['button'] += 1
    elif event.type == Gdk.EventType.SCROLL:
        counts['scroll'] += 1
    elif event.type == Gdk.EventType.KEY_PRESS:
        counts['key'] += 1
    return False
window.connect('event', count_event)
def count_scroll(_, event):
    counts['scroll'] += 1
    return False
entry.connect('scroll-event', count_scroll)
def count_button(_, event):
    if event.button == 2:
        counts['middle'] += 1
    return False
entry.connect('button-press-event', count_button)
def method(connection, sender, path, interface, name, parameters, invocation):
    if name == 'Copy':
        # Prepare the source; the harness sends a real Ctrl+C to this Wayland
        # text view. Clipboard ownership must carry a genuine input serial.
        buf = entry.get_buffer()
        buf.set_text(FIXTURES[parameters.unpack()[0]])
        buf.select_range(buf.get_start_iter(), buf.get_end_iter())
        entry.grab_focus()
        invocation.return_value(None)
    elif name == 'Clear':
        entry.get_buffer().set_text('')
        invocation.return_value(None)
    elif name == 'State':
        buf = entry.get_buffer()
        result = {'text': buf.get_text(buf.get_start_iter(), buf.get_end_iter(), False),
                  'display': Gdk.Display.get_default().__gtype__.name, 'events': counts}
        invocation.return_value(GLib.Variant('(s)', (json.dumps(result),)))
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
node = Gio.DBusNodeInfo.new_for_xml(XML)
bus.register_object('/com/pasteroll/WaylandTarget', node.interfaces[0], method, None, None)
Gio.bus_own_name_on_connection(bus, 'com.pasteroll.WaylandTarget', Gio.BusNameOwnerFlags.NONE, None, None)
window.show_all()
entry.grab_focus()
Gtk.main()
