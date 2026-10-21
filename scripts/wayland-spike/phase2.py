"""Drive the real Rust bridge against stock GNOME and a native GTK4 clipboard."""
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import threading
import time
import zlib
from gi.repository import Gio, GLib
import run as common

OUT, SOURCE = common.OUT, common.SOURCE

def session():
    procs, checks = [], []
    completed = False
    env = dict(os.environ)
    geometry_layout = env.get('PASTEROLL_GEOMETRY_LAYOUT', '')
    env.update(WAYLAND_DISPLAY='pasteroll-test', GDK_BACKEND='wayland')
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    def call(name, method, params=None, path=None, interface=None):
        return bus.call_sync(name, path or '/' + name.replace('.', '/'), interface or name,
            method, params, None, Gio.DBusCallFlags.NONE, 4000, None)
    def state():
        return json.loads(call('com.pasteroll.WaylandSpike', 'State').unpack()[0])
    def wait(predicate, timeout=30):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            try:
                result = predicate()
                if result: return result
            except GLib.Error:
                pass
            time.sleep(.1)
        raise AssertionError('native integration condition timed out')
    def check(name, result):
        checks.append({'name': name, 'pass': bool(result)})
        print(('PASS ' if result else 'FAIL ') + name, flush=True)
        assert result, name
    def send(kind, **args):
        call('com.pasteroll.WaylandSpike', 'Input', GLib.Variant('(s)', (json.dumps({'kind': kind, **args}),)))
        time.sleep(.1)
    def mime_set(mime, data, sensitive=False):
        call('com.pasteroll.WaylandFixture', 'Set', GLib.Variant('(sayb)', (mime, data, sensitive)))
        time.sleep(.15)
    def mime_read(mime):
        return bytes(call('com.pasteroll.WaylandFixture', 'Read', GLib.Variant('(s)', (mime,))).unpack()[0])
    try:
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'enabled-extensions',
            "['wayland-spike@pasteroll.com', 'pasteroll@pasteroll.com']"], check=True, env=env)
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'disable-user-extensions', 'false'], check=True, env=env)
        subprocess.run(['gsettings', 'set', 'org.gnome.desktop.interface', 'enable-animations', 'false'], check=True, env=env)
        if geometry_layout == 'logical':
            subprocess.run(['gsettings', 'set', 'org.gnome.mutter', 'experimental-features',
                "['scale-monitor-framebuffer']"], check=True, env=env)
        shell = common.launch(['gnome-shell', '--headless', '--virtual-monitor',
            '3840x2160' if geometry_layout else '1280x800',
            '--wayland', '--no-x11', '--sm-disable', '--wayland-display=pasteroll-test'], env, 'shell')
        procs.append(shell)
        wait(state, 45)
        check('stock GNOME Wayland with XWayland disabled', state()['wayland'])
        if geometry_layout:
            current = call('org.gnome.Mutter.DisplayConfig', 'GetCurrentState').unpack()
            connector, modes = current[1][0][0][0], current[1][0][1]
            mode = next(m[0] for m in modes if m[-1].get('is-current'))
            layout_mode = 2 if geometry_layout == 'physical' else 1
            properties = {}
            if current[3]['layout-mode'] != layout_mode:
                assert current[3].get('supports-changing-layout-mode'), 'headless layout is fixed'
                properties['layout-mode'] = GLib.Variant('u', layout_mode)
            call('org.gnome.Mutter.DisplayConfig', 'ApplyMonitorsConfig',
                GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})', (current[0], 1,
                    [(0, 0, 2.0, 0, True, [(connector, mode, {})])],
                    properties)))
            wait(lambda: state()['monitor'][4] == 2)
            expected_size = [3840, 2160] if layout_mode == 2 else [1920, 1080]
            check('isolated 4K 200% ' + geometry_layout + ' layout configured',
                state()['monitor'][2:4] == expected_size)
        procs.append(common.launch(['/usr/bin/python3', str(SOURCE / 'fixture.py')], env, 'fixture'))
        native = wait(lambda: json.loads(call('com.pasteroll.WaylandFixture', 'State').unpack()[0]))
        check('native GTK4 provider and consumer', native['display'] == 'GdkWaylandDisplay')
        wait(lambda: (call('com.pasteroll.WaylandSpike', 'FocusTarget'), True)[1])
        time.sleep(.8)
        send('move', x=300, y=300)
        send('button', button=1, down=True)
        send('button', button=1, down=False)
        # The fixture has now received an actual Wayland input serial.
        rejected = False
        try:
            call('com.pasteroll.Gnome', 'Handshake', GLib.Variant('(u)', (3,)),
                path='/com/pasteroll/Gnome', interface='com.pasteroll.Gnome.Bridge2')
        except GLib.Error as error:
            rejected = 'AccessDenied' in str(error)
        check('unapproved executable cannot attach to the bridge', rejected)
        probe = subprocess.Popen([str(Path(env['HOME']) / '.local/bin/pasteroll')], env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        procs.append(probe)
        threading.Thread(target=common.pump, args=(probe.stderr, OUT / 'probe.log'), daemon=True).start()
        def response():
            assert select.select([probe.stdout], [], [], 8)[0], 'Rust probe timed out'
            line = probe.stdout.readline().decode().strip()
            assert line, 'Rust probe exited; see sanitized probe.log'
            return line
        def request(command):
            probe.stdin.write((command + '\n').encode()); probe.stdin.flush()
            result = response()
            if command == 'poll': return result
            assert result == 'ok ' + command, 'unexpected probe response'
            return True
        check('real Rust client completes owner-pinned handshake', response() == 'ready')
        check('missing mouse capability is reported explicitly', request('caps'))
        check('Context uses the nested geometry and scale signature', request('context'))
        check('keyboard interaction cannot be silently selected', request('no-opt-in'))
        mime_set('text/plain;charset=utf-8', b'Native Wayland fixture')
        check('Rust captures native Wayland UTF-8 text', request('text'))
        mime_set('text/uri-list', b'file:///tmp/synthetic%20clip.png\r\nfile:///tmp/idea.txt\r\n')
        check('Rust captures native Wayland file references', request('files'))
        def chunk(kind, data): return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
        png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 6, 0, 0, 0))
        png += chunk(b'IDAT', zlib.compress((b'\0' + bytes([31, 80, 60, 255]) * 2) * 2)) + chunk(b'IEND', b'')
        mime_set('image/png', png)
        check('Rust captures and validates a native PNG', request('image'))
        mime_set('text/plain;charset=utf-8', b'Public sensitive-marker fixture', True)
        check('sensitive MIME markers prevent capture', request('sensitive'))
        mime_set('text/plain;charset=utf-8', b'x' * 8192)
        check('clipboard transfer respects the requested byte cap', request('large'))
        mime_set('text/plain;charset=utf-8', b'Native Wayland fixture')
        check('changed clipboard sequence invalidates a read', request('stale-read'))
        for command, mime in [('write-text', 'text/plain;charset=utf-8'), ('write-files', 'text/uri-list'), ('write-image', 'image/png')]:
            request(command)
            data = mime_read(mime)
            valid = data == b'Native Wayland fixture' if command == 'write-text' else (
                b'file:///tmp/synthetic%20clip.png' in data if command == 'write-files' else data.startswith(b'\x89PNG'))
            check('native GTK4 receives Rust ' + command, valid)
            check('own clipboard write is not re-recorded: ' + command, request('own'))
        request('show')
        wait(lambda: state()['bridgeVisible'])
        time.sleep(.4)
        check('Rust renderer frame appears on GNOME surface', state()['bridgeVisible'])
        if geometry_layout:
            geometry = state()['companion']
            expected_stage = 1 if geometry_layout == 'physical' else 2
            check('real Clutter stage and UI scales are independent',
                geometry['uiScale'] == 2 and geometry['stageScale'] == expected_stage)
        check('Bridge2 viewer transfer accepts device-pixel frames', request('viewer'))
        check('4K viewer is transferred without a downsampling fallback', request('viewer-4k'))
        view = state()['companion']
        check('real GNOME commits exactly two full-resolution 4K strips',
            view['viewerSize'] == [3840, 2160] and view['viewerTiles'] == 2 and not view['viewerPending'])
        if geometry_layout:
            # Only this isolated synthetic compositor is captured. Check both sides
            # of the strip seam in actual rendered output, not just actor metadata.
            import gi
            gi.require_version('GdkPixbuf', '2.0')
            from gi.repository import GdkPixbuf
            name = 'snap-geometry-viewer.png'
            call('com.pasteroll.WaylandSpike', 'Snapshot', GLib.Variant('(s)', (name,)))
            image = GdkPixbuf.Pixbuf.new_from_file(str(Path(env['PASTEROLL_SPIKE_OUTPUT']) / name))
            pixels = image.get_pixels()
            width, height = image.get_width(), image.get_height()
            stride, channels = image.get_rowstride(), image.get_n_channels()
            sample_y = [100, 1090, 1094, 2000]
            colors = [list(pixels[int(y * height / 2160) * stride + int(width / 2) * channels:][:3])
                      for y in sample_y]
            check('rendered 4K pixels are intact above and below the strip seam',
                all(all(abs(a - b) <= 1 for a, b in zip(color, [60, 80, 31])) for color in colors))
        request('viewer-hide')
        check('Bridge2 suspend/resume keeps the active generation', request('suspend'))
        time.sleep(3.4)  # A legitimate native dialog can outlive the regular watchdog.
        request('resume')
        check('a suspended dialog preserves the surface past the watchdog timeout', state()['bridgeVisible'])
        call('com.pasteroll.WaylandSpike', 'Snapshot', GLib.Variant('(s)', ('frame-0000.png',)))
        check('stale frame generation is rejected', request('stale-frame'))
        send('scroll', down=True)
        send('scroll', down=False)
        send('key', key=ord('a'), down=True)
        send('key', key=ord('a'), down=False)
        kinds = request('poll').removeprefix('events ').split(',')
        check('native wheel and keyboard input reaches Rust with generations', kinds.count('1') == 2 and '2' in kinds)
        request('hide')
        check('hiding the surface releases the modal overlay', not state()['bridgeVisible'])
        request('arm')
        for key, down in [(0xffe3, True), (0xffe1, True), (0x20, True), (0x20, False), (0xffe1, False), (0xffe3, False)]:
            send('key', key=key, down=down)
        check('explicitly armed keyboard shortcut routes activation to Rust', '0' in request('poll').split(' ')[1].split(','))
        request('disarm')
        request('show')
        time.sleep(3.5)
        check('stalled client cannot leave input grabbed', not state()['bridgeVisible'])
        request('show')
        request('drop')
        wait(lambda: not state()['bridgeVisible'])
        check('client disconnect closes the surface', not state()['bridgeVisible'])
        request('reconnect')
        check('client reconnects after owner loss', request('caps'))
        subprocess.run(['gnome-extensions', 'disable', 'pasteroll@pasteroll.com'], env=env,
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        check('extension disable invalidates the Rust client', request('lost'))
        probe.stdin.write(b'quit\n'); probe.stdin.flush()
        probe.wait(timeout=5)
        check('Rust probe exits cleanly', probe.returncode == 0)
        completed = True
    finally:
        (OUT / 'results.json').write_text(json.dumps({'checks': checks,
            'geometry_layout': geometry_layout,
            'completed': completed,
            'required_checks_passed': completed and all(c['pass'] for c in checks),
            'full_controller_integrated': False, 'global_mouse_activation': False}, indent=2) + '\n')
        for proc in reversed(procs):
            common.stop(proc)
