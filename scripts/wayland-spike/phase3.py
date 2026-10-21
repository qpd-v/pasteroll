"""Phase-three end-to-end suite for the real Pasteroll binary on GNOME Wayland.

The launcher supplies private mount/network/PID namespaces, HOME, XDG dirs, a
private session bus, an empty system bus and a headless Mutter compositor. This
module only drives synthetic input through the test-only extension and native
GTK4 fixtures. It never reads the live clipboard or connects to a host display.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import select
import signal
import struct
import subprocess
import time
import zlib
import run as common

OUT, SOURCE = common.OUT, common.SOURCE
WIDTH, HEIGHT = 520, 440
CTRL, SHIFT, ALT, SUPER = 0xffe3, 0xffe1, 0xffe9, 0xffeb
ESCAPE, ENTER, SPACE = 0xff1b, 0xff0d, 0x20
UP, DOWN, LEFT, RIGHT = 0xff52, 0xff54, 0xff51, 0xff53
PAGE_UP, PAGE_DOWN, HOME, END = 0xff55, 0xff56, 0xff50, 0xff57
F3, F11 = 0xffc0, 0xffc8
A, C, E, F, S = ord('a'), ord('c'), ord('e'), ord('f'), ord('s')
TARGET_A = 'Pasteroll synthetic Wayland target'
TARGET_B = 'Pasteroll synthetic Wayland target B'


class Blocked(Exception):
    """The harness could not establish its isolated environment."""


class CheckFailed(Exception):
    """A required end-to-end condition failed."""


def png_bytes(width=64, height=48):
    """A small deterministic RGBA PNG, encoded without third-party packages."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend((x * 4 % 256, y * 5 % 256, 160, 255))
    def chunk(kind, data):
        return (struct.pack('>I', len(data)) + kind + data
                + struct.pack('>I', zlib.crc32(kind + data)))
    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(rows), 9))
            + chunk(b'IEND', b''))


def png_size(data):
    if len(data) < 24 or not data.startswith(b'\x89PNG\r\n\x1a\n'):
        return None
    width, height = struct.unpack('>II', data[16:24])
    return width, height


def png_pixels(data):
    import gi
    gi.require_version('GdkPixbuf', '2.0')
    from gi.repository import GdkPixbuf
    loader = GdkPixbuf.PixbufLoader.new_with_type('png')
    loader.write(data)
    loader.close()
    picture = loader.get_pixbuf()
    return picture, bytes(picture.get_pixels())


def decode_bgra(value):
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    return bytes(value)


def session():
    from gi.repository import Gio, GLib

    procs = []
    checks = []
    screenshots = {}
    harness_errors = []
    limitations = []
    private_home = Path(os.environ['HOME'])
    installed = private_home / '.local/bin/pasteroll'
    built = common.APP_BINARY
    digest = hashlib.sha256(installed.read_bytes()).hexdigest() if installed.is_file() else ''
    built_digest = hashlib.sha256(built.read_bytes()).hexdigest() if built.is_file() else ''
    if not digest or digest != built_digest:
        raise Blocked('the private install is not the freshly built real pasteroll binary')

    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(name, method, params=None, path=None, interface=None):
        return bus.call_sync(name, path or '/' + name.replace('.', '/'),
                             interface or name, method, params, None,
                             Gio.DBusCallFlags.NONE, 8000, None)

    def shell(method, params=None):
        return call('com.pasteroll.WaylandSpike', method, params,
                    path='/com/pasteroll/WaylandSpike',
                    interface='com.pasteroll.WaylandSpike')

    def state():
        return json.loads(shell('State').unpack()[0])

    def fixture(method, params=None):
        return call('com.pasteroll.WaylandFixture', method, params,
                    path='/com/pasteroll/WaylandFixture',
                    interface='com.pasteroll.WaylandFixture')

    def fixture_state():
        return json.loads(fixture('State').unpack()[0])

    def wait(predicate, description, timeout=20):
        end = time.monotonic() + timeout
        error = None
        while time.monotonic() < end:
            try:
                result = predicate()
                if result:
                    return result
            except Exception as exc:
                error = exc
            time.sleep(.08)
        detail = f': {error}' if error else ''
        raise CheckFailed(description + detail)

    def record(name, ok, required=True, detail=''):
        checks.append({'name': name, 'pass': bool(ok), 'required': required,
                       'detail': common.sanitize(detail)})
        print(('PASS ' if ok else 'FAIL ') + name, flush=True)
        if not ok and required:
            raise CheckFailed(name + (': ' + detail if detail else ''))

    def input_event(kind, **values):
        shell('Input', GLib.Variant('(s)', (json.dumps({'kind': kind, **values}),)))

    def move(x, y, pause=.06):
        input_event('move', x=int(x), y=int(y))
        time.sleep(pause)

    def button(number=1, down=True, pause=.06):
        input_event('button', button=int(number), down=bool(down))
        time.sleep(pause)

    def click(x, y, number=1):
        move(x, y)
        button(number, True)
        button(number, False)

    def key(symbol, down=True, pause=.04):
        input_event('key', key=int(symbol), down=bool(down))
        time.sleep(pause)

    def tap(symbol, mods=()):
        for modifier in mods:
            key(modifier, True)
        key(symbol, True)
        key(symbol, False)
        for modifier in reversed(mods):
            key(modifier, False)

    def type_text(value):
        for character in value:
            tap(ord(character))

    def scroll(down=True):
        input_event('scroll', down=bool(down))
        time.sleep(.18)

    def snapshot(name):
        if not re.fullmatch(r'snap-[a-z0-9-]+', name):
            raise ValueError('snapshot name')
        shell('Snapshot', GLib.Variant('(s)', (name + '.png',)))
        path = Path(os.environ['PASTEROLL_SPIKE_OUTPUT']) / (name + '.png')
        data = path.read_bytes()
        screenshots[name] = {'file': path.relative_to(OUT).as_posix(),
                             'sha256': hashlib.sha256(data).hexdigest()}
        return data

    def focus(title):
        def acquire():
            shell('FocusWindow', GLib.Variant('(s)', (title,)))
            time.sleep(.08)
            return (state().get('focus') or {}).get('title', '') == title
        wait(acquire, f'{title} did not receive focus', 8)
        time.sleep(.2)

    def target_text(which=0):
        result = fixture_state()
        return result['windows'][which]['text']

    def clear_target(which=0):
        fixture('Clear', GLib.Variant('(u)', (which,)))
        time.sleep(.1)

    def set_text(which, value):
        fixture('SetText', GLib.Variant('(us)', (which, value)))
        time.sleep(.1)

    def set_clipboard(mime, data, sensitive=False):
        # A native Wayland offer needs a current input serial from this client.
        # In particular, a Shell dialog consumes serials the GTK fixture has not
        # seen. Deliver a harmless real key before asking GTK to own selection.
        if (state().get('focus') or {}).get('title') != TARGET_A:
            focus(TARGET_A)
        tap(0xffc5)  # F8, unused by the fixture's text view
        fixture('Set', GLib.Variant('(sayb)', (mime, data, sensitive)))
        time.sleep(.12)

    def set_clipboard_text(value, sensitive=False):
        set_clipboard('text/plain;charset=utf-8', value.encode(), sensitive)

    def read_clipboard(mime):
        return bytes(fixture('Read', GLib.Variant('(s)', (mime,))).unpack()[0])

    def actors():
        value = state()
        return value.get('namedActors', []) + [a for a in value.get('stage', []) if a.get('name')]

    def named(part):
        part = part.lower()
        for actor in actors():
            if part in (actor.get('name') or '').lower() and actor.get('visible', True):
                return actor
        return None

    def overlay():
        for part in ('overlay', 'carousel'):
            actor = named('pasteroll-wayland-' + part) or named('pasteroll-' + part)
            if actor:
                return actor
        for actor in actors():
            name = (actor.get('name') or '').lower()
            if 'pasteroll' in name and 'viewer' not in name and 'panel' not in name:
                width, height = (actor.get('rect') or [0, 0, 0, 0])[2:]
                if width >= 200 and height >= 150:
                    return actor
        return None

    def viewer_actor():
        for actor in actors():
            name = (actor.get('name') or '').lower()
            if 'pasteroll' in name and ('viewer' in name or 'editor' in name):
                return actor
        return None

    def activation():
        tap(SPACE, (CTRL, SHIFT))

    def open_carousel(title=TARGET_A, x=320, y=260):
        focus(title)
        move(x, y)
        activation()
        actor = wait(overlay, 'the real Pasteroll carousel did not open', 8)
        time.sleep(.45)
        return actor

    def close_carousel():
        for _ in range(3):
            if overlay() is None and viewer_actor() is None:
                return
            tap(ESCAPE)
            time.sleep(.45)
        wait(lambda: overlay() is None, 'the carousel did not close', 6)

    def panel_tree(value=None):
        value = value or state()
        result = []
        def walk(items):
            for item in items or []:
                result.append(item)
                walk(item.get('children'))
        for entry in (value.get('panel') or {}).values():
            result.append(entry)
            walk(entry.get('children'))
        return result

    def find_label(needle, value=None):
        needle = needle.lower()
        for item in panel_tree(value):
            if needle in (item.get('text') or '').lower():
                return item
        return None

    def ui_items(value=None):
        value = value or state()
        result = list(panel_tree(value))
        def walk(items):
            for item in items or []:
                result.append(item)
                walk(item.get('children'))
        for actor in value.get('stage', []):
            result.append(actor)
            walk(actor.get('children'))
        for actor in value.get('namedActors', []):
            result.append(actor)
        return result

    def find_ui(needle, exact=False):
        needle = needle.lower()
        for item in ui_items():
            text = (item.get('text') or '').lower()
            if ((text == needle if exact else needle in text)
                    and item.get('visible', True) and item.get('mapped', True)):
                return item
        return None

    def visible_text():
        return [item.get('text') for item in ui_items() if item.get('text')]

    def nested_ui():
        value = state()
        out = []
        def walk(item, path):
            out.append((item, path))
            for child in item.get('children') or []:
                walk(child, path + [item])
        for root in value.get('stage', []):
            walk(root, [])
        return value, out

    def descendants(item):
        for child in item.get('children') or []:
            yield child
            yield from descendants(child)

    def click_control_for(title, text=None, accessible=None):
        for _ in range(20):
            value, items = nested_ui()
            for item, path in items:
                if ((item.get('text') or '').casefold() != title.casefold()
                        or 'pasteroll-setting-title' not in (item.get('style') or '').split()
                        or not item.get('mapped', True)):
                    continue
                for ancestor in reversed(path):
                    if 'pasteroll-setting-row' not in (ancestor.get('style') or '').split():
                        continue
                    for candidate in descendants(ancestor):
                        if text is not None and (candidate.get('text') or '') == text:
                            if not candidate.get('mapped', True):
                                continue
                            click_actor(candidate)
                            return candidate
                        if accessible is not None and (candidate.get('accessible') or '') == accessible:
                            click_actor(candidate)
                            return candidate
                    break
            time.sleep(.15)
        raise CheckFailed(f'no native control found for {title!r}')

    def panel_button():
        value = state()
        for key, entry in (value.get('panel') or {}).items():
            if key.lower() == 'pasteroll' or 'pasteroll' in (entry.get('text') or '').lower() or \
                    'pasteroll' in (entry.get('name') or '').lower():
                return entry
        return None

    def open_panel_menu():
        button = wait(panel_button, 'the native Pasteroll panel item is absent', 8)
        click_actor(button)
        wait(lambda: find_ui('Quit Pasteroll'), 'the native Pasteroll menu did not open', 6)

    def click_panel_item(label):
        open_panel_menu()
        item = wait(lambda: find_ui(label), f'panel item {label!r} is absent', 6)
        click_actor(item)
        time.sleep(.35)

    def open_settings():
        click_panel_item('Settings')
        wait(lambda: find_ui('Pasteroll settings'), 'the native settings dialog did not open', 8)
        return wait(lambda: find_ui('Save changes'), 'settings did not expose Save changes', 6)

    def click_actor(actor):
        x, y, w, h = actor['rect']
        click(x + w / 2, y + h / 2)
        time.sleep(.35)

    def status(app):
        return app.status()

    def config_value(path, key):
        match = re.search(rf'^\s*{re.escape(key)}\s*=\s*([^#\r\n]+)', path.read_text(), re.M)
        return match.group(1).strip() if match else None

    class App:
        def __init__(self, name, config='', env_extra=None, keyboard=True, first_run=False):
            self.name = name
            self.dir = private_home / 'data' / name
            self.dir.mkdir(parents=True, exist_ok=True)
            self.db = self.dir / 'history.db'
            self.config = self.dir / 'config.toml'
            self.config.write_text(config)
            if not first_run:
                (self.dir / 'first-run.txt').write_text('test acknowledgement\n')
            self.proc = None
            self.extra = env_extra or {}
            self.keyboard = keyboard

        def start(self):
            app_env = dict(os.environ)
            app_env.update(self.extra)
            args = [str(installed), 'run', '--db', str(self.db), '--backend', 'gnome']
            if self.keyboard:
                args.append('--wayland-keyboard')
            self.proc = common.launch(args, app_env, 'app-' + self.name)
            procs.append(self.proc)
            end = time.monotonic() + 12
            while time.monotonic() < end:
                if self.proc.poll() is not None:
                    log = OUT / f'app-{self.name}.log'
                    detail = log.read_text()[-2000:] if log.exists() else ''
                    raise Blocked(f'the real Pasteroll process exited during start: {detail}')
                if 'running with the native GNOME Wayland backend' in self.log():
                    time.sleep(.6)
                    return self
                time.sleep(.1)
            raise Blocked('the real Pasteroll process did not become ready')

        def log(self):
            path = OUT / f'app-{self.name}.log'
            return path.read_text(errors='replace') if path.exists() else ''

        def status(self):
            out = subprocess.run([str(installed), 'status', '--db', str(self.db)],
                                 env=os.environ, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, timeout=8)
            text = out.stdout.decode('utf-8', 'replace')
            match = re.search(r'stored:\s+(\d+) clips,\s+(\d+) pinned', text)
            return {'code': out.returncode,
                    'clips': int(match.group(1)) if match else None,
                    'pins': int(match.group(2)) if match else None,
                    'text': text, 'error': out.stderr.decode('utf-8', 'replace')}

        def wait_clips(self, count, timeout=8):
            return wait(lambda: self.status().get('clips') == count,
                        f'history did not reach {count} clip(s); got {self.status().get("clips")}', timeout)

        def wait_growth(self, count, timeout=8):
            return wait(lambda: (self.status().get('clips') or 0) >= count,
                        f'history did not reach {count} clip(s); got {self.status().get("clips")}', timeout)

        def stop(self, graceful=True, timeout=8):
            if self.proc is None or self.proc.poll() is not None:
                return self.proc.returncode if self.proc else None
            if graceful:
                try:
                    subprocess.run([str(installed), 'stop', '--db', str(self.db)],
                                   env=os.environ, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=4)
                except subprocess.TimeoutExpired:
                    pass
            try:
                self.proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                common.stop(self.proc)
            return self.proc.returncode

    def bootstrap_clipboard():
        set_clipboard_text('Sensitive bootstrap marker', sensitive=True)

    # ---------------------------------------------------------------- environment
    try:
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'enabled-extensions',
                        "['wayland-spike@pasteroll.com', 'pasteroll@pasteroll.com']"],
                       env=os.environ, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['gsettings', 'set', 'org.gnome.shell', 'disable-user-extensions', 'false'],
                       env=os.environ, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['gsettings', 'set', 'org.gnome.desktop.interface', 'enable-animations', 'false'],
                       env=os.environ, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        env = dict(os.environ)
        env.update(WAYLAND_DISPLAY='pasteroll-test', GDK_BACKEND='wayland')
        shell_proc = common.launch(['gnome-shell', '--headless', '--virtual-monitor', '1280x800',
                                    '--wayland', '--no-x11', '--sm-disable',
                                    '--wayland-display=pasteroll-test'], env, 'shell')
        procs.append(shell_proc)
        first = wait(state, 'the isolated GNOME test extension did not start', 45)
        record('stock GNOME 46 Wayland with XWayland disabled',
               first.get('wayland') is True and first.get('monitor', [None])[4] == 1.0,
               detail=json.dumps({'wayland': first.get('wayland'), 'monitor': first.get('monitor')}))

        env['WAYLAND_DISPLAY'] = 'pasteroll-test'
        env['GDK_BACKEND'] = 'wayland'
        env.pop('DISPLAY', None)
        fixture_proc = common.launch(['/usr/bin/python3', str(SOURCE / 'fixture.py')], env, 'fixture')
        procs.append(fixture_proc)
        native = wait(fixture_state, 'the native GTK4 fixture did not start', 15)
        record('native GTK4 MIME provider and two consumers', native['display'] == 'GdkWaylandDisplay',
               detail=native['display'])
        focus(TARGET_A)
        fixture_exe = Path(os.readlink(f'/proc/{state()["focus"]["pid"]}/exe')).name.lower()
        bootstrap_clipboard()

        refused_db = private_home / 'not-authorized/history.db'
        for options in ([], ['--backend', 'gnome']):
            refused = subprocess.run([str(installed), 'run', '--db', str(refused_db), *options],
                                     env=os.environ, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, timeout=8)
            record('native launch refuses missing keyboard opt-in' + (' (explicit backend)' if options else ' (auto)'),
                   refused.returncode != 0 and not refused_db.exists() and
                   b'--wayland-keyboard' in refused.stderr)

        # The real bridge command handshake before any long-running controller owns the peer.
        wire = subprocess.run([str(installed), 'wayland-check'], env=os.environ,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        wire_text = (wire.stdout + wire.stderr).decode('utf-8', 'replace')
        record('real binary completes Bridge2 handshake',
               wire.returncode == 0 and 'protocol 3' in wire_text and 'global-mouse=false' in wire_text,
               detail=wire_text[-1200:])
        record('Bridge2 reports paste, viewer and settings capabilities',
               all(token in wire_text for token in ('paste=true', 'viewer=true', 'settings=true')))
        time.sleep(.4)

        config = '''[limits]
max_clips = 1000
max_total_mb = 500
max_age_days = 90
max_clip_mb = 16
max_pins = 200

[capture]
paused = false
exclude_apps = []

[paste]
warn_multiline_in_terminals = true
terminal_apps = []

[appearance]
size = 100
'''
        quiet = {'PASTEROLL_NO_NOTICE': '1'}

        # ------------------------------------------------------------ first-run acknowledgement
        notice = App('notice', config, quiet, first_run=True).start()
        marker = notice.db.with_name('first-run.txt')
        time.sleep(1.0)
        record('first-run state alone does not acknowledge the notice', not marker.exists())
        welcome = wait(lambda: find_ui('Welcome to Pasteroll'),
                       'the native first-run notice did not appear', 12)
        snapshot('snap-first-run')
        candidates = [find_ui(text, exact=True) for text in
                      ('Acknowledge', 'Continue', 'Got it', 'I understand', 'OK')]
        acknowledgement = next((item for item in candidates if item), None)
        record('first-run notice exposes a real acknowledgement control', acknowledgement is not None,
               detail=json.dumps(visible_text()[-20:]))
        if acknowledgement:
            click_actor(acknowledgement)
            time.sleep(.6)
            # The companion reports kind 25 only from the real control.
            wait(lambda: marker.exists(), 'kind 25 did not mark the first-run notice done', 10)
            record('real first-run acknowledgement sends kind 25 and is recorded', marker.exists())
        notice.stop(graceful=True)

        # ------------------------------------------------------------ capture, search, paste
        app = App('core', config, quiet).start()
        try:
            Path(f'/proc/{app.proc.pid}/exe').readlink()
            hardened = False
        except PermissionError:
            hardened = True
        record('real app is hardened after peer authentication', hardened)
        bootstrap_clipboard()
        text_alpha = 'ALPHA marker: the useful earlier synthetic clip'
        text_beta = 'BETA marker: the newest synthetic clip'
        set_clipboard_text(text_alpha)
        app.wait_clips(1)
        set_clipboard_text(text_beta)
        app.wait_clips(2)
        record('native clipboard changes are captured in newest-first order',
               app.status().get('clips') == 2, detail=json.dumps(app.status().get('text')))
        focus(TARGET_A)
        clear_target()
        open_carousel()
        record('explicit --wayland-keyboard opens the real carousel', overlay() is not None)
        tap(ENTER)
        wait(lambda: target_text() == text_beta, 'the newest clip was not pasted', 8)
        record('real GNOME paste writes a fixed Ctrl+V chord', target_text() == text_beta)
        record('carousel closes after the paste', overlay() is None)

        focus(TARGET_A)
        clear_target()
        open_carousel()
        before_clicks = fixture_state()['windows'][0]['events']['button']
        click(160, 220)  # Real native client area, outside the carousel.
        wait(lambda: target_text() == text_beta, 'a physical click did not paste into the native target', 8)
        record('physical click is replayed once and delivers the selected clip',
               target_text() == text_beta and
               fixture_state()['windows'][0]['events']['button'] == before_clicks + 1)

        focus(TARGET_A)
        clear_target()
        open_carousel()
        type_text('alpha')
        tap(ENTER)
        wait(lambda: target_text() == text_alpha, 'search did not paste the ALPHA clip', 8)
        record('search changes the common controller selection and pastes the match',
               target_text() == text_alpha)

        focus(TARGET_A)
        clear_target()
        open_carousel()
        scroll(True)
        tap(ENTER)
        wait(lambda: len(target_text()) > 0, 'wheel step did not paste', 8)
        record('real wheel direction selects the next clip',
               target_text() == text_beta)

        partial_source = 'SELECT marker: select only the beginning of this synthetic clip'
        set_clipboard_text(partial_source)
        app.wait_clips(3)
        focus(TARGET_A)
        clear_target()
        carousel = open_carousel()
        snapshot('snap-carousel-before-selection')
        ox, oy = carousel['rect'][0], carousel['rect'][1]
        sx, sy = ox + 30, oy + 140
        ex, ey = ox + 155, oy + 140
        move(ox + 260, oy + 240)
        move(sx, sy)
        button(1, True)
        for x in range(sx + 18, ex + 1, 18):
            move(x, sy)
        button(1, False)
        snapshot('snap-carousel-selected')
        tap(ENTER)
        wait(lambda: len(target_text()) > 0, 'a dragged text selection was not pasted', 8)
        partial = target_text()
        record('drag-selected partial text is pasted instead of the whole clip',
               partial and len(partial) < len(partial_source) and partial_source.startswith(partial),
               detail=json.dumps({'selected_chars': len(partial), 'whole_chars': len(partial_source)}))

        # Expanded viewer: a long clip, real wheel scrolling and find.
        lines = ['Synthetic long clip line %02d' % i for i in range(60)]
        lines[45] = 'Synthetic long clip line 45 needlefox marker'
        long_text = '\n'.join(lines)
        set_clipboard_text(long_text)
        app.wait_clips(4)
        focus(TARGET_A)
        clear_target()
        open_carousel()
        tap(E, (CTRL,))
        wait(viewer_actor, 'the real expanded viewer did not open', 10)
        time.sleep(1.2)
        before = snapshot('snap-viewer-top')
        bounds = viewer_actor()['rect']
        move(bounds[0] + bounds[2] / 2, bounds[1] + bounds[3] / 2)
        scroll(True)
        time.sleep(.3)
        scrolled = snapshot('snap-viewer-scrolled')
        record('expanded viewer wheel changes the rendered content',
               hashlib.sha256(before).digest() != hashlib.sha256(scrolled).digest())
        record('viewer wheel is swallowed instead of reaching the native target',
               fixture_state()['windows'][0]['events']['scroll'] == 0)
        tap(F, (CTRL,))
        type_text('needlefox')
        time.sleep(.5)
        found = snapshot('snap-viewer-find')
        record('viewer find updates a real rendered frame',
               hashlib.sha256(found).digest() != hashlib.sha256(scrolled).digest())
        tap(ENTER)  # next match while the find field owns Enter
        tap(ESCAPE)  # close find, keep the viewer
        record('find field consumes Enter without pasting', target_text() == '')
        tap(ENTER)
        wait(lambda: target_text() == long_text, 'viewer did not paste the whole clip after find closed', 8)
        record('viewer closes find first and then pastes through the real bridge',
               target_text() == long_text)
        app.stop(graceful=True)
        record('graceful stop exits the real controller', app.proc.returncode == 0,
               detail=str(app.proc.returncode))

        # ------------------------------------------------------------ terminal warning and rejection
        warn_config = config.replace('terminal_apps = []', f'terminal_apps = ["{fixture_exe}"]')
        app = App('warning', warn_config, quiet).start()
        bootstrap_clipboard()
        multiline = 'synthetic line one\nsynthetic line two'
        set_clipboard_text(multiline)
        app.wait_clips(1)
        focus(TARGET_A)
        clear_target()
        open_carousel()
        tap(ENTER)
        time.sleep(.8)
        record('multiline paste into a configured terminal warns without pasting',
               target_text() == '' and overlay() is not None)
        tap(ENTER)
        wait(lambda: target_text() == multiline, 'second Enter did not paste after the warning', 8)
        record('second Enter accepts the terminal warning', target_text() == multiline)

        wrong = 'WRONGTARGET synthetic clip'
        set_clipboard_text(wrong)
        app.wait_clips(2)
        focus(TARGET_A)
        clear_target(0)
        clear_target(1)
        open_carousel()
        focus(TARGET_B)
        wait(lambda: overlay() is None, 'a changed foreground did not cancel the session', 5)
        record('foreground change rejects the paste and leaves both targets unchanged',
               target_text(0) == '' and target_text(1) == '' and
               'foreground changed' in app.log())
        close_carousel()

        focus(TARGET_A)
        clear_target()
        open_carousel()
        key(ALT, True)
        tap(ENTER)
        time.sleep(.6)
        record('held Alt cancels the paste instead of sending a changed chord',
               target_text() == '' and overlay() is None)
        key(ALT, False)
        time.sleep(.2)

        focus(TARGET_A)
        clear_target()
        open_carousel()
        tap(ESCAPE)
        record('Escape cancels without pasting', overlay() is None and target_text() == '')
        app.stop(graceful=True)

        # ------------------------------------------------------------ editor save and preservation
        app = App('editor', config, quiet).start()
        bootstrap_clipboard()
        image = png_bytes()
        set_clipboard('image/png', image)
        app.wait_clips(1)
        before_editor = app.status().get('clips')
        focus(TARGET_A)
        open_carousel()
        move(1160, 710)
        key(CTRL, True)
        click(1160, 710, 1)
        key(CTRL, False)
        editor = wait(lambda: viewer_actor(), 'the Ctrl+desktop image editor did not open', 12)
        time.sleep(1.2)
        snapshot('snap-image-editor')
        before_clicks = fixture_state()['windows'][0]['events']['button']
        click(90, 300)
        record('image editor swallows outside clicks without closing or replaying',
               viewer_actor() is not None and
               fixture_state()['windows'][0]['events']['button'] == before_clicks)
        bounds = viewer_actor()['rect']
        click(bounds[0] + bounds[2] - 40, bounds[1] + 191)  # Rounded corners switch.
        time.sleep(.45)
        snapshot('snap-image-editor-rounded')
        tap(ENTER)
        desktop = private_home / 'Desktop'
        saved = wait(lambda: list(desktop.glob('Pasteroll image *.png')),
                     'the edited image was not saved to the private Desktop', 15)
        saved_data = saved[-1].read_bytes()
        record('image editor saves a real PNG to the private Desktop',
               png_size(saved_data) == (64, 48), detail=str(png_size(saved_data)))
        decoded, pixels = png_pixels(saved_data)
        record('editor mouse controls change the saved image pixels',
               decoded.get_has_alpha() and pixels[3] == 0 and saved_data != image)
        record('saving the edited image preserves the original history entry',
               app.status().get('clips') == before_editor)
        # File creation precedes the worker reply and the editor's closing
        # animation. Do not send activation while the modal editor still owns it.
        wait(lambda: 'image editor closed' in app.log() and viewer_actor() is None and overlay() is None,
             'the saved editor did not finish closing', 10)
        focus(TARGET_A)
        open_carousel()
        tap(E, (CTRL,))
        wait(viewer_actor, 'the saved image history entry did not reopen in the viewer', 10)
        time.sleep(1)
        tap(C, (CTRL,))
        # Wayland offers the new clipboard to the native client after the Shell
        # modal releases focus. This observes the user's real copy-then-return path.
        close_carousel()
        focus(TARGET_A)
        wait(lambda: read_clipboard('image/png').startswith(b'\x89PNG'), 'viewer image copy did not reach GTK4', 8)
        copied = read_clipboard('image/png')
        record('the original image remains available after editor save',
               copied == image,
               detail=json.dumps({'sha256': hashlib.sha256(copied).hexdigest(),
                                  'source_sha256': hashlib.sha256(image).hexdigest()}))
        close_carousel()
        app.stop(graceful=True)

        # ------------------------------------------------------------ capture pause and exclusion
        paused = App('paused', config.replace('paused = false', 'paused = true'), quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('PAUSED should not be recorded')
        time.sleep(1.2)
        record('paused capture records no native clipboard change',
               paused.status().get('clips') == 0, detail=json.dumps(paused.status().get('text')))
        paused.stop(graceful=True)

        excluded = App('excluded', config.replace('exclude_apps = []',
                                                  f'exclude_apps = ["{fixture_exe}"]'), quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('EXCLUDED source should not be recorded')
        time.sleep(1.2)
        record('per-app exclusion records no native clipboard change',
               excluded.status().get('clips') == 0, detail=json.dumps(excluded.status().get('text')))
        excluded.stop(graceful=True)

        # ------------------------------------------------------------ native settings and autostart
        settings_app = App('settings', config, quiet).start()
        bootstrap_clipboard()
        for i in range(101):
            set_clipboard_text('SETTINGS synthetic clip %03d' % i)
            settings_app.wait_clips(i + 1, 4)
        record('settings fixture history reaches one hundred and one clips',
               settings_app.status().get('clips') == 101)
        # Simulate a custom filter saved by the common carousel after startup.
        with settings_app.config.open('a') as saved_config:
            saved_config.write('\n[[filters]]\nname = "Synthetic design"\nextensions = ["psd", "ai"]\n')
        open_settings()
        for _ in range(3):
            click_control_for('Unpinned clips', accessible='Decrease')
        click_actor(wait(lambda: find_ui('Save changes'), 'Save changes disappeared', 6))
        wait(lambda: find_ui('Remove clips and save?'),
             'lowering the history limit did not ask for confirmation', 10)
        record('destructive settings proposal is not written before confirmation',
               config_value(settings_app.config, 'max_clips') == '1000',
               detail=str(config_value(settings_app.config, 'max_clips')))
        snapshot('snap-settings-confirm')
        click_actor(wait(lambda: find_ui('Cancel', exact=True), 'confirmation has no Cancel', 6))
        wait(lambda: find_ui('Remove clips and save?') is None,
             'Cancel did not close the destructive confirmation', 6)
        time.sleep(.6)
        record('cancelled settings confirmation changes nothing',
               config_value(settings_app.config, 'max_clips') == '1000' and
               settings_app.status().get('clips') == 101)

        open_settings()
        for _ in range(3):
            click_control_for('Unpinned clips', accessible='Decrease')
        click_actor(wait(lambda: find_ui('Save changes'), 'Save changes disappeared', 6))
        wait(lambda: find_ui('Remove clips and save?'),
             'the second settings proposal did not ask for confirmation', 10)
        runs = settings_app.log().count('running with the native GNOME Wayland backend')
        click_actor(wait(lambda: find_ui('Remove and save', exact=True), 'confirmation has no apply button', 6))
        wait(lambda: config_value(settings_app.config, 'max_clips') == '100',
             'confirmed settings were not written', 20)
        wait(lambda: settings_app.status().get('clips') == 100,
             'confirmed prune did not leave exactly one hundred clips', 20)
        wait(lambda: settings_app.log().count('running with the native GNOME Wayland backend') > runs,
             'the real controller did not restart after applying settings', 12)
        record('confirmed settings apply the proposed limit and prune',
               config_value(settings_app.config, 'max_clips') == '100' and
               settings_app.status().get('clips') == 100)
        record('settings keep a custom filter saved after process startup',
               'Synthetic design' in settings_app.config.read_text() and
               'extensions = ["psd", "ai"]' in settings_app.config.read_text())

        autostart = Path(os.environ['XDG_CONFIG_HOME']) / 'autostart/pasteroll.desktop'
        runs = settings_app.log().count('running with the native GNOME Wayland backend')
        open_settings()
        snapshot('snap-autostart-before')
        click_control_for('Start when I sign in', text='Off')
        snapshot('snap-autostart-after')
        click_actor(wait(lambda: find_ui('Save changes'), 'Save changes disappeared', 6))
        wait(lambda: autostart.is_file(), 'settings did not create the private autostart entry', 15)
        wait(lambda: settings_app.log().count('running with the native GNOME Wayland backend') > runs,
             'the controller did not restart after enabling autostart', 12)
        entry = autostart.read_text()
        record('autostart stays explicit about Wayland and keyboard opt-in',
               'run --backend gnome --wayland-keyboard' in entry and
               'X-Pasteroll-Backend=gnome-wayland' in entry)
        runs = settings_app.log().count('running with the native GNOME Wayland backend')
        open_settings()
        click_control_for('Start when I sign in', text='On')
        click_actor(wait(lambda: find_ui('Save changes'), 'Save changes disappeared', 6))
        wait(lambda: not autostart.exists(), 'settings did not remove its own autostart entry', 15)
        wait(lambda: settings_app.log().count('running with the native GNOME Wayland backend') > runs,
             'the controller did not restart after disabling autostart', 12)
        record('disabling autostart removes only the application entry', not autostart.exists())

        click_panel_item('Pause recording')
        wait(lambda: config_value(settings_app.config, 'paused') == 'true',
             'the panel pause action did not persist', 6)
        pause_count = settings_app.status().get('clips')
        set_clipboard_text('PAUSED VIA PANEL must not be captured')
        time.sleep(1.2)
        record('panel pause stops real clipboard capture',
               settings_app.status().get('clips') == pause_count)
        click_panel_item('Resume recording')
        wait(lambda: config_value(settings_app.config, 'paused') == 'false',
             'the panel resume action did not persist', 6)
        set_clipboard_text('RESUMED VIA PANEL is captured')
        # The existing store batches automatic pruning with bounded slack;
        # a newly captured clip can temporarily bring this count to 101.
        settings_app.wait_clips(pause_count + 1, 6)
        focus(TARGET_A)
        clear_target()
        open_carousel()
        tap(ENTER)
        wait(lambda: target_text() == 'RESUMED VIA PANEL is captured',
             'resumed clip did not become the newest entry', 8)
        record('panel resume restores real clipboard capture',
               target_text() == 'RESUMED VIA PANEL is captured' and
               settings_app.status().get('clips') == pause_count + 1)
        click_panel_item('Quit Pasteroll')
        settings_app.proc.wait(timeout=8)
        record('panel quit asks the real controller to exit', settings_app.proc.returncode == 0,
               detail=str(settings_app.proc.returncode))

        # ------------------------------------------------------------ lifecycle
        lifecycle = App('lifecycle', config, quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('LIFECYCLE synthetic clip')
        lifecycle.wait_clips(1)
        open_carousel()
        lifecycle.stop(graceful=True)
        record('graceful stop releases the companion and exits cleanly',
               lifecycle.proc.returncode == 0 and wait(lambda: overlay() is None,
                                                       'overlay remained after stop', 5) is not False)

        lifecycle = App('lifecycle-disable', config, quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('DISABLE synthetic clip')
        lifecycle.wait_clips(1)
        open_carousel()
        subprocess.run(['gnome-extensions', 'disable', 'pasteroll@pasteroll.com'],
                       env=os.environ, check=True, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL)
        wait(lambda: overlay() is None, 'extension disable left the carousel visible', 8)
        record('disabling the production extension tears down its carousel', overlay() is None)
        wait(lambda: lifecycle.proc.poll() is not None, 'the app did not exit after companion loss', 8)
        record('companion loss stops the real app with an error', lifecycle.proc.returncode != 0)
        lifecycle.stop(graceful=False)

        subprocess.run(['gnome-extensions', 'enable', 'pasteroll@pasteroll.com'],
                       env=os.environ, check=True, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL)
        time.sleep(.8)
        lifecycle = App('lifecycle-disconnect', config, quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('DISCONNECT synthetic clip')
        lifecycle.wait_clips(1)
        open_carousel()
        os.kill(lifecycle.proc.pid, signal.SIGKILL)
        lifecycle.proc.wait(timeout=5)
        wait(lambda: overlay() is None, 'owner loss left the carousel visible', 8)
        record('client disconnect removes the companion UI', overlay() is None)

        subprocess.run(['gnome-extensions', 'enable', 'pasteroll@pasteroll.com'],
                       env=os.environ, check=True, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL)
        time.sleep(.8)
        lifecycle = App('lifecycle-watchdog', config, quiet).start()
        bootstrap_clipboard()
        set_clipboard_text('WATCHDOG synthetic clip')
        lifecycle.wait_clips(1)
        open_carousel()
        os.kill(lifecycle.proc.pid, signal.SIGSTOP)
        time.sleep(3.8)
        record('companion watchdog releases a stalled client UI', overlay() is None)
        os.kill(lifecycle.proc.pid, signal.SIGCONT)
        time.sleep(.3)
        open_carousel()
        record('a resumed app starts a fresh session after watchdog revocation', overlay() is not None)
        close_carousel()
        lifecycle.stop(graceful=False)

        fixture('Harden')
        unidentified = App('unidentified', config.replace('exclude_apps = []',
                               f'exclude_apps = ["{fixture_exe}"]'), quiet).start()
        set_clipboard_text('UNIDENTIFIED source must not bypass exclusions')
        time.sleep(.8)
        source_state = state()['companion']['snapshot']
        record('unidentified hardened source cannot bypass configured exclusions',
               source_state[2] == '' and source_state[3] and
               unidentified.status().get('clips') == 0)
        unidentified.stop(graceful=True)

        limitations.extend([
            'The suite uses one 1280x800 software-rendered virtual monitor at scale 1.0; mixed-DPI and physical GPU behavior are untested.',
            'GNOME 46 only. No newer GNOME Shell or non-GNOME Wayland compositor was exercised.',
            'No physical lock screen, GDM, multiple monitors, or hardware input device was used.',
            'Settings confirmation is covered and reported separately by its checks in this file.',
        ])
    except CheckFailed as exc:
        if not checks or checks[-1].get('pass'):
            harness_errors.append(common.sanitize(exc))
        print('Required check failed:', common.sanitize(exc), flush=True)
    except Exception as exc:
        harness_errors.append(common.sanitize(exc))
        print('Harness error:', common.sanitize(exc), flush=True)
    finally:
        if harness_errors or any(not item['pass'] for item in checks if item['required']):
            try:
                (OUT / 'failure-state.json').write_text(json.dumps(state(), indent=2) + '\n')
                (OUT / 'failure-fixture.json').write_text(json.dumps(fixture_state(), indent=2) + '\n')
                snapshot('snap-failure')
            except Exception:
                pass
        for proc in reversed(procs):
            try:
                common.stop(proc)
            except Exception:
                pass
        passed = all(item['pass'] for item in checks if item['required'])
        report = {
            'checks': checks,
            'harness_errors': harness_errors,
            'limitations': limitations,
            'real_binary_sha256': digest,
            'full_e2e_passed': passed and not harness_errors,
            'startup_entries': [p.relative_to(private_home.parent).as_posix()
                                for p in private_home.parent.rglob('pasteroll.desktop')],
        }
        (OUT / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
        (OUT / 'screenshots.json').write_text(json.dumps(screenshots, indent=2) + '\n')
    if harness_errors or not passed:
        raise SystemExit(1)
