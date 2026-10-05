#!/usr/bin/env python3
"""Installer tests use isolated homes and service stubs, never the real desktop."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "installer", Path(__file__).resolve().parents[1] / "scripts/prebuilt_installer.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.path = ".local/bin/gardengate"
        self.files = {self.path: (b"binary v1", 0o755), "share/menu": (b"menu", 0o644)}

    def test_fresh_repeat_and_upgrade(self):
        installer.install_files(self.home, self.files)
        installer.install_files(self.home, self.files)
        self.files[self.path] = (b"binary v2", 0o755)
        installer.install_files(self.home, self.files)
        self.assertEqual((self.home / self.path).read_bytes(), b"binary v2")
        self.assertEqual((self.home / self.path).stat().st_mode & 0o777, 0o755)

    def test_unmanaged_destination_blocks_all_writes(self):
        destination = self.home / "share/menu"
        destination.parent.mkdir()
        destination.write_bytes(b"user file")
        with self.assertRaisesRegex(RuntimeError, "Unmanaged"):
            installer.install_files(self.home, self.files)
        self.assertFalse((self.home / self.path).exists())
        self.assertEqual(destination.read_bytes(), b"user file")

    def test_modified_file_blocks_upgrade_and_removal(self):
        installer.install_files(self.home, self.files)
        (self.home / self.path).write_bytes(b"user edit")
        with self.assertRaisesRegex(RuntimeError, "Modified"):
            installer.install_files(self.home, self.files)
        with patch.object(installer, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "Modified"):
                installer.uninstall(self.home)
            run.assert_not_called()

    def test_parent_symlink_is_rejected(self):
        (self.home / ".local").symlink_to(self.home / "elsewhere")
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            installer.install_files(self.home, self.files)

    def test_record_symlink_is_rejected(self):
        record = self.home / installer.RECORD
        record.parent.mkdir(parents=True)
        record.symlink_to(self.home / "other")
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            installer.install_files(self.home, self.files)

    def test_traversal_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "Unsafe"):
            installer.install_files(self.home, {"../escape": (b"bad", 0o644)})

    def test_uninstall_keeps_user_data(self):
        installer.install_files(self.home, self.files)
        data = self.home / "my-inbox.txt"
        data.write_text("keep me")
        with patch.object(installer, "run"):
            installer.uninstall(self.home)
        self.assertFalse((self.home / self.path).exists())
        self.assertFalse((self.home / installer.RECORD).exists())
        self.assertEqual(data.read_text(), "keep me")

    def test_removed_file_is_retired_on_upgrade(self):
        installer.install_files(self.home, self.files)
        installer.install_files(self.home, {self.path: (b"v2", 0o755)})
        self.assertFalse((self.home / "share/menu").exists())
        record = json.loads((self.home / installer.RECORD).read_text())
        self.assertEqual(list(record["files"]), [self.path])


if __name__ == "__main__":
    unittest.main()
