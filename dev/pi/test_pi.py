import importlib.util
from pathlib import Path
import shlex
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("pi", Path(__file__).with_name("pi.py"))
pi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pi)


class PiTests(unittest.TestCase):
    def config(self):
        text = Path(__file__).with_name("config.example.toml").read_text()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.toml"
            path.write_text(text.replace("REPLACE_WITH_PI_SERIAL", "0123456789abcdef"))
            return pi.load_config(path)

    def test_shell_arguments_round_trip(self):
        value = "spaces 'quotes' ; $(not-a-command)"
        command = pi.Pi(self.config()).command("exec", ["printf", "%s", value])
        self.assertEqual(shlex.split(command)[-1], value)

    def test_nonzero_remote_exit_fails(self):
        result = subprocess.CompletedProcess([], 7, b"", b"operation failed")
        with patch.object(pi.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(RuntimeError, "failed.*7"):
                pi.Pi(self.config()).remote("status", capture=True)

    def test_linker_symlink_name_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            wrapper = Path(directory) / "toolchain-wrapper"
            wrapper.touch()
            compiler = Path(directory) / "aarch64-linux-gcc"
            compiler.symlink_to(wrapper)
            text = Path(__file__).with_name("config.example.toml").read_text()
            text = text.replace("REPLACE_WITH_PI_SERIAL", "0123456789abcdef")
            text = text.replace("../BuildrootImages/.output/REPLACE_WITH_OUTPUT/host/bin/aarch64-linux-gcc", str(compiler))
            path = Path(directory) / "config.toml"
            path.write_text(text)
            self.assertEqual(pi.load_config(path)["linker"], str(compiler))

    def test_reboot_not_replayed_after_disconnect(self):
        client = pi.Pi(self.config())
        calls = []

        def remote(action, *args, **kwargs):
            calls.append(args)
            if args == ("reboot",):
                raise RuntimeError("disconnected")
            if args == ("cat", "/proc/sys/kernel/random/boot_id"):
                return "before" if len(calls) == 1 else "after"
            if args == ("cat", "/var/lib/dbus/machine-id"):
                return "machine"
            if args[0] == "cat":
                return "token"
            return ""

        with patch.object(client, "remote", side_effect=remote), patch.object(pi.uuid, "uuid4") as token:
            token.return_value.hex = "token"
            client.reboot()
        self.assertEqual(calls.count(("reboot",)), 1)

    def test_elf_architecture_and_dynamic_loader(self):
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", header, 18, 183)
        struct.pack_into("<Q", header, 32, 64)
        struct.pack_into("<HH", header, 54, 56, 1)
        segment = bytearray(56)
        struct.pack_into("<I", segment, 0, 1)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "artifact"
            path.write_bytes(header + segment)
            pi.validate_elf(path)
            struct.pack_into("<I", segment, 0, 3)
            path.write_bytes(header + segment)
            with self.assertRaisesRegex(ValueError, "static"):
                pi.validate_elf(path)
            struct.pack_into("<H", header, 18, 62)
            path.write_bytes(header + segment)
            with self.assertRaisesRegex(ValueError, "AArch64"):
                pi.validate_elf(path)


if __name__ == "__main__":
    unittest.main()
