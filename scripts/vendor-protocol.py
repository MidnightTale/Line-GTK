#!/usr/bin/env python3
"""Prepare pinned protocol sources from GitHub, or verify the local cache.

No network is used by --verify or --offline. npm dependencies remain pinned in deno.lock.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "protocol/vendor/deps"
SOURCES = {
    "linejs": ("evex-dev/linejs", "v3.5.0", "c92a7972c76710d8839d7e6dd356a34518b56031fdcda34d2eb872a5204f3259"),
    "std": ("denoland/std", "88186e6c53ff8fe19ed6fbba1badafc949651c55", "1fafc308b3838232c542355f96267953d52e92e3bad4a6463e34c294b09200ac"),
    "loose-types": ("EdamAme-x/loose-types", "41aaafe078be3c047445195b004664130297a431", "9c2f3aaccdb44107e8349d8e1226f0f4cc60eb843060c2dd1a94c4686e7c7ad8"),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify():
    manifest = json.loads((DEST / "manifest.json").read_text())
    expected_sources = {k: {"repository": v[0], "ref": v[1], "archive_sha256": v[2]}
                        for k, v in SOURCES.items()}
    if manifest["sources"] != expected_sources:
        raise SystemExit("Dependency source pins differ from manifest; run scripts/update-protocol.sh")
    for name, expected in manifest["files"].items():
        path = DEST / name
        if not path.is_file() or digest(path) != expected:
            raise SystemExit(f"Vendored dependency missing or modified: {path}")
    print(f"Verified {len(manifest['files'])} vendored dependency files")


def ensure(archives, *, offline=False):
    manifest = json.loads((DEST / "manifest.json").read_text())
    missing = False
    for name, expected in manifest["files"].items():
        path = DEST / name
        if not path.is_file():
            missing = True
        elif digest(path) != expected:
            raise SystemExit(f"Vendored dependency modified: {path}")
    if missing:
        restore(archives, offline=offline, locked=True)
    else:
        verify()


def restore(archives, *, offline=False, locked=False):
    imports = {}
    packages = []
    with tempfile.TemporaryDirectory(prefix="line-gtk-vendor-") as directory:
        work = Path(directory)
        prepared = work / "prepared"
        for name, (repo, ref, expected) in SOURCES.items():
            archive = archives / f"{name}.tar.gz"
            if not archive.exists():
                if offline:
                    raise SystemExit(f"Offline dependency archive missing: {archive}. Run python3 scripts/vendor-protocol.py --ensure online first.")
                archives.mkdir(parents=True, exist_ok=True)
                url = f"https://api.github.com/repos/{repo}/tarball/{ref}"
                request = urllib.request.Request(url, headers={"User-Agent": "line-gtk-vendor"})
                with urllib.request.urlopen(request, timeout=60) as response:
                    payload = response.read()
                if hashlib.sha256(payload).hexdigest() != expected:
                    raise SystemExit(f"Archive checksum mismatch: {archive}")
                archive.write_bytes(payload)
            if digest(archive) != expected:
                raise SystemExit(f"Archive checksum mismatch: {archive}")
            extracted = work / name
            with tarfile.open(archive) as source:
                source.extractall(extracted, filter="data")
            upstream = next(extracted.iterdir())
            if name == "linejs":
                selections = [("packages/linejs", "linejs"), ("packages/types", "linejs-types")]
            elif name == "std":
                selections = [(p, "std-" + p) for p in ("path", "internal", "assert")]
            else:
                selections = [(".", "loose-types")]
            for subdir, target in selections:
                source_dir = upstream / subdir
                config = json.loads((source_dir / "deno.json").read_text())
                packages.append((target, config))
                target_dir = prepared / target
                if target_dir.exists():
                    shutil.rmtree(target_dir)
                target_dir.mkdir(parents=True)
                for file in source_dir.rglob("*.ts"):
                    if file.name.endswith(("_test.ts", ".test.ts")):
                        continue
                    relative = file.relative_to(source_dir)
                    output = target_dir / relative
                    output.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(file, output)
                shutil.copyfile(upstream / "LICENSE", target_dir / "LICENSE")

        # Explicit npm versions prevent a vendored package's original import map
        # from resolving a newer dependency. Deno's lockfile checks their integrity.
        imports.update({
            "thrift-types": "npm:@types/thrift@0.10.17",
            "crypto-js": "npm:crypto-js@4.2.0",
            "curve25519-js": "npm:curve25519-js@0.0.4",
            "node-bignumber": "npm:node-bignumber@1.2.2",
            "thrift": "npm:thrift@0.20.0",
            "tweetnacl": "npm:tweetnacl@1.0.3",
            "node-int64": "npm:node-int64@0.4.0",
            "undici": "npm:undici@7.29.0",
            "@noble/ciphers/aes.js": "npm:@noble/ciphers@2.2.0/aes.js",
            "opusscript": "npm:opusscript@0.1.1",
        })
        for target, config in packages:
            for export, value in config["exports"].items():
                key = config["name"] + ("" if export == "." else export[1:])
                imports[key] = f"./vendor/deps/{target}/{value.removeprefix('./')}"
        imports["@linegtk/shop"] = "./vendor/deps/linejs/base/service/shop/mod.ts"
        import_map = ROOT / "protocol/dependencies.json"
        import_bytes = (json.dumps({"imports": imports}, indent=2, ensure_ascii=False) + "\n").encode()
        files = {str(p.relative_to(prepared)): digest(p) for p in sorted(prepared.rglob("*"))
                 if p.is_file()}
        files["../../dependencies.json"] = hashlib.sha256(import_bytes).hexdigest()
        manifest = {"sources": {k: {"repository": v[0], "ref": v[1], "archive_sha256": v[2]}
                                for k, v in SOURCES.items()}, "files": files}
        manifest_path = DEST / "manifest.json"
        if locked:
            if manifest != json.loads(manifest_path.read_text()) or import_bytes != import_map.read_bytes():
                raise SystemExit("Downloaded sources differ from pinned manifest/import map; refusing to update pins")
        # Verify the entire download before replacing any local library files.
        DEST.mkdir(parents=True, exist_ok=True)
        for package in prepared.iterdir():
            target = DEST / package.name
            if target.exists():
                shutil.rmtree(target)
            shutil.copytree(package, target)
        if not locked:
            import_map.write_bytes(import_bytes)
            manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
        verify()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--verify", action="store_true")
    mode.add_argument("--ensure", action="store_true", help="Prepare missing sources without changing pins")
    parser.add_argument("--offline", action="store_true", help="Use only cached archives")
    parser.add_argument("--archives", type=Path, default=ROOT / "target/protocol-archives")
    args = parser.parse_args()
    offline = args.offline or os.environ.get("DENO_OFFLINE") == "1"
    if args.verify:
        verify()
    elif args.ensure:
        ensure(args.archives, offline=offline)
    else:
        restore(args.archives, offline=offline)
