"""Real evdev/uinput helper + stock headless GNOME + real Pasteroll integration.

Headless Mutter does not represent a physical libinput seat. The driver reflects
synthetic keyboard/ordinary button input into Clutter separately, while testing
the actual kernel helper's suppression and D-Bus activation without substitution.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from gi.repository import Gio, GLib
import run as common

def session():
    procs=[]
    checks=[]
    errors=[]
    bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
    system=Gio.bus_get_sync(Gio.BusType.SYSTEM,None)
    def call(name,method,args=None,connection=bus,path=None,iface=None):
        return connection.call_sync(name,path or '/'+name.replace('.','/'),iface or name,
            method,args,None,Gio.DBusCallFlags.NONE,4000,None)
    def shell(method,args=None): return call('com.pasteroll.WaylandSpike',method,args)
    def state(): return json.loads(shell('State').unpack()[0])
    def fixture(method,args=None): return call('com.pasteroll.WaylandFixture',method,args)
    def native(): return json.loads(fixture('State').unpack()[0])
    def kernel(method,args=None): return call('com.pasteroll.InputFixture',method,args,connection=system)
    def output():
        # The observer is an independent process. Allow both the kernel worker and
        # its GLib reader to drain; this is not an input latency measurement.
        time.sleep(.08)
        return json.loads(kernel('Read').unpack()[0])
    def emit(device,ty,code,value):
        kernel('Emit',GLib.Variant('(suui)',(device,ty,code,value)))
        kernel('Emit',GLib.Variant('(suui)',(device,0,0,0)))
        time.sleep(.025)
    def wait(predicate,what,seconds=10):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                value=predicate()
                if value:return value
            except GLib.Error:pass
            time.sleep(.05)
        raise AssertionError(what)
    def check(name,value):
        checks.append({'name':name,'pass':bool(value)})
        print(('PASS ' if value else 'FAIL ')+name,flush=True)
        if not value:raise AssertionError(name)
    def key(code,down,device='keyboard'):
        ev={29:0xffe3,97:0xffe4,42:0xffe1,54:0xffe2,56:0xffe9,100:0xffea,125:0xffeb,126:0xffec}[code]
        emit(device,1,code,int(down))
        shell('Input',GLib.Variant('(s)',(json.dumps({'kind':'key','key':ev,'down':down}),)))
        time.sleep(.025)
    def tap(sym):
        for down in (True,False):
            shell('Input',GLib.Variant('(s)',(json.dumps({'kind':'key','key':sym,'down':down}),)))
            time.sleep(.08)
    def move(x=320,y=260):
        shell('Input',GLib.Variant('(s)',(json.dumps({'kind':'move','x':x,'y':y}),)))
        time.sleep(.15)
    def overlay():
        return next((a for a in state().get('namedActors',[]) if a.get('name')=='pasteroll-wayland-overlay' and a.get('visible')),None)
    def focus():
        shell('FocusWindow',GLib.Variant('(s)',('Pasteroll synthetic Wayland target',)))
        wait(lambda:(state().get('focus') or {}).get('title')=='Pasteroll synthetic Wayland target','target focus')
        time.sleep(.3)
    def chord(release_modifiers_first=False,mouse='mouse'):
        focus();move();output()
        key(29,True);key(54,True,'keyboard2')
        emit(mouse,1,274,1)
        if release_modifiers_first:
            key(29,False);key(54,False,'keyboard2')
        emit(mouse,1,274,0)
        if not release_modifiers_first:
            key(54,False,'keyboard2');key(29,False)
        return output()['events']
    try:
        env=dict(os.environ,WAYLAND_DISPLAY='pasteroll-test',GDK_BACKEND='wayland')
        for key_name,value in [('enabled-extensions',"['wayland-spike@pasteroll.com', 'pasteroll@pasteroll.com']"),('disable-user-extensions','false')]:
            subprocess.run(['gsettings','set','org.gnome.shell',key_name,value],env=env,check=True,stdout=subprocess.DEVNULL)
        procs.append(common.launch(['gnome-shell','--headless','--virtual-monitor','1280x800','--wayland','--no-x11','--sm-disable','--wayland-display=pasteroll-test'],env,'shell'))
        wait(state,'stock GNOME startup',45)
        check('stock GNOME compositor, no XWayland',state()['wayland'])
        procs.append(common.launch(['/usr/bin/python3',str(common.SOURCE/'fixture.py')],env,'fixture'))
        wait(native,'native target startup')
        check('native Wayland client',native()['display']=='GdkWaylandDisplay')
        focus();move()
        # Even the same UID cannot control the service from an arbitrary executable.
        denied=False
        try:
            call('com.pasteroll.Input','Poll',GLib.Variant('(bbtt)',(True,True,1,1)),connection=system,path='/com/pasteroll/Input',iface='com.pasteroll.Input1')
        except GLib.Error as exc:denied='AccessDenied' in str(exc)
        check('untrusted process denied by real root helper',denied)
        installed=Path(os.environ['HOME'])/'.local/bin/pasteroll'
        db=Path(os.environ['HOME'])/'mouse-history/history.db'
        db.parent.mkdir()
        db.with_name('first-run.txt').write_text('test acknowledgement\n')
        app=common.launch([str(installed),'run','--backend','gnome','--db',str(db)],env,'app-mouse')
        procs.append(app)
        def running():
            if app.poll() is not None:raise AssertionError('real mouse app exited; inspect sanitized app log')
            return (state().get('companion') or {}).get('mouseReady')
        wait(running,'mouse helper readiness',15)
        time.sleep(.5)
        devices=output()
        (common.OUT/'device-state.json').write_text(json.dumps(devices,indent=2)+'\n')
        check('real host helper acquired two synthetic mice',devices['outputs']==2)
        chord();wait(overlay,'empty history feedback')
        check('empty history gesture shows feedback without a paste session',not state()['companion']['inputActive'])
        wait(lambda:not overlay(),'empty history notification cleanup',5)
        # Clipboard provider needs an input serial in the actual native client.
        tap(0xffc5)
        fixture('Set',GLib.Variant('(sayb)',('text/plain;charset=utf-8',b'Pasteroll mouse integration clip',False)))
        time.sleep(.8)
        fixture('Clear',GLib.Variant('(u)',(0,)))
        events=chord(True)
        check('matching middle press and release suppressed in kernel output',not any(e[0]==1 and e[1]==274 for e in events))
        wait(overlay,'native mouse carousel')
        check('original gesture opens real Pasteroll carousel without keyboard opt-in',overlay() is not None)
        time.sleep(.3)
        shell('Snapshot',GLib.Variant('(s)',('snap-native-mouse.png',)))
        check('native client did not receive activation click',native()['windows'][0]['events']['button']==0)
        tap(0xff0d)
        wait(lambda:native()['windows'][0]['text']=='Pasteroll mouse integration clip','native paste')
        check('real clipboard/controller/paste path succeeds',not overlay())
        shell('Snapshot',GLib.Variant('(s)',('snap-native-paste.png',)))
        focus();output()
        for value in (1,0):emit('mouse',1,274,value)
        events=output()['events']
        check('ordinary middle-click forwarded as one intact pair',[e for e in events if e[:2]==[1,274]]==[[1,274,1],[1,274,0]])
        check('ordinary middle-click does not activate',not overlay())
        output();emit('mouse',2,0,13);emit('mouse',2,1,-7);emit('mouse',2,11,120);emit('mouse',2,8,1)
        events=output()['events']
        check('motion and high-resolution wheel preserved',all(e in events for e in [[2,0,13],[2,1,-7],[2,11,120],[2,8,1]]))
        for code in (29,42,56):key(code,True)
        output()
        for value in (1,0):emit('mouse',1,274,value)
        events=output()['events']
        for code in (56,42,29):key(code,False)
        check('extra Alt prevents gesture and preserves click',[1,274,1] in events and [1,274,0] in events and not overlay())
        output();emit('mouse',1,274,1);key(29,True);key(42,True);emit('mouse',1,274,0)
        events=output()['events'];key(42,False);key(29,False)
        check('modifiers pressed after button cannot claim release',[1,274,1] in events and [1,274,0] in events and not overlay())
        focus();output()
        kernel('Batch',GLib.Variant('(a(suui))',([
            ('keyboard',1,29,1),('keyboard2',1,42,1),('mouse',1,274,1),
            ('keyboard',1,29,0),('keyboard2',1,42,0),('mouse',1,274,0),
        ],)))
        wait(overlay,'rapid chord event ordering')
        events=output()['events']
        check('rapid cross-device chord survives queued modifier releases',not any(e[:2]==[1,274] for e in events))
        tap(0xff1b);wait(lambda:not overlay(),'rapid chord close')
        focus();key(29,True);key(42,True);output()
        emit('mouse',1,272,1)
        for value in (1,0):emit('mouse2',1,274,value)
        emit('mouse',1,272,0)
        events=output()['events'];key(42,False);key(29,False)
        check('drag on another mouse prevents activation and preserves both pairs',all(e in events for e in [[1,272,1],[1,272,0],[1,274,1],[1,274,0]]) and not overlay())
        events=chord(mouse='mouse2');wait(overlay,'second mouse activation')
        check('second mouse with separate keyboard modifiers activates',not any(e[:2]==[1,274] for e in events))
        tap(0xff1b);wait(lambda:not overlay(),'carousel close')
        kernel('Device',GLib.Variant('(sb)',('mouse2',False)))
        time.sleep(.6)
        kernel('Device',GLib.Variant('(sb)',('mouse2',True)))
        time.sleep(1)
        events=chord(mouse='mouse2');wait(overlay,'hotplug activation')
        check('hotplugged mouse activates without stale ownership',not any(e[:2]==[1,274] for e in events))
        tap(0xff1b);wait(lambda:not overlay(),'close after hotplug')
        for mouse in ('mouse','mouse2'):kernel('Device',GLib.Variant('(sb)',(mouse,False)))
        wait(lambda:output()['outputs']==0,'all mice disconnected')
        time.sleep(.2)
        check('losing the last mouse leaves the app running without a grab',app.poll() is None and not overlay())
        kernel('Device',GLib.Variant('(sb)',('mouse',True)))
        wait(running,'last mouse reconnected',15)
        chord();wait(overlay,'activation after last mouse reconnection')
        check('last mouse reconnection restores activation without restarting the app',app.poll() is None)
        tap(0xff1b);wait(lambda:not overlay(),'close after reconnection')
        kernel('Device',GLib.Variant('(sb)',('mouse2',True)))
        time.sleep(.7)
        subprocess.run(['gnome-extensions','disable','pasteroll@pasteroll.com'],env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        app.wait(timeout=8)
        check('companion disable ends the app and releases the lease',app.returncode!=0 and wait(lambda:output()['outputs']==0,'disabled companion lease'))
        subprocess.run(['gnome-extensions','enable','pasteroll@pasteroll.com'],env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        time.sleep(.5)
        app=common.launch([str(installed),'run','--backend','gnome','--db',str(db)],env,'app-extension-reloaded')
        procs.append(app)
        wait(running,'extension instance lease renewal',15)
        chord();wait(overlay,'activation after companion reload')
        check('companion reload acquires a fresh lease without restarting root helper',app.poll() is None)
        tap(0xff1b);wait(lambda:not overlay(),'close after companion reload')
        # Revocation while held must drain the claimed release and restore normal forwarding.
        focus();key(29,True);key(42,True);output();emit('mouse',1,274,1)
        wait(overlay,'held activation')
        subprocess.run([str(installed),'stop','--db',str(db)],env=env,check=True,stdout=subprocess.DEVNULL)
        app.wait(timeout=8)
        key(42,False);key(29,False);emit('mouse',1,274,0)
        events=output()['events']
        check('app stop while held consumes the owned release',not any(e[:2]==[1,274] for e in events))
        wait(lambda:output()['outputs']==0,'helper release after stop')
        check('helper releases all mouse grabs after app stop',not overlay())
        # The Shell is genuine; only its private logind fixture changes session policy.
        kernel('Policy',GLib.Variant('(bb)',(False,True)))
        refused=subprocess.run([str(installed),'run','--backend','gnome','--db',str(db)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=8)
        check('inactive or locked logind session cannot acquire devices',refused.returncode!=0 and output()['outputs']==0)
        kernel('Policy',GLib.Variant('(bb)',(True,False)))
        kernel('Fallback',GLib.Variant('(b)',(True,)))
        time.sleep(.3)
        app=common.launch([str(installed),'run','--backend','gnome','--db',str(db)],env,'app-recovery')
        procs.append(app)
        wait(running,'recovery after session policy denial',15)
        check('systemd user-manager Shell resolves the verified user display session',output()['outputs']==2)
        helper_pid=call('org.freedesktop.DBus','GetConnectionUnixProcessID',
            GLib.Variant('(s)',('com.pasteroll.Input',)),connection=system,
            path='/org/freedesktop/DBus',iface='org.freedesktop.DBus').unpack()[0]
        subprocess.run(['sudo','-n','kill','-KILL',str(helper_pid)],check=True)
        wait(lambda:output()['outputs']==0,'helper crash did not release devices')
        time.sleep(.2)
        check('helper crash releases devices and preserves the app',app.poll() is None and not overlay())
        restart_env=['sudo','-n','env','-i','PATH=/usr/bin:/bin','LANG=C.UTF-8',
                     'DBUS_SYSTEM_BUS_ADDRESS='+os.environ['DBUS_SYSTEM_BUS_ADDRESS']]
        procs.append(common.launch([*restart_env,str(common.HELPER_BINARY)],env,'input-service-restarted'))
        wait(running,'helper restart recovery',15)
        chord();wait(overlay,'activation after helper restart')
        check('authenticated helper restart restores activation without app restart',app.poll() is None)
        tap(0xff1b);wait(lambda:not overlay(),'close after helper restart')
        # A hung Shell must not leave root input interception alive indefinitely.
        shell_pid=procs[0].pid
        os.kill(shell_pid,signal.SIGSTOP)
        time.sleep(.8)
        check('expired Shell lease releases all device grabs',output()['outputs']==0)
        os.kill(shell_pid,signal.SIGCONT)
        wait(running,'Shell resumes and reacquires its lease',15)
        chord();wait(overlay,'activation after Shell resumes')
        check('resumed Shell restores mouse activation safely',app.poll() is None)
        tap(0xff1b);wait(lambda:not overlay(),'close after Shell recovery')
    except Exception as exc:
        try:(common.OUT/'failure-state.json').write_text(json.dumps(state(),indent=2)+'\n')
        except Exception:pass
        try:(common.OUT/'failure-fixture.json').write_text(json.dumps(native(),indent=2)+'\n')
        except Exception:pass
        errors.append(common.sanitize(exc))
        print('FAIL '+common.sanitize(exc),flush=True)
    finally:
        if procs:
            try:os.kill(procs[0].pid,signal.SIGCONT)
            except ProcessLookupError:pass
        for proc in reversed(procs):common.stop(proc)
        (common.OUT/'results.json').write_text(json.dumps({'checks':checks,'errors':errors,
            'passed':not errors and all(c['pass'] for c in checks),
            'limits':['Synthetic kernel devices and fake private logind; keyboard/native input reflected into headless Clutter. Physical libinput, GPU, real login/lock and mixed-DPI remain untested.']},indent=2)+'\n')
    if errors or not all(c['pass'] for c in checks):raise SystemExit(1)
