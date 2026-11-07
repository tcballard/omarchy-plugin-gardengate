import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('xps', Path(__file__).with_name('xps.py'))
xps = importlib.util.module_from_spec(spec)
spec.loader.exec_module(xps)


class SafetyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.patch = patch.object(xps, 'BASE', base)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.ident = 'a' * 32
        self.root = base / self.ident
        self.root.mkdir()
        binary = base / 'binary'
        binary.write_bytes(b'test')
        (self.root / 'run.json').write_text(json.dumps({'id': self.ident, 'binary': str(binary), 'binary_sha256': xps.digest(b'test')}))
        for directory in ['local', 'config/gardengate', 'state/gardengate']:
            (self.root / directory).mkdir(parents=True)
        self.run = xps.Run(self.ident)
        self.addCleanup(self.run.lock.close)
        (self.run.local / xps.MARKER).write_text(self.ident)

    def test_invalid_id_and_unknown_run(self):
        for ident in ['../../production', 'b' * 32, '', '/tmp']:
            with self.assertRaises(RuntimeError):
                xps.Run(ident)

    def test_rejects_symlink_and_changed_binary(self):
        link = self.root / 'local/link'
        link.symlink_to('/tmp')
        with self.assertRaises(RuntimeError):
            xps.manifest(self.run.local)
        Path(self.run.binary).write_bytes(b'changed')
        with self.assertRaises(RuntimeError):
            xps.Run(self.ident)

    def test_guard_rejects_cloud_marker_and_production_mapping(self):
        with patch.object(self.run, 'rc', return_value=b'other'):
            with self.assertRaises(RuntimeError):
                self.run.guard()
        profile = self.root / 'config/gardengate/profile.json'
        profile.write_text(json.dumps({'remote_folder': 'Production', 'local': str(self.run.local), 'paused': True}))
        with patch.object(self.run, 'rc', return_value=self.ident.encode()):
            with self.assertRaises(RuntimeError):
                self.run.guard()

    def test_existing_cloud_folder_never_adopted(self):
        with patch.object(self.run, 'preflight', return_value=True), patch.object(self.run, 'approve') as approval, patch.object(self.run, 'rc') as rc:
            with self.assertRaises(RuntimeError):
                self.run.run()
            approval.assert_not_called()
            rc.assert_not_called()

    def test_requires_tty_and_exact_approval(self):
        with patch('sys.stdin.isatty', return_value=False):
            with self.assertRaises(RuntimeError):
                self.run.approve('WRITE')
        with patch('sys.stdin.isatty', return_value=True), patch('builtins.input', return_value='yes'):
            with self.assertRaises(RuntimeError):
                self.run.approve('DELETE')

    def test_preflight_only_reads_and_checks_pin(self):
        self.run.config.write_bytes(b'# Encrypted rclone configuration File\n\nRCLONE_ENCRYPT_V0:\nfixture')
        calls = []
        def rc(*args):
            calls.append(args)
            return b'rclone v1.75.1\n' if args[0] == 'version' else b'[]'
        with patch.object(self.run, 'rc', side_effect=rc):
            self.assertFalse(self.run.preflight())
        self.assertEqual(calls, [('version',), ('lsjson', 'icloud:', '--dirs-only')])
        with patch.object(self.run, 'rc', return_value=b'rclone v1.60.1\n'):
            with self.assertRaises(RuntimeError):
                self.run.preflight()

    def test_assertions_fail_for_missing_content_deletion_and_backup(self):
        data = {'local': {'a': xps.digest(b'expected')}, 'cloud': {'a': xps.digest(b'expected')}}
        self.run.check(data, {'a': 'expected'})
        for kwargs in [{'files': {'a': 'wrong'}}, {'absent': ['a']}, {'preserved': ['expected']}]:
            with self.assertRaises(RuntimeError):
                self.run.check(data, **kwargs)

    def test_snapshot_hashes_both_sides_without_cloud_writes(self):
        def rc(*args):
            self.assertEqual(args[0], 'copy')
            dest = Path(args[2])
            dest.mkdir()
            (dest / xps.MARKER).write_text(self.ident)
            return b''
        with patch.object(self.run, 'guard'), patch.object(self.run, 'rc', side_effect=rc):
            data = self.run.snapshot('test')
        self.assertEqual(data['local'], data['cloud'])
        self.assertEqual(len(list((self.root / 'evidence').glob('*/manifest.json'))), 1)

    def test_recovery_never_runs_without_approval(self):
        before = {'sync_state': {'phase': 'recovery-required'}}
        with patch.object(self.run, 'guard'), patch.object(self.run, 'snapshot', return_value=before), patch.object(self.run, 'approve', side_effect=RuntimeError('declined')), patch.object(self.run, 'gg') as gg:
            with self.assertRaises(RuntimeError):
                self.run.recover()
            gg.assert_not_called()

    def test_network_failure_checks_local_preservation(self):
        from subprocess import CompletedProcess
        before = {'local': {}, 'cloud': {}}
        with patch.object(self.run, 'guard'), patch.object(self.run, 'state_json', return_value={'phase': 'ready'}), patch.object(self.run, 'approve'), patch.object(self.run, 'snapshot', return_value=before), patch.object(self.run, 'gg', return_value=CompletedProcess([], 1)), patch('builtins.input', return_value='OFFLINE'):
            with self.assertRaisesRegex(RuntimeError, 'Local files changed'):
                self.run.network()

    def test_command_failure_records_no_provider_output(self):
        import sys
        with self.assertRaises(RuntimeError):
            self.run.command([sys.executable, '-c', "import sys; sys.stderr.write('SECRET_PROVIDER_OUTPUT'); sys.exit(2)"])
        # The fixture text appears in argv; raw returned provider output is not logged.
        events = [json.loads(line) for line in (self.root / 'events.jsonl').read_text().splitlines()]
        self.assertEqual(events[-1]['returncode'], 2)
        self.assertNotIn('stderr', events[-1])
        self.assertNotIn('stdout', events[-1])

    def test_restart_refused_and_recovery_requires_unready_history(self):
        (self.root / 'started').touch()
        with self.assertRaises(RuntimeError):
            self.run.run()
        with patch.object(self.run, 'guard'), patch.object(self.run, 'snapshot', return_value={'sync_state': {'phase': 'ready'}}), patch.object(self.run, 'approve') as approve:
            with self.assertRaises(RuntimeError):
                self.run.recover()
            approve.assert_not_called()


if __name__ == '__main__':
    unittest.main()
