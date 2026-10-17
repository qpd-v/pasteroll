"""Launch and measure the GNOME experiment in the namespace created by run.sh.

No production Pasteroll binary is run. A real GNOME Wayland compositor, native
GTK client and research-only extension prove the required desktop capabilities.
"""
import getpass
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parent
ROOT = SOURCE.parents[1]
OUT = ROOT / 'target-dev/wayland-phase1'
UUID = 'wayland-spike@pasteroll.com'

def sanitize(value):
    text = str(value)
    for private in (str(ROOT), str(Path.home()), getpass.getuser()):
        if private and len(private) > 2:
            text = text.replace(private, '[private]')
    return text

def pump(pipe, destination):
    with destination.open('w', encoding='utf-8') as log:
        for line in iter(pipe.readline, b''):
            log.write(sanitize(line.decode('utf-8', 'replace')))
            log.flush()

def launch(command, env, name):
    proc = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            start_new_session=True)
    threading.Thread(target=pump, args=(proc.stdout, OUT / f'{name}.log'), daemon=True).start()
    return proc

def stop(proc):
    if proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()

def session():
    from gi.repository import Gio, GLib
    procs = []
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    def call(dest, method, params=None, interface=None):
        return bus.call_sync(dest, '/' + dest.replace('.', '/'), interface or dest,
            method, params, None, Gio.DBusCallFlags.NONE, 3000, None)
    def state():
        return json.loads(call('com.pasteroll.WaylandSpike', 'State').unpack()[0])
    def target():
        return json.loads(call('com.pasteroll.WaylandTarget', 'State').unpack()[0])
    def wait(predicate, description, timeout=15):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            try:
                result = predicate()
                if result:
                    return result
            except GLib.Error:
                pass
            time.sleep(.1)
        raise AssertionError(description)
    checks = []
    recording_stop = threading.Event()
    recording_thread = None
    frame_samples = []
    recording_errors = []
    def check(name, ok, required=True):
        checks.append({'name': name, 'pass': bool(ok), 'required': required})
        print(('PASS ' if ok else 'FAIL ') + name, flush=True)
        if not ok and required:
            raise AssertionError(name)
    try:
        # Memory dconf backend is per-process; use the disposable HOME-backed dconf database.
        env = dict(os.environ)
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'enabled-extensions', f"['{UUID}']"],
                       env=env, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'disable-user-extensions', 'false'], env=env, check=True)
        subprocess.run(['gsettings', 'set', 'org.gnome.desktop.interface', 'enable-animations', 'false'], env=env, check=True)
        shell = launch(['gnome-shell', '--headless', '--virtual-monitor', '1280x800', '--wayland', '--no-x11', '--sm-disable',
                        '--wayland-display=pasteroll-test'], env, 'shell')
        procs.append(shell)
        wait(lambda: state(), 'GNOME extension did not start', 45)
        check('real GNOME Wayland compositor', state()['wayland'])
        time.sleep(1)
        env['WAYLAND_DISPLAY'] = 'pasteroll-test'
        env['GDK_BACKEND'] = 'wayland'
        env.pop('DISPLAY', None)
        procs.append(launch(['/usr/bin/python3', str(SOURCE / 'target.py')], env, 'target'))
        wait(lambda: target(), 'native GTK target did not start')
        check('native Wayland client (XWayland disabled)', target()['display'] == 'GdkWaylandDisplay')
        wait(lambda: (call('com.pasteroll.WaylandSpike', 'FocusTarget'), True)[1],
             'native target window was not mapped')
        time.sleep(1)
        def send(kind, **values):
            call('com.pasteroll.WaylandSpike', 'Input', GLib.Variant('(s)',
                (json.dumps({'kind': kind, **values}),)))
            time.sleep(.12)
        def open_diagnostic():
            for key, down in [(0xffe3, True), (0xffe1, True), (0x20, True),
                              (0x20, False), (0xffe1, False), (0xffe3, False)]:
                send('key', key=key, down=down)
        for n in range(3):
            call('com.pasteroll.WaylandTarget', 'Copy', GLib.Variant('(u)', (n,)))
            for key, down in [(0xffe3, True), (ord('c'), True), (ord('c'), False), (0xffe3, False)]:
                send('key', key=key, down=down)
            wait(lambda: state()['history'] == n + 1, 'clipboard change was not captured')
            time.sleep(.3)
        check('three native clipboard changes captured', state()['history'] == 3)
        check('clipboard capture order remains newest first', state()['historyLengths'] == [26, 24, 16])
        call('com.pasteroll.WaylandTarget', 'Clear')
        def record():
            started = time.monotonic()
            while not recording_stop.is_set() and len(frame_samples) < 600:
                name = f'frame-{len(frame_samples):04d}.png'
                try:
                    call('com.pasteroll.WaylandSpike', 'Snapshot', GLib.Variant('(s)', (name,)))
                    frame_samples.append({'file': name, 'at': time.monotonic() - started})
                except Exception as exc:
                    recording_errors.append(sanitize(exc))
                    return
                recording_stop.wait(.1)
        recording_thread = threading.Thread(target=record, daemon=True)
        recording_thread.start()
        time.sleep(1)
        send('move', x=500, y=250)
        send('key', key=0xffe3, down=True)
        send('key', key=0xffe1, down=True)
        send('button', button=2, down=True)
        send('button', button=2, down=False)
        send('key', key=0xffe1, down=False)
        send('key', key=0xffe3, down=False)
        check('Ctrl+Shift+middle-click opens overlay over native client', state()['visible'], required=False)
        check('native client received the unclaimed middle-click', target()['events']['middle'] > 0)
        time.sleep(.8)
        if not state()['visible']:
            # Diagnostic only. This does not select a replacement product gesture.
            open_diagnostic()
            check('diagnostic keyboard activation opens overlay', state()['visible'])
        time.sleep(1)
        opened = [e for e in state()['events'] if e['kind'] == 'opened'][-1]
        check('overlay appears near the actual pointer', opened['pointer'] == [500, 250] and opened['origin'] == [514, 264])
        send('scroll', down=True)
        check('wheel chooses older clip', state()['selected'] == 1)
        time.sleep(.7)
        send('scroll', down=True)
        check('wheel reaches oldest clip', state()['selected'] == 2)
        time.sleep(.7)
        send('scroll', down=False)
        check('reverse wheel chooses newer clip', state()['selected'] == 1)
        time.sleep(1)
        send('move', x=220, y=420)
        send('button', button=1, down=True)
        send('button', button=1, down=False)
        wait(lambda: target()['text'] == 'Keep the useful thought.', 'real paste did not reach GTK target')
        check('real selected text pasted into native target', target()['text'] == 'Keep the useful thought.')
        check('overlay closes after paste', not state()['visible'])
        time.sleep(2)
        check('scroll did not leak to the target', target()['events']['scroll'] == 0)
        recording_stop.set()
        recording_thread.join(timeout=5)
        check('real compositor frames recorded', not recording_errors and len(frame_samples) >= 20)
        open_diagnostic()
        check('diagnostic activation can reopen after paste', state()['visible'])
        send('key', key=0xff1b, down=True)
        send('key', key=0xff1b, down=False)
        check('Escape cancels without changing pasted text', not state()['visible'] and target()['text'] == 'Keep the useful thought.')
        open_diagnostic()
        send('key', key=0xffe9, down=True)
        send('move', x=220, y=420)
        send('button', button=1, down=True)
        send('button', button=1, down=False)
        time.sleep(.5)
        send('key', key=0xffe9, down=False)
        check('held Alt rejects a paste', any(e['kind'] == 'paste-rejected' and e['reason'] == 'held-modifier'
              for e in state()['events']) and target()['text'] == 'Keep the useful thought.')
        send('move', x=1200, y=750)
        for key, down in [(0xffe3, True), (0xffe1, True)]:
            send('key', key=key, down=down)
        send('button', button=2, down=True)
        send('button', button=2, down=False)
        for key, down in [(0xffe1, False), (0xffe3, False)]:
            send('key', key=key, down=down)
        check('same mouse chord is observable over Shell desktop', state()['visible'])
        check('overlay visible before extension cleanup test', state()['visible'])
        (OUT / 'state.json').write_text(json.dumps(state(), indent=2) + '\n')
        (OUT / 'target.json').write_text(json.dumps(target(), indent=2) + '\n')
        subprocess.run(['gnome-extensions', 'disable', UUID], env=env, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        def extension_absent():
            return not bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'NameHasOwner', GLib.Variant('(s)', ('com.pasteroll.WaylandSpike',)),
                None, Gio.DBusCallFlags.NONE, 3000, None).unpack()[0]
        wait(extension_absent, 'test extension did not clean up its bus')
        check('disabling an open extension removes its test service', extension_absent())
        check('compositor and native client survive extension disable', shell.poll() is None and
              target()['display'] == 'GdkWaylandDisplay')
    finally:
        recording_stop.set()
        if recording_thread:
            recording_thread.join(timeout=5)
        (OUT / 'capture.json').write_text(json.dumps({'directory': os.environ.get('PASTEROLL_SPIKE_TAKE'),
            'frames': frame_samples, 'errors': recording_errors}, indent=2) + '\n')
        for name, getter in [('state', state), ('target', target)]:
            try:
                (OUT / f'{name}.json').write_text(json.dumps(getter(), indent=2) + '\n')
            except Exception:
                pass
        (OUT / 'results.json').write_text(json.dumps({'checks': checks,
            'full_mouse_workflow_passed': all(c['pass'] for c in checks),
            'required_checks_passed': all(c['pass'] for c in checks if c['required']),
            'recording_errors': recording_errors}, indent=2) + '\n')
        for proc in reversed(procs):
            stop(proc)

def main():
    assert os.environ.get('PASTEROLL_WAYLAND_SPIKE') == '1', 'use run.sh'
    marker = Path('/run/pasteroll-wayland-spike')
    assert marker.is_file() and marker.stat().st_uid == 0, 'private namespace marker is missing; use run.sh'
    OUT.mkdir(parents=True, exist_ok=True)
    if '--session' in sys.argv:
        assert os.environ.get('HOME', '').startswith('/tmp/pasteroll-wayland-'), 'private home required'
        session()
        return
    with tempfile.TemporaryDirectory(prefix='pasteroll-wayland-') as temporary:
        private = Path(temporary)
        env = dict(os.environ)
        take_name = 'take-' + time.strftime('%Y%m%dT%H%M%S', time.gmtime())
        take = OUT / take_name
        take.mkdir()
        env['PASTEROLL_SPIKE_OUTPUT'] = str(take)
        env['PASTEROLL_SPIKE_TAKE'] = take_name
        env.update(HOME=str(private / 'home'), XDG_DATA_HOME=str(private / 'data'),
                   XDG_CONFIG_HOME=str(private / 'config'), XDG_CACHE_HOME=str(private / 'cache'),
                   XDG_RUNTIME_DIR=str(private / 'runtime'), XDG_SESSION_TYPE='wayland',
                   XDG_CURRENT_DESKTOP='GNOME', GNOME_SHELL_SESSION_MODE='user',
                   DBUS_SYSTEM_BUS_ADDRESS='unix:path=/run/pasteroll-no-system-bus',
                   LIBGL_ALWAYS_SOFTWARE='1', GSETTINGS_BACKEND='dconf', NO_AT_BRIDGE='1')
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'DBUS_SESSION_BUS_ADDRESS'):
            env.pop(key, None)
        for key in ('HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            Path(env[key]).mkdir(mode=0o700, parents=True, exist_ok=True)
        extension = Path(env['XDG_DATA_HOME']) / 'gnome-shell/extensions' / UUID
        shutil.copytree(SOURCE / 'extension', extension)
        # A separate empty bus satisfies Shell's system-bus connection without
        # exposing the host's accounts, login manager, power or clipboard services.
        env['DBUS_SYSTEM_BUS_ADDRESS'] = 'unix:path=' + str(private / 'system-bus')
        private_system = launch(['dbus-daemon', '--session', '--nofork',
                                 '--address=' + env['DBUS_SYSTEM_BUS_ADDRESS']], env, 'private-system-bus')
        try:
            time.sleep(.3)
            proc = launch(['dbus-run-session', '--', '/usr/bin/python3', str(Path(__file__)), '--session'], env, 'session')
            result = proc.wait(timeout=120)
            for line in (OUT / 'session.log').read_text().splitlines():
                if line.startswith(('PASS ', 'FAIL ')):
                    print(line)
            if result:
                print('Experiment failed; see sanitized logs and results.json')
                raise SystemExit(result)
        finally:
            if 'proc' in locals():
                stop(proc)
            stop(private_system)

if __name__ == '__main__':
    try:
        main()
    except Exception as exc:
        print('Experiment error:', sanitize(exc))
        raise SystemExit(1)
