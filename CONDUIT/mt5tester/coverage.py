"""Audit every preset in a sweep against the current native EA contract.

No experiment is launched. Missing active behavior is an error, while known
parameters of disabled features remain classified as inactive.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

import contract


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--space", type=Path, required=True)
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    manifest_path = args.space / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    defaults = json.loads(args.defaults.read_text(encoding="utf-8-sig"))
    source_path = Path(__file__).resolve().parents[1] / "mql5/CONDUIT_XT.mq5"
    source_bytes = source_path.read_bytes()
    source = source_bytes.decode("utf-8")
    source_sha256 = hashlib.sha256(source_bytes).hexdigest()
    available = set(re.findall(r"^\s*input\s+\w+\s+(In_\w+)", source, re.M))
    executable = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
    targets = {name: target for name, target, _ in contract.MAP}
    targets.update({name: target for name, (target, _) in contract.ENUMY.items()})
    changed_axes = {key for candidate in manifest["candidates"] for key in candidate["changes"]}
    axis_implementation = {}
    for key in sorted(changed_axes):
        target = targets.get(key)
        uses = len(re.findall(r"\b" + target + r"\b", executable)) - 1 if target else 0
        axis_implementation[key] = {"native_input": target, "code_references_after_declaration": max(uses, 0),
                                    "behavioral_parity_proven_by_source_scan": False}
    counts, unsupported, candidates = Counter(), Counter(), []
    for candidate in manifest["candidates"]:
        name = candidate["id"]
        if Path(name).name != name or any(c in name for c in "/\\:"):
            raise ValueError("Candidate id must be a plain filename stem.")
        path = args.space / "configs" / (name + ".json")
        preset = json.loads(path.read_text(encoding="utf-8-sig"))
        _, report = contract.build(defaults, preset["settings"], source, "synthetic_bridge.csv", available)
        family = candidate["family"]
        status = "mapped_requires_execution_proof" if report["mapping_complete"] else "unsupported_active_setting"
        counts[family + ":" + status] += 1
        unsupported.update(error["field"] for error in report["errors"])
        candidates.append({"id": name, "family": family, "status": status,
                           "preset_sha256": contract.fingerprint(path), "errors": report["errors"],
                           "changed_axes": {key: report["fields"].get(key, "missing")
                                            for key in candidate["changes"]}})
    result = {"schema": "conduit.mt5.coverage.v1", "execution_parity_proven": False,
              "manifest_sha256": contract.fingerprint(manifest_path),
              "defaults_sha256": contract.fingerprint(args.defaults),
              "expert_sha256": source_sha256,
              "candidate_count": len(candidates), "family_status_counts": dict(counts),
              "unsupported_field_counts": dict(unsupported),
              "changed_axis_implementation": axis_implementation, "candidates": candidates}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({key: value for key, value in result.items() if key not in {"candidates", "changed_axis_implementation"}}), flush=True)


if __name__ == "__main__":
    main()
