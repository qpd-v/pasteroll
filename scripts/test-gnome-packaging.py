#!/usr/bin/env python3
"""Verify development packages in disposable roots; never enable a live service."""
import hashlib
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

ROOT=Path(__file__).resolve().parents[1]

class GnomePackages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.version=tomllib.loads((ROOT/'crates/pasteroll-linux/Cargo.toml').read_text())['package']['version']
        cls.dist=Path(os.environ.get('PASTEROLL_PACKAGE_OUTPUT',str(ROOT/'target-dev/wayland-mouse/packages'))).resolve()
        cls.tar=cls.dist/f'pasteroll-{cls.version}-linux-x86_64-gnome-development.tar.gz'
        cls.deb=cls.dist/f'pasteroll_{cls.version}_amd64-gnome-development.deb'
        assert cls.tar.is_file() and cls.deb.is_file(),'build the development packages first'

    def setUp(self):
        self.scratch=tempfile.TemporaryDirectory(prefix='pasteroll-gnome-package-')
        self.addCleanup(self.scratch.cleanup)
        self.temp=Path(self.scratch.name)
        with tarfile.open(self.tar) as archive:archive.extractall(self.temp,filter='data')
        self.bundle=self.temp/f'pasteroll-{self.version}-linux-x86_64-gnome-development'
        self.env=dict(os.environ,HOME=str(self.temp/'home'),XDG_CONFIG_HOME=str(self.temp/'config'),XDG_DATA_HOME=str(self.temp/'data'))
        for key in ('DISPLAY','WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS','DESTDIR','DPKG_ROOT'):self.env.pop(key,None)

    def command(self,*args,env=None,ok=True):
        result=subprocess.run([str(a) for a in args],cwd=self.temp,env=env or self.env,capture_output=True,text=True,timeout=30)
        if ok:self.assertEqual(result.returncode,0,result.stderr.replace(str(ROOT),'/src'))
        else:self.assertNotEqual(result.returncode,0)
        return result.stdout

    def helper(self,*args,env=None,ok=True):return self.command('sh',self.bundle/'install-input-helper.sh',*args,env=env,ok=ok)

    def test_archive_static_app_separate_dynamic_helper_and_metadata(self):
        with tarfile.open(self.tar) as archive:
            for member in archive.getmembers():
                self.assertEqual((member.uid,member.gid,member.uname,member.gname),(0,0,'',''))
                self.assertTrue(member.isfile() or member.isdir())
                self.assertNotIn('..',Path(member.name).parts)
        self.assertNotIn('INTERP',self.command('readelf','-lW',self.bundle/'pasteroll'))
        self.assertIn('libevdev.so.2',self.command('readelf','-dW',self.bundle/'pasteroll-input'))
        self.assertIn('optional host service',self.command(self.bundle/'pasteroll-input','--help'))
        self.command(self.bundle/'pasteroll-input','--inject',ok=False)
        for file in (ROOT/'extensions/pasteroll@pasteroll.com').iterdir():
            if file.suffix in ('.js','.css','.json'):
                self.assertEqual((self.bundle/'gnome-companion'/file.name).read_bytes(),file.read_bytes())

    def test_helper_install_upgrade_and_remove_preserve_user_data(self):
        stage=self.temp/'root'
        env=dict(self.env,DESTDIR=str(stage))
        history=stage/'home/example/history.db'
        history.parent.mkdir(parents=True)
        history.write_bytes(b'synthetic history fixture')
        self.helper('--enable',env=env)
        self.helper(env=env)
        self.assertEqual((stage/'usr/local/libexec/pasteroll-input').read_bytes(),(self.bundle/'pasteroll-input').read_bytes())
        unit=stage/'etc/systemd/system/pasteroll-input.service'
        self.assertIn('ExecStart=/usr/local/libexec/pasteroll-input',unit.read_text())
        self.assertIn('WatchdogSec=3',unit.read_text())
        self.assertEqual((stage/'etc/modules-load.d/pasteroll-input.conf').read_text(),'uinput\n')
        self.helper('--uninstall',env=env)
        self.helper('--uninstall',env=env)
        self.assertFalse(unit.exists())
        self.assertFalse((stage/'usr/local/libexec/pasteroll-input').exists())
        self.assertEqual(history.read_bytes(),b'synthetic history fixture')

    def test_helper_refuses_unknown_install_and_invalid_options(self):
        stage=self.temp/'unknown'
        binary=stage/'usr/local/libexec/pasteroll-input'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b'not owned by this installer')
        env=dict(self.env,DESTDIR=str(stage))
        self.helper(env=env,ok=False)
        self.helper('--uninstall',env=env,ok=False)
        self.assertEqual(binary.read_bytes(),b'not owned by this installer')
        self.helper('--unknown',ok=False)
        self.helper(env=dict(self.env,DESTDIR='relative'),ok=False)
        if os.geteuid()!=0:self.helper(ok=False)

    def test_per_user_companion_install_is_repeatable(self):
        if os.geteuid()==0:self.skipTest('normal desktop account required')
        installer=self.bundle/'install-gnome-companion.sh'
        self.command('sh',installer)
        self.command('sh',installer)
        target=Path(self.env['XDG_DATA_HOME'])/'gnome-shell/extensions/pasteroll@pasteroll.com'
        self.assertEqual((target/'input-helper.js').read_bytes(),(self.bundle/'gnome-companion/input-helper.js').read_bytes())
        self.assertFalse(Path(self.env['XDG_CONFIG_HOME']).exists())

    def test_debian_payload_dependencies_and_disabled_service(self):
        depends=self.command('dpkg-deb','--field',self.deb,'Depends')
        for word in ('libc6','libevdev2','gnome-shell (>= 46)','gnome-shell (<< 47)','systemd'):self.assertIn(word,depends)
        stage=self.temp/'deb'
        self.command('dpkg-deb','--extract',self.deb,stage)
        self.command('dpkg-deb','--control',self.deb,stage/'DEBIAN')
        for path in ('usr/bin/pasteroll','usr/libexec/pasteroll-input','usr/share/dbus-1/system.d/com.pasteroll.Input.conf','usr/share/gnome-shell/extensions/pasteroll@pasteroll.com/input-helper.js','usr/lib/modules-load.d/pasteroll-input.conf'):
            self.assertTrue((stage/path).is_file())
        self.assertEqual(hashlib.sha256((stage/'usr/libexec/pasteroll-input').read_bytes()).digest(),hashlib.sha256((self.bundle/'pasteroll-input').read_bytes()).digest())
        self.assertNotIn('systemctl enable',(stage/'DEBIAN/postinst').read_text())
        self.assertFalse((stage/'etc/systemd/system/multi-user.target.wants/pasteroll-input.service').exists())

    def test_systemd_unit_syntax(self):
        # This checks the real options without installing the executable in /usr.
        unit=self.temp/'pasteroll-input.service'
        unit.write_text((self.bundle/'pasteroll-input.service').read_text().replace('ExecStart=/usr/local/libexec/pasteroll-input','ExecStart=/usr/bin/true'))
        self.command('systemd-analyze','verify',unit)
        for name in ('install-input-helper.sh','install-gnome-companion.sh'):
            self.command('sh','-n',self.bundle/name)

    def test_real_dpkg_upgrade_remove_purge_in_disposable_root(self):
        stage=self.temp/'dpkg-root'
        (stage/'var/lib/dpkg').mkdir(parents=True)
        (stage/'var/lib/dpkg/status').touch()
        (stage/'var/log').mkdir()
        data=stage/'home/example/history.db'
        data.parent.mkdir(parents=True)
        data.write_bytes(b'keep synthetic data')
        env=dict(self.env,DPKG_ROOT=str(stage))
        command=['dpkg',f'--root={stage}',f'--log={stage}/var/log/dpkg.log','--force-not-root','--force-script-chrootless','--force-depends']
        # Forced dependencies only isolate package lifecycle scripts. This does not
        # establish dependency resolution or a clean-machine desktop install.
        for _ in range(2):self.command(*command,'--install',self.deb,env=env)
        self.command(*command,'--remove','pasteroll',env=env)
        self.assertFalse((stage/'usr/libexec/pasteroll-input').exists())
        self.command(*command,'--install',self.deb,env=env)
        self.command(*command,'--purge','pasteroll',env=env)
        self.assertEqual(data.read_bytes(),b'keep synthetic data')

if __name__=='__main__':unittest.main(verbosity=2)
