"""Dependency preparation tests use local archive fixtures and forbid network I/O."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "vendor-protocol.py"
SPEC = importlib.util.spec_from_file_location("vendor_protocol", SCRIPT)
vendor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vendor)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.dest = root / "protocol/vendor/deps"
        self.dest.mkdir(parents=True)
        self.archives = root / "archives"
        self.archives.mkdir()
        sources = {}
        for name, selections in {
            "linejs": ("packages/linejs", "packages/types"),
            "std": ("path", "internal", "assert"),
            "loose-types": (".",),
        }.items():
            archive = self.archives / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as target:
                files = {"LICENSE": b"Fixture license"}
                for index, selection in enumerate(selections):
                    prefix = "" if selection == "." else selection + "/"
                    config = {"name": f"@fixture/{name}-{index}",
                              "exports": {".": "./mod.ts"}}
                    files[prefix + "deno.json"] = json.dumps(config).encode()
                    files[prefix + "mod.ts"] = b"export const value = 1;\n"
                for path, data in files.items():
                    info = tarfile.TarInfo("release/" + path)
                    info.size = len(data)
                    target.addfile(info, io.BytesIO(data))
            sources[name] = (f"fixture/{name}", "pinned-ref",
                             hashlib.sha256(archive.read_bytes()).hexdigest())
        for key, value in (("ROOT", root), ("DEST", self.dest), ("SOURCES", sources)):
            context = patch.object(vendor, key, value)
            context.start()
            self.addCleanup(context.stop)
        network = patch.object(vendor.urllib.request, "urlopen",
                               side_effect=AssertionError("Unexpected network access"))
        self.network = network.start()
        self.addCleanup(network.stop)
        vendor.restore(self.archives, offline=True)
        self.manifest = (self.dest / "manifest.json").read_bytes()
        self.imports = root / "protocol/dependencies.json"
        self.import_bytes = self.imports.read_bytes()

    def remove_sources(self):
        for path in self.dest.iterdir():
            if path.is_dir():
                shutil.rmtree(path)

    def test_fresh_checkout_restores_from_cached_archives_without_changing_pins(self):
        self.remove_sources()
        vendor.ensure(self.archives, offline=True)
        vendor.verify()
        self.assertEqual(self.manifest, (self.dest / "manifest.json").read_bytes())
        self.assertEqual(self.import_bytes, self.imports.read_bytes())

    def test_prepared_sources_need_neither_archives_nor_network(self):
        shutil.rmtree(self.archives)
        vendor.ensure(self.archives, offline=True)
        self.network.assert_not_called()

    def test_offline_missing_archive_fails_without_network(self):
        self.remove_sources()
        (self.archives / "linejs.tar.gz").unlink()
        with self.assertRaisesRegex(SystemExit, "Offline dependency archive missing"):
            vendor.ensure(self.archives, offline=True)
        self.network.assert_not_called()

    def test_corrupt_archive_does_not_install_sources_or_change_pins(self):
        self.remove_sources()
        (self.archives / "std.tar.gz").write_bytes(b"corrupt archive")
        with self.assertRaisesRegex(SystemExit, "Archive checksum mismatch"):
            vendor.ensure(self.archives, offline=True)
        self.assertFalse((self.dest / "linejs").exists())
        self.assertEqual(self.manifest, (self.dest / "manifest.json").read_bytes())

    def test_modified_source_is_rejected_even_when_another_file_is_missing(self):
        (self.dest / "linejs/mod.ts").write_text("modified")
        (self.dest / "std-path/mod.ts").unlink()
        with self.assertRaisesRegex(SystemExit, "dependency modified"):
            vendor.ensure(self.archives, offline=True)

    def test_restore_rejects_a_manifest_that_does_not_match_downloads(self):
        self.remove_sources()
        manifest = json.loads(self.manifest)
        manifest["files"]["linejs/mod.ts"] = "0" * 64
        (self.dest / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(SystemExit, "differ from pinned manifest"):
            vendor.ensure(self.archives, offline=True)
        self.assertFalse((self.dest / "linejs").exists())


if __name__ == "__main__":
    unittest.main()
