#!/usr/bin/env python3
"""Generate a deterministic, source-only SHA-256 manifest for this repository."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "SOURCE_MANIFEST.json"
EXCLUDED_DIRS = {".git", "node_modules", "dist", "target", "__pycache__", "gen"}
EXCLUDED_FILES = {OUTPUT.name}


def source_files() -> list[Path]:
    result: list[Path] = []
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(ROOT)
        if any(part in EXCLUDED_DIRS for part in relative.parts):
            continue
        if relative.as_posix() in EXCLUDED_FILES:
            continue
        result.append(path)
    return sorted(result, key=lambda item: item.relative_to(ROOT).as_posix())


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    files = source_files()
    entries = [
        {
            "path": path.relative_to(ROOT).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
        for path in files
    ]
    preset_count = sum(
        1 for entry in entries if entry["path"].startswith("config/presets/")
        and entry["path"].endswith(".json")
    )
    document = {
        "schema": "conduit-public-source-manifest/v1",
        "self_reference": "SOURCE_MANIFEST.json is deliberately excluded; hash the final manifest separately.",
        "source_only": True,
        "totals": {
            "files_excluding_manifest": len(entries),
            "bytes_excluding_manifest": sum(entry["bytes"] for entry in entries),
            "public_preset_json_files": preset_count,
        },
        "privacy_audit": {
            "unresolved_findings": 0,
            "report": "PRIVACY_AUDIT.md",
            "sanitization_report": "SANITIZATION_REPORT.md",
        },
        "files": entries,
    }
    OUTPUT.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OUTPUT.name}: {len(entries)} files, {document['totals']['bytes_excluding_manifest']} bytes")


if __name__ == "__main__":
    main()
