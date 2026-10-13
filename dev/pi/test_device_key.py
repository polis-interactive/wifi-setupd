import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch

import device_key


class DeviceKeyTests(unittest.TestCase):
    def test_generated_password_has_three_words_and_three_digits(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(device_key.secrets, "choice", side_effect=["cedar", "maple", "river"]) as choice:
                with patch.object(device_key.secrets, "randbelow", return_value=7) as digits:
                    key = device_key.read_key(device_key.prepare(Path(temporary)))
            self.assertEqual(key, "cedarmapleriver007")
            self.assertEqual(choice.call_count, 3)
            choice.assert_called_with(device_key.WORDS)
            digits.assert_called_once_with(1000)
            self.assertEqual(len(device_key.WORDS), len(set(device_key.WORDS)))
            self.assertTrue(all(word.isascii() and word.isalpha() and word.islower()
                                for word in device_key.WORDS))
            self.assertLessEqual(3 * max(map(len, device_key.WORDS)) + 3, 63)

    def test_generation_is_private_distinct_and_retried_after_failed_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "serial-a"
            other = device_key.prepare(Path(temporary) / "serial-b")
            pi = Mock(config={"wifi_interface": "wlan0"})
            upload = Mock(side_effect=RuntimeError("offline"))
            with self.assertRaises(RuntimeError):
                device_key.provision(pi, upload, "/device", "Setup", directory)
            pending = directory / "device-key.pending"
            key = device_key.read_key(pending)
            self.assertNotEqual(key, device_key.read_key(other))
            self.assertEqual(pending.stat().st_mode & 0o777, 0o600)
            self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
            upload.side_effect = None
            current = device_key.provision(pi, upload, "/device", "Setup", directory)
            self.assertEqual(device_key.read_key(current), key)
            self.assertFalse(pending.exists())
            self.assertEqual(device_key.prepare(directory), current)
            self.assertEqual(upload.call_args_list[0].args[2], upload.call_args_list[1].args[2])

    def test_rotation_preserves_old_local_key_until_install_succeeds(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            pi = Mock(config={"wifi_interface": "wlan0"})
            current = device_key.provision(pi, Mock(), "/device", "Setup", directory)
            old = current.read_bytes()
            with self.assertRaises(RuntimeError):
                device_key.provision(pi, Mock(side_effect=RuntimeError()), "/device", "Setup", directory, rotate=True)
            self.assertEqual(current.read_bytes(), old)
            with self.assertRaises(ValueError):
                device_key.prepare(directory, rotate=True)
            device_key.provision(pi, Mock(), "/device", "Setup", directory)
            self.assertNotEqual(current.read_bytes(), old)

    def test_imported_secret_drives_both_management_file_and_iwd_psk(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "import"
            source.write_text("password\n")
            candidate = device_key.prepare(Path(temporary) / "serial", source)
            data = device_key.payload("IEEE", device_key.read_key(candidate))
            with tarfile.open(fileobj=io.BytesIO(data)) as archive:
                self.assertEqual(archive.extractfile("device-key").read(), b"password\n")
                profile = archive.extractfile("ap.profile").read().decode()
                self.assertIn("PreSharedKey=f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e", profile)
                self.assertNotIn("password", profile)
                self.assertIn("DNSList=192.168.16.1\n", profile)
                self.assertIn("Gateway=192.168.16.1\n", profile)

    def test_install_rejects_truncated_upload_and_can_repair_partial_pair(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "device"
            root.mkdir()
            profile = base / "iwd" / "Setup.ap"
            profile.parent.mkdir()
            profile.write_bytes(b"old-profile")
            (root / "device-key").write_bytes(b"old-password")
            (root / "provisioning-incomplete").touch()
            # Exercise the actual install script with only the hardware command
            # stubbed; there is no host or Pi Wi-Fi access.
            binaries = base / "bin"
            binaries.mkdir()
            for name in ("iwctl", "sync"):
                script = binaries / name
                script.write_text("#!/bin/sh\nexit 0\n")
                script.chmod(0o700)
            script = device_key.INSTALL.replace("/bin/busybox flock", "flock")
            data = device_key.payload("Setup", "replacement-password")
            args = ["sh", "-c", script, "provision", str(root), "wlan0", str(profile), hashlib.sha256(data).hexdigest()]
            env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ["PATH"])
            failed = subprocess.run(args, input=data[:100], capture_output=True, env=env)
            self.assertNotEqual(failed.returncode, 0)
            self.assertEqual(profile.read_bytes(), b"old-profile")
            self.assertEqual((root / "device-key").read_bytes(), b"old-password")
            result = subprocess.run(args, input=data, capture_output=True, env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / "device-key").read_text(), "replacement-password\n")
            self.assertFalse((root / "provisioning-incomplete").exists())
            self.assertEqual(profile.stat().st_mode & 0o777, 0o600)
            self.assertEqual((root / "device-key").stat().st_mode & 0o777, 0o600)
            self.assertNotIn(b"replacement-password", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
