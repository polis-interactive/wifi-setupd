"""Validate the scoped development policy without mounting or using host D-Bus."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

import portal


class PortalPolicyTests(unittest.TestCase):
    def test_policy_preserves_existing_configuration_and_only_grants_exact_root_service(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            etc, run, root, binaries = (base / name for name in ("etc", "run", "device", "bin"))
            for directory in (etc / "system.d", run, root, binaries):
                directory.mkdir(parents=True, exist_ok=True)
            (etc / "system.conf").write_text("<busconfig/>")
            (etc / "system.d/existing.conf").write_text("<busconfig/>")
            (root / "device-key").touch()
            for name in ("mount", "dbus-send"):
                stub = binaries / name
                stub.write_text("#!/bin/sh\nexit 0\n")
                stub.chmod(0o700)
            script = portal.DBUS_POLICY.replace("/etc/dbus-1", str(etc)).replace("/run/wifi-setupd-dbus", str(run / "wifi-setupd-dbus"))
            result = subprocess.run(["sh", "-c", script, "test", str(root)], env=dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ["PATH"]), capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            stage, = run.iterdir()
            self.assertEqual((stage / "system.conf").read_bytes(), (etc / "system.conf").read_bytes())
            self.assertEqual((stage / "system.d/existing.conf").read_bytes(), (etc / "system.d/existing.conf").read_bytes())
            self.assertEqual(stage.stat().st_mode & 0o777, 0o755)
            policy = ET.parse(stage / "system.d/wifi-setupd-development.conf").getroot()
            entry, = policy
            self.assertEqual(entry.attrib, {"user": "root"})
            self.assertEqual([rule.attrib for rule in entry], [{"own": "tv.polis.WifiSetup1"}, {"send_destination": "tv.polis.WifiSetup1"}])
