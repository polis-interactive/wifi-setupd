"""Packaging boundary checks; no Cargo, network, SSH or target device required."""
import importlib.util
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "build-support/release.py"
SPEC = importlib.util.spec_from_file_location("release", SCRIPT)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class InstallTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.web = self.root / "public"
        (self.web / "assets").mkdir(parents=True)
        (self.web / "index.html").write_text("<html>portal</html>")
        (self.web / "assets/app.wasm").write_bytes(b"\0asm")
        self.binary = self.root / "wifi-setupd"
        self.binary.write_bytes(b"\x7fELFtest fixture")
        self.dest = self.root / "target"
        self.args = SimpleNamespace(binary=self.binary, web_root=self.web, destdir=self.dest)

    def test_install_allowlist_permissions_and_refresh(self):
        unrelated = self.dest / "etc/keep"
        unrelated.parent.mkdir(parents=True)
        unrelated.write_text("image-owned configuration")
        release.install(self.args)
        installed_web = self.dest / "usr/share/wifi-setupd/web"
        (installed_web / "assets/stale.js").write_text("old release")
        release.install(self.args)
        files = {str(p.relative_to(self.dest)) for p in self.dest.rglob("*") if p.is_file()}
        self.assertEqual(files, {
            "usr/bin/wifi-setupd", "usr/share/licenses/wifi-setupd/LICENSE-MIT",
            "usr/share/wifi-setupd/web/index.html",
            "usr/share/wifi-setupd/web/assets/app.wasm", "etc/keep",
        })
        self.assertEqual(stat.S_IMODE((self.dest / "usr/bin/wifi-setupd").stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((installed_web / "index.html").stat().st_mode), 0o644)
        self.assertEqual(unrelated.read_text(), "image-owned configuration")

    def test_unexpected_files_rejected_before_install(self):
        for name in ("device-key", "config.toml", "assets/source.rs", "assets/app.js.map"):
            with self.subTest(name=name):
                path = self.web / name
                path.write_text("must not ship")
                with self.assertRaises(ValueError):
                    release.install(self.args)
                self.assertFalse(self.dest.exists())
                path.unlink()

    def test_incomplete_build_preserves_previous_install(self):
        release.install(self.args)
        (self.web / "assets/app.wasm").unlink()
        with self.assertRaises(ValueError):
            release.install(self.args)
        self.assertTrue((self.dest / "usr/share/wifi-setupd/web/assets/app.wasm").is_file())

    def test_symlink_inputs_and_destinations_rejected(self):
        link = self.web / "assets/secret.js"
        link.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            release.install(self.args)
        link.unlink()
        self.dest.mkdir()
        (self.dest / "usr").symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            release.install(self.args)

    def test_live_root_and_non_elf_rejected(self):
        self.args.destdir = Path("/")
        with self.assertRaises(ValueError):
            release.install(self.args)
        self.args.destdir = self.dest
        self.binary.write_text("not a built executable")
        with self.assertRaises(ValueError):
            release.install(self.args)
        self.assertFalse(self.dest.exists())


if __name__ == "__main__":
    unittest.main()
