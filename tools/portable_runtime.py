"""Build a pinned Windows x64 sidecar runtime; never connect to MetaTrader.

Upstream archives are hash-checked before extraction. Third-party licenses
and wheel metadata are retained. No global Python installation is copied.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import urllib.request
import zipfile

ARTIFACTS = (
    {"name": "CPython", "version": "3.13.15", "target": ".",
     "url": "https://www.python.org/ftp/python/3.13.15/python-3.13.15-embed-amd64.zip",
     "sha256": "d1f04d990aee1253d8569e8e5104e30fa9f5fa830899f14843448872d936a2cf"},
    {"name": "MetaTrader5", "version": "5.0.5735", "target": "Lib/site-packages",
     "url": "https://files.pythonhosted.org/packages/87/2c/7ffc362d84f402b97773be5c6b37f1a0ab9882244c987bbb224c379d17d8/metatrader5-5.0.5735-cp313-cp313-win_amd64.whl",
     "sha256": "0933ea4a9a52b32adcf5590df00f9f75ff380a02bad7b62e23cbd757f34fbb12"},
    {"name": "numpy", "version": "2.5.1", "target": "Lib/site-packages",
     "url": "https://files.pythonhosted.org/packages/10/70/800b3fca480af32df9e8ea9f3d4a0c8feb4b32d7f195d174eabbda4829ad/numpy-2.5.1-cp313-cp313-win_amd64.whl",
     "sha256": "6c3fe51bc6a16453d452997053454f309e8e0ed7b42d6b361ce4ac8c32913d74"},
)
SCHEMA = "conduit.portable-runtime.v1"


def file_hash(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def extract(archive: Path, destination: Path):
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve(strict=True)
    seen = set()
    with zipfile.ZipFile(archive) as zipped:
        for info in zipped.infolist():
            rel = PurePosixPath(info.filename)
            key = str(rel).casefold()
            mode = info.external_attr >> 16
            if (rel.is_absolute() or ".." in rel.parts or not rel.parts
                    or "\\" in info.filename or ":" in info.filename
                    or stat.S_ISLNK(mode) or key in seen):
                raise ValueError("Unsafe or duplicate runtime archive member")
            seen.add(key)
            target = root.joinpath(*rel.parts)
            if not target.resolve().is_relative_to(root):
                raise ValueError("Runtime archive path escapes destination")
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("xb") as output, zipped.open(info) as source:
                shutil.copyfileobj(source, output)


def verify(runtime: Path) -> dict:
    manifest = json.loads((runtime / "RUNTIME_MANIFEST.json").read_text("utf-8"))
    if manifest.get("schema") != SCHEMA or manifest.get("artifacts") != list(ARTIFACTS):
        raise ValueError("Unrecognized runtime manifest")
    actual = set()
    for path in runtime.rglob("*"):
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
            raise ValueError("Runtime contains a reparse point")
        if path.is_file():
            actual.add(path.relative_to(runtime).as_posix())
    expected = set(manifest["files"]) | {"RUNTIME_MANIFEST.json"}
    if actual != expected:
        raise ValueError("Runtime file inventory changed")
    for name, expected_hash in manifest["files"].items():
        rel = PurePosixPath(name)
        if rel.is_absolute() or ".." in rel.parts or "\\" in name or ":" in name:
            raise ValueError("Unsafe runtime manifest member")
        if file_hash(runtime.joinpath(*rel.parts)) != expected_hash:
            raise ValueError("Runtime file hash changed")
    return {"ok": True, "files": len(manifest["files"]), "versions": manifest["smoke_test"],
            "broker_connection_started": False}


def build(destination: Path, downloads: Path) -> dict:
    if os.name != "nt":
        raise ValueError("Build and smoke-test this runtime on Windows x64")
    if destination.exists():
        raise ValueError("Choose a new runtime destination")
    downloads.mkdir(parents=True, exist_ok=True)
    destination.mkdir(parents=True)
    (destination / "INCOMPLETE").write_text("Runtime build in progress\n", encoding="utf-8")
    for artifact in ARTIFACTS:
        archive = downloads / artifact["url"].rsplit("/", 1)[1]
        if not archive.exists():
            temporary = archive.with_suffix(archive.suffix + ".part")
            with urllib.request.urlopen(artifact["url"], timeout=60) as response, temporary.open("xb") as output:
                shutil.copyfileobj(response, output)
            if file_hash(temporary) != artifact["sha256"]:
                raise ValueError("Upstream runtime archive hash mismatch")
            temporary.rename(archive)
        if file_hash(archive) != artifact["sha256"]:
            raise ValueError("Cached runtime archive hash mismatch")
        extract(archive, destination / artifact["target"])
    # Only the packaged sidecar directory and vendored modules are importable.
    # Environment PYTHONPATH, registry settings and user site remain isolated.
    (destination / "python313._pth").write_text(
        "python313.zip\n.\nLib/site-packages\n..\nimport site\n", encoding="ascii")
    script = ("import json,sys,struct,numpy,MetaTrader5; "
              "assert struct.calcsize('P') == 8; "
              "print(json.dumps({'python':sys.version.split()[0],"
              "'numpy':numpy.__version__,'MetaTrader5':MetaTrader5.__version__}))")
    process = subprocess.run([str(destination.resolve() / "python.exe"), "-I", "-B", "-c", script],
                             cwd=downloads.resolve(), capture_output=True, text=True, timeout=60)
    if process.returncode:
        raise ValueError("Portable runtime import smoke test failed")
    versions = json.loads(process.stdout)
    if versions != {"python": "3.13.15", "numpy": "2.5.1", "MetaTrader5": "5.0.5735"}:
        raise ValueError("Portable runtime resolved unexpected dependency versions")
    licenses = [p.relative_to(destination).as_posix() for p in destination.rglob("*")
                if p.is_file() and ("license" in p.name.lower() or "copying" in p.name.lower())]
    if len(licenses) < 3:
        raise ValueError("Upstream license payload incomplete")
    (destination / "INCOMPLETE").unlink()
    manifest = {"schema": SCHEMA, "artifacts": list(ARTIFACTS), "smoke_test": versions,
                "broker_connection_started": False, "licenses": sorted(licenses),
                "files": {p.relative_to(destination).as_posix(): file_hash(p)
                          for p in sorted(destination.rglob("*")) if p.is_file()}}
    (destination / "RUNTIME_MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return verify(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--downloads", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    if args.verify:
        result = verify(args.destination)
    elif args.downloads:
        result = build(args.destination, args.downloads)
    else:
        parser.error("--downloads is required when building")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
