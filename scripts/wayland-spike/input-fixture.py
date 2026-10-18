"""Root-only synthetic evdev fixture + fake logind, inside the private test namespace.

No production service imports this file. No physical devices or live bus are used.
"""
import ctypes as C
import json
import os
from pathlib import Path
import stat
import struct
import sys
from gi.repository import Gio, GLib

sys.dont_write_bytecode = True
guard = Path('/run/pasteroll-wayland-spike')
assert os.geteuid() == 0 and guard.is_file() and guard.stat().st_uid == 0
assert os.environ['DBUS_SYSTEM_BUS_ADDRESS'].startswith('unix:path=/tmp/pasteroll-wayland-')
assert Path('/dev/input').stat().st_dev != Path('/dev').stat().st_dev, 'private /dev/input mount required'
UID = int(sys.argv[1])
lib = C.CDLL('libevdev.so.2')
P = C.c_void_p
for name, restype, args in [
    ('libevdev_new', P, []), ('libevdev_free', None, [P]),
    ('libevdev_set_name', None, [P,C.c_char_p]),
    ('libevdev_set_phys', None, [P,C.c_char_p]),
    ('libevdev_enable_event_type', C.c_int, [P,C.c_uint]),
    ('libevdev_enable_event_code', C.c_int, [P,C.c_uint,C.c_uint,P]),
    ('libevdev_uinput_create_from_device', C.c_int, [P,C.c_int,C.POINTER(P)]),
    ('libevdev_uinput_get_syspath', C.c_char_p, [P]),
    ('libevdev_uinput_write_event', C.c_int, [P,C.c_uint,C.c_uint,C.c_int]),
    ('libevdev_uinput_destroy', None, [P]),
]:
    f = getattr(lib,name)
    f.restype, f.argtypes = restype, args

devices = {}
observers = {}
forwarded = []
EVENT = struct.Struct('llHHi')
policy = {'active': True, 'locked': False, 'fallback': False}

def create(name, codes):
    dev = lib.libevdev_new()
    lib.libevdev_set_name(dev, ('Pasteroll synthetic ' + name).encode())
    lib.libevdev_set_phys(dev, ('pasteroll-fixture/' + name).encode())
    for ty, values in codes.items():
        assert lib.libevdev_enable_event_type(dev,ty) == 0
        for code in values:
            assert lib.libevdev_enable_event_code(dev,ty,code,None) == 0
    output = P()
    assert lib.libevdev_uinput_create_from_device(dev,-2,C.byref(output)) == 0
    lib.libevdev_free(dev)
    devices[name] = output

def mirror():
    # Only our disposable devices appear in this namespace's /dev/input. The
    # host devtmpfs remains behind this private tmpfs mount.
    paths = set()
    for path in Path('/sys/class/input').glob('event*'):
        try:
            name = (path/'device/name').read_text().strip()
            if not name.startswith('Pasteroll synthetic '):
                continue
            major, minor = map(int,(path/'dev').read_text().split(':'))
            node = Path('/dev/input')/path.name
            paths.add(node)
            if not node.exists():
                os.mknod(node, stat.S_IFCHR|0o600, os.makedev(major,minor))
            phys = (path/'device/phys').read_text().strip()
            if phys.startswith('pasteroll-input/') and node not in observers:
                observers[node] = os.open(node, os.O_RDONLY|os.O_NONBLOCK|os.O_CLOEXEC)
        except (OSError,ValueError):
            continue
    for node in Path('/dev/input').glob('event*'):
        if node not in paths:
            node.unlink(missing_ok=True)
    for node, fd in list(observers.items()):
        try:
            while True:
                data = os.read(fd, EVENT.size*256)
                for at in range(0,len(data),EVENT.size):
                    _,_,ty,code,value = EVENT.unpack_from(data,at)
                    forwarded.append([ty,code,value])
                    if len(forwarded)>8192:
                        del forwarded[:4096]
        except BlockingIOError:
            pass
        except OSError:
            os.close(fd)
            del observers[node]
    return GLib.SOURCE_CONTINUE

create('keyboard', {1:[29,97,42,54,56,100,125,126]})
create('keyboard2', {1:[29,97,42,54,56,100,125,126]})
create('mouse', {1:[272,273,274,275],2:[0,1,6,8,11,12]})
create('mouse2', {1:[272,273,274,275],2:[0,1,6,8,11,12]})
mirror()
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM,None)
XML = '''<node>
<interface name="com.pasteroll.InputFixture">
 <method name="Emit"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="u" direction="in"/><arg type="i" direction="in"/></method>
 <method name="Batch"><arg type="a(suui)" direction="in"/></method>
 <method name="Read"><arg type="s" direction="out"/></method>
 <method name="Policy"><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
 <method name="Fallback"><arg type="b" direction="in"/></method>
 <method name="Device"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
</interface>
<interface name="org.freedesktop.login1.Manager">
 <method name="GetSessionByPID"><arg type="u" direction="in"/><arg type="o" direction="out"/></method>
 <method name="GetUser"><arg type="u" direction="in"/><arg type="o" direction="out"/></method>
</interface>
<interface name="org.freedesktop.DBus.Properties">
 <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
 <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
</interface></node>'''

def handle(_bus,_sender,_path,iface,method,params,inv):
    try:
        args=params.unpack()
        if method=='GetSessionByPID':
            pid=args[0]
            assert Path(f'/proc/{pid}').stat().st_uid==UID
            assert os.readlink(f'/proc/{pid}/exe')=='/usr/bin/gnome-shell'
            if policy['fallback']:
                inv.return_dbus_error('org.freedesktop.login1.NoSessionForPID','Synthetic user-manager fixture')
                return
            inv.return_value(GLib.Variant('(o)',('/org/freedesktop/login1/session/test',)))
        elif method=='GetUser':
            assert args[0]==UID
            inv.return_value(GLib.Variant('(o)',('/org/freedesktop/login1/user/test',)))
        elif method=='Get':
            assert args==('org.freedesktop.login1.User','Display')
            inv.return_value(GLib.Variant('(v)',(GLib.Variant('(so)',('test','/org/freedesktop/login1/session/test')),)))
        elif method=='GetAll':
            values={
                'Active': GLib.Variant('b',policy['active']),
                'LockedHint': GLib.Variant('b',policy['locked']),
                'Remote': GLib.Variant('b',False),
                'Class': GLib.Variant('s','user'), 'Type': GLib.Variant('s','wayland'),
                'User': GLib.Variant('(uo)',(UID,'/org/freedesktop/login1/user/test')),
                'Seat': GLib.Variant('(so)',('seat0','/org/freedesktop/login1/seat/seat0')),
            }
            inv.return_value(GLib.Variant('(a{sv})',(values,)))
        elif method=='Emit':
            name,ty,code,value=args
            assert name in devices and ty in (0,1,2)
            assert lib.libevdev_uinput_write_event(devices[name],ty,code,value)==0
            inv.return_value(None)
        elif method=='Read':
            mirror()
            result=json.dumps({'events':forwarded[:], 'outputs':len(observers),'sources':len(devices)})
            forwarded.clear()
            inv.return_value(GLib.Variant('(s)',(result,)))
        elif method=='Batch':
            assert len(args[0])<=64
            for name,ty,code,value in args[0]:
                assert name in devices and ty in (0,1,2)
                assert lib.libevdev_uinput_write_event(devices[name],ty,code,value)==0
                assert lib.libevdev_uinput_write_event(devices[name],0,0,0)==0
            inv.return_value(None)
        elif method=='Policy':
            policy.update(active=args[0],locked=args[1])
            inv.return_value(None)
        elif method=='Fallback':
            policy['fallback']=args[0]
            inv.return_value(None)
        elif method=='Device':
            name,present=args
            assert name in ('mouse','mouse2','keyboard2')
            if not present and name in devices:
                lib.libevdev_uinput_destroy(devices.pop(name))
            elif present and name not in devices:
                create(name,{1:[272,273,274,275],2:[0,1,6,8,11,12]} if name.startswith('mouse') else {1:[29,97,42,54,56,100,125,126]})
            mirror()
            inv.return_value(None)
        else:
            raise ValueError()
    except Exception:
        inv.return_dbus_error('com.pasteroll.InputFixture.Error','synthetic fixture request rejected')

info=Gio.DBusNodeInfo.new_for_xml(XML)
for iface in info.interfaces:
    path=('/org/freedesktop/login1' if iface.name.endswith('Manager') else
          '/org/freedesktop/login1/session/test' if iface.name.endswith('Properties') else '/com/pasteroll/InputFixture')
    bus.register_object(path,iface,handle,None,None)
    if iface.name.endswith('Properties'):
        bus.register_object('/org/freedesktop/login1/user/test',iface,handle,None,None)
Gio.bus_own_name_on_connection(bus,'org.freedesktop.login1',Gio.BusNameOwnerFlags.NONE,None,None)
Gio.bus_own_name_on_connection(bus,'com.pasteroll.InputFixture',Gio.BusNameOwnerFlags.NONE,None,None)
GLib.timeout_add(10,mirror,priority=GLib.PRIORITY_DEFAULT)
try:
    GLib.MainLoop().run()
finally:
    for value in devices.values():
        lib.libevdev_uinput_destroy(value)
    for fd in observers.values():
        os.close(fd)
