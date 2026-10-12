import unittest
import subprocess
import tempfile
from pathlib import Path
from unittest.mock import Mock, patch

import probe


class ProbeTests(unittest.TestCase):
    def test_temporary_ap_profile_is_removed_after_probe_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            script = probe.AP_PROFILE.replace("/var/lib/iwd/ap", directory)
            result = subprocess.run(["sh", "-c", script, "probe-ap", "/bin/false", "wlan0", "test"],
                                    input=b"test-profile", capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((Path(directory) / "test.ap").exists())

    def test_temporary_ap_profile_never_overwrites_existing_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "test.ap"
            profile.write_bytes(b"existing-profile")
            script = probe.AP_PROFILE.replace("/var/lib/iwd/ap", directory)
            result = subprocess.run(["sh", "-c", script, "probe-ap", "/bin/true", "wlan0", "test"],
                                    input=b"replacement", capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(profile.read_bytes(), b"existing-profile")

    @patch("probe.subprocess.run")
    def test_ap_profile_psk_is_stdin_only_and_matches_wpa_vector(self, run):
        pi = Mock()
        pi.config = {"wifi_interface": "test-wlan"}
        pi.ssh.return_value = ["ssh", "test-pi"]
        pi.command.return_value = "AP command"
        probe.run_ap(pi, "/data/bin/probe", "IEEE", "password")
        profile = run.call_args.kwargs["input"].decode()
        self.assertIn("DisableHT=true", profile)
        self.assertIn("PreSharedKey=f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e", profile)
        self.assertNotIn("password", profile)
        self.assertNotIn("f42c6fc5", str(pi.command.call_args))
        self.assertNotIn("f42c6fc5", str(run.call_args.args))

    @patch("probe.subprocess.run")
    def test_ap_rejects_profile_path_escape_and_invalid_secret_before_ssh(self, run):
        for ssid, password in [("../saved", "password"), ("AP", "short"), ("AP", "password\n")]:
            with self.subTest(ssid=ssid), self.assertRaises(ValueError):
                probe.run_ap(Mock(), "/data/bin/probe", ssid, password)
        run.assert_not_called()

    @patch("probe.subprocess.run")
    def test_secret_is_sent_only_on_stdin(self, run):
        pi = Mock()
        pi.config = {"wifi_interface": "test-wlan"}
        pi.ssh.return_value = ["ssh", "test-pi"]
        pi.command.return_value = "quoted non-secret command"
        secret = b"test-network\ntest-password\n"
        probe.run(pi, "/data/bin/probe", "connect", [], secret)
        pi.command.assert_called_once_with("exec", ["/data/bin/probe", "test-wlan", "connect"])
        args, kwargs = run.call_args
        self.assertNotIn("test-password", str(args))
        self.assertEqual(kwargs["input"], secret)
        self.assertTrue(kwargs["check"])
        self.assertEqual(kwargs["timeout"], 240)

    @patch("probe.subprocess.run")
    def test_read_only_operation_has_no_secret_input(self, run):
        pi = Mock()
        pi.config = {"wifi_interface": "test-wlan"}
        pi.ssh.return_value = ["ssh", "test-pi"]
        pi.command.return_value = "status command"
        probe.run(pi, "/data/bin/probe", "status", [])
        self.assertIsNone(run.call_args.kwargs["input"])

    @patch("probe.subprocess.run", side_effect=probe.subprocess.CalledProcessError(1, ["ssh"]))
    def test_remote_failure_is_not_reported_as_success(self, _run):
        pi = Mock()
        pi.config = {"wifi_interface": "test-wlan"}
        pi.ssh.return_value = ["ssh", "test-pi"]
        pi.command.return_value = "status command"
        with self.assertRaises(probe.subprocess.CalledProcessError):
            probe.run(pi, "/data/bin/probe", "status", [])


if __name__ == "__main__":
    unittest.main()
