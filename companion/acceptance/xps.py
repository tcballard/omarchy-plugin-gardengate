#!/usr/bin/env python3
"""Opt-in live acceptance. No cloud writes occur in new/preflight/evidence."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid

MARKER = 'acceptance-owner.txt'
BASE = Path.home() / 'GardenGate-Acceptance'


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def manifest(root):
    result = {}
    if root.exists():
        for p in sorted(root.rglob('*')):
            require(not p.is_symlink(), 'Symlink in acceptance data; stopped')
            if p.is_file():
                result[str(p.relative_to(root))] = digest(p.read_bytes())
    return result


def safe_path(path):
    for p in [path, *path.parents]:
        require(not p.is_symlink(), 'Symlink path refused')


class Run:
    def __init__(self, ident):
        require(re.fullmatch('[a-f0-9]{32}', ident), 'Invalid run ID')
        self.ident = ident
        self.root = BASE / ident
        safe_path(self.root)
        require(self.root.is_dir(), 'Unknown run; use new')
        self.meta = json.loads((self.root / 'run.json').read_text())
        require(self.meta['id'] == ident, 'Run identity mismatch')
        self.folder = 'GardenGate-Acceptance-' + ident
        self.remote = 'icloud:' + self.folder
        self.local = self.root / 'local'
        self.state = self.root / 'state/gardengate'
        self.config = self.root / 'config/gardengate/rclone.conf'
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('RCLONE_')}
        self.env.update(XDG_CONFIG_HOME=str(self.root / 'config'), XDG_STATE_HOME=str(self.root / 'state'))
        self.binary = self.meta['binary']
        require(digest(Path(self.binary).read_bytes()) == self.meta['binary_sha256'], 'Binary changed; create a new run')
        self.lock = (self.root / 'runner.lock').open('a')
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def event(self, action, **data):
        with (self.root / 'events.jsonl').open('a') as out:
            out.write(json.dumps(dict(time=time.time(), action=action, **data)) + '\n')

    def command(self, args, allow_failure=False):
        self.event('command-start', argv=args)
        p = subprocess.Popen(args, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = p.communicate(timeout=900)
        except BaseException:
            os.killpg(p.pid, signal.SIGTERM)
            try:
                p.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                p.communicate()
            self.event('command-aborted')
            raise
        p = subprocess.CompletedProcess(args, p.returncode, stdout, stderr)
        self.event('command-end', returncode=p.returncode)
        # Provider output may contain account details; never persist raw stderr.
        require(allow_failure or p.returncode == 0, 'Command failed; evidence retained (provider stderr withheld)')
        return p

    def rc(self, *args):
        return self.command(['rclone', '--config', str(self.config), '--password-command',
                             'secret-tool lookup application gardengate', *args]).stdout

    def gg(self, *args, allow_failure=False):
        return self.command([self.binary, *args], allow_failure)

    def approve(self, purpose):
        phrase = purpose + ' ' + self.ident
        print('\nTarget:', self.remote, '\nLocal:', self.local)
        print('May modify disposable cloud files and sibling recovery storage. No automatic cleanup.')
        require(sys.stdin.isatty(), 'Live approval requires an interactive terminal')
        require(input('Type exactly ' + phrase + ': ') == phrase, 'Approval declined')
        self.event('approved', purpose=purpose)

    def state_json(self):
        return json.loads((self.state / 'two-way.json').read_text())

    def guard(self):
        safe_path(self.root)
        for path in self.root.rglob('*'):
            require(not path.is_symlink(), 'Symlink in acceptance run; stopped')
        require((self.local / MARKER).read_text() == self.ident, 'Local ownership marker mismatch')
        require(self.rc('cat', self.remote + '/' + MARKER).decode() == self.ident, 'Cloud ownership marker mismatch')
        profile = self.root / 'config/gardengate/profile.json'
        if profile.exists():
            p = json.loads(profile.read_text())
            require(p['remote_folder'] == self.folder and p['local'] == str(self.local), 'Profile mapping changed')
            require(p['paused'] or p['mode'] == 'download-only', 'Unexpected background approval')
        if (self.state / 'two-way.json').exists():
            s = self.state_json()
            require(s['remote'] == self.remote and s['local'] == str(self.local), 'Sync mapping changed')
            require(re.fullmatch('[a-f0-9]{32}', s['id']), 'Invalid recovery ID')

    def preflight(self):
        require(self.rc('version').decode().splitlines()[0] == 'rclone v1.75.1', 'Requires exactly rclone 1.75.1')
        require(b'RCLONE_ENCRYPT_V0:' in self.config.read_bytes()[:128], 'Encrypted config required')
        entries = json.loads(self.rc('lsjson', 'icloud:', '--dirs-only'))
        # Retain only the test folder match, not other account folder names.
        exists = any(e['Name'] == self.folder for e in entries)
        self.event('preflight', test_folder_exists=exists, cloud_writes=False)
        return exists

    def snapshot(self, label):
        self.guard()
        manifest(self.local)
        manifest(self.state / 'recovery')
        dest = self.root / 'evidence' / (str(time.time_ns()) + '-' + label)
        dest.mkdir(parents=True)
        self.rc('copy', self.remote, str(dest / 'cloud'), '--max-transfer', '32M', '--retries', '1')
        shutil.copytree(self.local, dest / 'local', symlinks=True)
        data = {'local': manifest(dest / 'local'), 'cloud': manifest(dest / 'cloud')}
        if (self.state / 'two-way.json').exists():
            s = self.state_json()
            data['sync_state'] = s
            recovery = self.remote + '.gardengate-recovery-' + s['id']
            # Missing backup roots are legitimate before the first replacement.
            names = json.loads(self.rc('lsjson', 'icloud:', '--dirs-only'))
            if any(e['Name'] == recovery.removeprefix('icloud:') for e in names):
                self.rc('copy', recovery, str(dest / 'cloud-recovery'), '--max-transfer', '32M', '--retries', '1')
            if (self.state / 'recovery').exists():
                shutil.copytree(self.state / 'recovery', dest / 'local-recovery', symlinks=True)
            data['cloud_recovery'] = manifest(dest / 'cloud-recovery')
            data['local_recovery'] = manifest(dest / 'local-recovery')
            for name in ['two-way.json', 'status.json']:
                if (self.state / name).exists():
                    shutil.copy2(self.state / name, dest / name)
        (dest / 'manifest.json').write_text(json.dumps(data, indent=2))
        self.event('snapshot', label=label, path=str(dest))
        return data

    def check(self, data, files=None, absent=(), preserved=()):
        for side in ['local', 'cloud']:
            for name, content in (files or {}).items():
                require(data[side].get(name) == digest(content.encode()), side + ': unexpected ' + name)
            for name in absent:
                require(name not in data[side], side + ': deletion not propagated: ' + name)
        for content in preserved:
            h = digest(content.encode())
            require(any(h in data.get(k, {}).values() for k in ['local_recovery', 'cloud_recovery']), 'Recovery copy missing: ' + content)
        self.event('assertions-passed')

    def phone(self, instruction):
        print('\nOn iPhone Files > iCloud Drive >', self.folder, '\n' + instruction)
        require(input('Wait for iCloud upload, then type DONE: ') == 'DONE', 'Phone step not confirmed')
        self.event('phone-step', instruction=instruction, status='operator-confirmed; content checked next')

    def sync(self, label):
        self.snapshot(label + '-before')
        self.gg('sync', '--apply')
        return self.snapshot(label + '-after')

    def run(self):
        require(not (self.root / 'started').exists(), 'Run already started; inspect evidence and use explicit recover, or create a new run')
        require(not self.preflight(), 'Test folder already exists; refusing adoption')
        require(not any(self.local.iterdir()), 'Local folder must be empty')
        self.approve('WRITE')
        # Recheck after the approval pause; never adopt a pre-existing cloud folder.
        require(not self.preflight(), 'Cloud folder appeared during approval')
        (self.root / 'started').touch(exist_ok=False)
        self.event('starting-state', local={}, cloud='absent')
        (self.local / MARKER).write_text(self.ident)
        self.rc('copyto', str(self.local / MARKER), self.remote + '/' + MARKER, '--immutable')
        # add requires an empty destination; hold marker outside until mapping exists.
        (self.local / MARKER).unlink()
        self.gg('add', self.folder, str(self.local))
        (self.local / MARKER).write_text(self.ident)
        for i in range(10):
            (self.local / ('keep%d.txt' % i)).write_text('keep')
        (self.local / 'xps.txt').write_text('xps-v1')
        self.phone('Create phone.txt containing exactly phone-v1 (no newline).')
        self.snapshot('initial-before')
        self.gg('two-way', 'plan')
        self.gg('two-way', 'enable', '--apply')
        self.check(self.snapshot('initial-after'), {'phone.txt': 'phone-v1', 'xps.txt': 'xps-v1'})
        self.guard()
        (self.local / 'xps.txt').write_text('xps-v2')
        self.phone('Verify xps.txt contains xps-v1. Change phone.txt to phone-v2 (no newline).')
        self.check(self.sync('edits'), {'phone.txt': 'phone-v2', 'xps.txt': 'xps-v2'}, preserved=('phone-v1', 'xps-v1'))
        self.approve('DELETE')
        self.guard()
        (self.local / 'xps.txt').rename(self.local / 'xps-renamed.txt')
        self.phone('Verify xps.txt contains xps-v2, then rename phone.txt to phone-renamed.txt.')
        self.check(self.sync('renames'), {'phone-renamed.txt': 'phone-v2', 'xps-renamed.txt': 'xps-v2'}, absent=('phone.txt', 'xps.txt'))
        self.guard()
        (self.local / 'xps-renamed.txt').unlink()
        self.phone('Delete phone-renamed.txt.')
        self.check(self.sync('deletions'), absent=('phone-renamed.txt', 'xps-renamed.txt'), preserved=('phone-v2', 'xps-v2'))
        (self.local / 'conflict.txt').write_text('base')
        self.sync('conflict-base')
        (self.local / 'conflict.txt').write_text('xps-conflict')
        self.phone('Edit conflict.txt to phone-conflict. Leave both editors closed before continuing.')
        data = self.sync('simultaneous')
        conflicts = data['sync_state']['conflicts']
        require(len(conflicts) == 2, 'Expected two conflict paths')
        for side in ['local', 'cloud']:
            require({data[side].get(p) for p in conflicts} == {digest(b'xps-conflict'), digest(b'phone-conflict')}, 'Both conflict versions not preserved')
        self.event('conflicts-verified')
        self.interrupt()

    def interrupt(self):
        self.approve('INTERRUPT')
        self.guard()
        (self.local / 'interrupted.txt').write_bytes(b'interruption-fixture\n' * 100000)
        self.snapshot('interruption-before')
        self.event('interrupt-start')
        p = subprocess.Popen([self.binary, 'sync', '--apply'], env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            deadline = time.monotonic() + 120
            while p.poll() is None and time.monotonic() < deadline:
                if self.state_json()['phase'] == 'running':
                    p.send_signal(signal.SIGTERM)
                    break
                time.sleep(.02)
            rc = p.wait(timeout=30)
        finally:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGTERM)
                try:
                    p.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid, signal.SIGKILL)
                    p.wait()
        self.event('interrupt-end', returncode=rc)
        require(rc != 0 and self.state_json()['phase'] != 'ready', 'Interruption window missed: INCONCLUSIVE; do not claim a pass')
        self.snapshot('interruption-after')
        require(self.gg('sync', '--apply', allow_failure=True).returncode != 0, 'Sync did not refuse interrupted history')
        self.event('recovery-required-verified')
        print('Stopped as intended. Review evidence, then invoke recover with this run ID.')

    def desktop(self):
        # Read-only host checks. Never start the globally named production units.
        for args in [
            ['systemctl', '--user', 'is-system-running'],
            ['systemctl', '--user', 'show', 'gardengate.service', 'gardengate-sync.service', '--property=ActiveState,SubState,FragmentPath'],
            ['hyprctl', 'version'],
            ['kdialog', '--version'],
        ]:
            p = self.command(args, allow_failure=True)
            self.event('desktop-read-only', argv=args, returncode=p.returncode)
        self.event('desktop-limit', keyring='only exercised by successful cloud preflight', menu_and_services='NOT RUN')

    def network(self):
        self.guard()
        require(self.state_json()['phase'] == 'ready', 'Recover first')
        self.approve('NETWORK')
        (self.local / 'offline.txt').write_text('offline-work')
        before = self.snapshot('network-before')
        print('Disconnect XPS networking now. Keep iPhone editors closed.')
        require(input('Type OFFLINE after disconnecting: ') == 'OFFLINE', 'Cancelled')
        p = self.gg('sync', '--apply', allow_failure=True)
        require(p.returncode != 0, 'Provider did not fail: INCONCLUSIVE')
        require(manifest(self.local) == before['local'], 'Local files changed during provider failure')
        self.event('offline-failure-verified')
        print('Reconnect networking. No automatic retry will occur.')
        require(input('Type ONLINE after reconnecting: ') == 'ONLINE', 'Cancelled')
        after = self.snapshot('network-after')
        require(after['cloud'] == before['cloud'], 'Cloud changed during offline failure; review evidence')
        self.event('network-verified', phase=after['sync_state']['phase'])
        print('Review evidence. If recovery is required, invoke recover explicitly; otherwise keep this run for inspection.')

    def recover(self):
        self.guard()
        before = self.snapshot('recovery-before')
        require(before['sync_state']['phase'] != 'ready', 'No interrupted recovery to accept')
        self.approve('RECOVER')
        self.gg('two-way', 'recover', '--apply')
        after = self.snapshot('recovery-after')
        require(after['sync_state']['phase'] == 'ready', 'Recovery not ready')
        for side in ['local', 'cloud']:
            for name, value in {**before['local'], **before['cloud']}.items():
                require(after[side].get(name) == value, 'Recovery union mismatch: ' + name)
        old = before['sync_state']['generation']
        require((self.state / 'bisync' / old).is_dir(), 'Old history missing')
        self.event('recovery-verified', physical_network_and_native_desktop='NOT RUN')
        print('Recovery checked. Native desktop and physical network checks remain manual; see README.')


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['new', 'preflight', 'run', 'evidence', 'recover', 'network', 'desktop'])
    parser.add_argument('--id')
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    require(os.geteuid() != 0, 'Run as your normal desktop user, never root')
    if args.action == 'new':
        require(args.binary and args.binary.is_file(), 'new requires --binary /absolute/path/to/gardengate')
        binary = args.binary.resolve()
        source = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config'))) / 'gardengate/rclone.conf'
        safe_path(source)
        require(b'RCLONE_ENCRYPT_V0:' in source.read_bytes()[:128], 'Connect Garden Gate first; never enter credentials in evidence')
        safe_path(BASE)
        BASE.mkdir(mode=0o700, exist_ok=True)
        ident = uuid.uuid4().hex
        root = BASE / ident
        root.mkdir(mode=0o700)
        for directory in ['local', 'config/gardengate', 'state/gardengate', 'evidence']:
            (root / directory).mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, root / 'config/gardengate/rclone.conf')
        (root / 'run.json').write_text(json.dumps({'id': ident, 'binary': str(binary), 'binary_sha256': digest(binary.read_bytes()), 'created': time.time()}, indent=2))
        print(ident)
        return
    run = Run(args.id or '')
    try:
        if args.action == 'preflight':
            run.preflight()
            print('Read-only cloud preflight passed; no transfer approval granted.')
        elif args.action == 'evidence':
            run.snapshot('inspection')
        else:
            getattr(run, args.action)()
    except BaseException as error:
        run.event('STOPPED', reason=type(error).__name__)
        print('STOPPED. No automatic retry, recovery or cleanup. Evidence:', run.root / 'evidence', file=sys.stderr)
        raise


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, subprocess.TimeoutExpired, KeyboardInterrupt) as error:
        sys.exit(str(error) or 'Cancelled')
