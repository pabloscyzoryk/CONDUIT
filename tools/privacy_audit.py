"""Audit Git working files, staged blobs and history without printing private values.

All private inputs and the report destination are explicit command-line arguments.
Reports contain paths and finding categories, never matched text or credentials.
This conservative audit reports candidates for review; it is not a secrecy proof.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from collections import Counter
from pathlib import Path

PRIVATE_NAMES = re.compile(r"^(secrets|settings|channels|lancuchy)(?:\.|$)", re.I)
SENSITIVE_KEY = re.compile(
    r"password|passwd|secret|token|api_hash|api_id|session|phone|email|smtp|mail_(?:to|from|user)|"
    r"(?:^|_)(?:login|account|peer|channel|topic|chat|user)(?:_id|_name)?$|handle|mt5_server", re.I
)
OMIT_DIRS = {".git", "target", "node_modules", ".pnpm-store", "dist", "__pycache__"}
PATTERNS = {
    "private_absolute_owner_path": re.compile(r"(?:[A-Za-z]:[\\/]+Users[\\/]+(?!Public\b|Default\b|<|YOUR_|user\b|…|\.\.\.)[^\\/\s\"']+|[/]home[/](?!user\b|<|…|\.\.\.)[^/\s\"']+)", re.I),
    "telegram_bot_token": re.compile(r"\b\d{7,13}:[A-Za-z0-9_-]{30,}\b"),
    "github_token": re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b"),
    "private_key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "cloud_key": re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "openai_key": re.compile(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{30,}\b"),
    "credential_url": re.compile(r"\b[a-z]+://[^\s/:@]+:[^\s/@]+@", re.I),
    "email_address": re.compile(r"\b[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
    "literal_credential": re.compile(r"[\"']?(?:api_?hash|bot_?token|mt5_?password|smtp_?password|telegram_?session|password)[\"']?\s*[:=]\s*[\"']([^\"'\r\n]{5,})[\"']", re.I),
}
PLACEHOLDER = re.compile(r"example\.(?:com|org|net)|localhost|^invalid$|^(?:test|dummy|placeholder|redacted|synthetic)(?:$|[_ -])|^your[_ -]|^<|^(?:password|secret|change.?me)$", re.I)
DENIED_PATH = re.compile(r"(?:^|/)(?:VPSREADY[^/]*|node_modules|target|backup_memory|logs)(?:/|$)|(?:^|/)(?:secrets\.(?:json|bak)|telegram\.session[^/]*|\.env(?:\.[^/]*)?|alllogs[^/]*\.(?:txt|json))$", re.I)
BINARY_SUFFIXES = {".exe", ".dll", ".pdb", ".zip", ".7z", ".session", ".rlib", ".rmeta"}


def git(repo: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(repo), *args], stderr=subprocess.PIPE)


def git_blobs(repo: Path, entries):
    """Read immutable Git objects rather than a potentially different worktree."""
    entries = list(entries)
    if not entries:
        return
    batch = subprocess.check_output(
        ["git", "-C", str(repo), "cat-file", "--batch"],
        input="".join(oid + "\n" for oid, _ in entries).encode(), stderr=subprocess.PIPE,
    )
    offset = 0
    for oid, path in entries:
        end = batch.index(b"\n", offset)
        header = batch[offset:end].decode().split()
        if len(header) != 3 or header[0] != oid:
            raise ValueError("Git object could not be read for privacy audit")
        size = int(header[2])
        data = batch[end + 1:end + 1 + size]
        if len(data) != size:
            raise ValueError("Truncated Git object in privacy audit")
        offset = end + 1 + size + 1
        if header[1] == "blob":
            yield oid, path, data


def walk_files(root: Path):
    if root.is_file():
        yield root
        return
    for p in root.rglob("*"):
        if p.is_file() and not OMIT_DIRS.intersection(p.relative_to(root).parts):
            yield p


def extract_values(inputs: list[Path]):
    values: dict[str, set[str]] = {}
    errors = []
    seen = set()
    files_read = 0

    def add(value, key):
        if isinstance(value, bool) or value is None:
            return
        s = str(value).strip()
        minimum = 4 if key in {"handle", "user_name"} else 6
        if len(s) < minimum or s in {"0", "000000", "false", "true"}:
            return
        if PLACEHOLDER.search(s) or s.startswith(("http://127.0.0.1", "http://localhost")):
            return
        values.setdefault(s, set()).add(key)

    def visit(obj, trail=""):
        if isinstance(obj, dict):
            for k, v in obj.items():
                key = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", str(k)).lower()
                if SENSITIVE_KEY.search(key) and isinstance(v, (str, int, float)):
                    add(v, key)
                visit(v, trail + "." + key)
        elif isinstance(obj, list):
            for v in obj:
                visit(v, trail)

    for root in inputs:
        try:
            candidates = [root] if root.is_file() else list(root.glob("**/*"))
            for p in candidates:
                if not p.is_file() or not PRIVATE_NAMES.search(p.name) or p in seen:
                    continue
                if any(part in OMIT_DIRS for part in p.parts):
                    continue
                seen.add(p)
                try:
                    obj = json.loads(p.read_text(encoding="utf-8-sig"))
                    visit(obj)
                    files_read += 1
                except (OSError, UnicodeError, ValueError) as e:
                    errors.append({"stage": "private_config", "file_name": p.name, "error_type": type(e).__name__})
        except OSError as e:
            errors.append({"stage": "private_config", "error_type": type(e).__name__})
    return values, files_read, errors


def words(s: str):
    return re.findall(r"[^\W_]+", s.casefold(), flags=re.UNICODE)


def shingle_hashes(text: str, count: int):
    tokens = words(text)
    for i in range(len(tokens) - count + 1):
        yield hashlib.blake2b(" ".join(tokens[i:i + count]).encode(), digest_size=16).digest()


def chat_corpus(inputs: list[Path], count: int):
    corpus = set()
    messages = 0
    errors = []

    def visit(obj):
        nonlocal messages
        if isinstance(obj, dict):
            # Telegram Desktop exports retain message text as a string or rich-text list.
            if "text" in obj and ("date" in obj or "date_unixtime" in obj):
                t = obj["text"]
                text = t if isinstance(t, str) else "".join(x if isinstance(x, str) else str(x.get("text", "")) for x in t) if isinstance(t, list) else ""
                corpus.update(shingle_hashes(text, count))
                messages += 1
            for v in obj.values():
                if isinstance(v, (dict, list)):
                    visit(v)
        elif isinstance(obj, list):
            for v in obj:
                visit(v)

    for root in inputs:
        candidates = [root] if root.is_file() else root.glob("**/result.json")
        for p in candidates:
            try:
                visit(json.loads(p.read_text(encoding="utf-8-sig")))
            except (OSError, UnicodeError, ValueError) as e:
                errors.append({"stage": "chat_corpus", "file_name": p.name, "error_type": type(e).__name__})
    return corpus, messages, errors


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", type=Path, default=Path.cwd())
    ap.add_argument("--private-input", type=Path, action="append", default=[])
    ap.add_argument("--chat-input", type=Path, action="append", default=[])
    ap.add_argument("--report", type=Path, required=True)
    ap.add_argument("--allowlist", type=Path, help="Reviewed weak-pattern exceptions matched by relative path and full-line SHA-256.")
    ap.add_argument("--history", action="store_true")
    ap.add_argument("--index", action="store_true", help="Also scan the actual staged blobs, including content no longer present in the working tree.")
    ap.add_argument("--include-untracked", action="store_true", help="Also scan unignored new files before they are staged.")
    ap.add_argument("--shingle-words", type=int, default=12)
    args = ap.parse_args()
    repo = args.repo.resolve()
    values, config_count, errors = extract_values(args.private_input)
    corpus, message_count, chat_errors = chat_corpus(args.chat_input, args.shingle_words)
    errors.extend(chat_errors)
    findings = []
    reviewed = []
    allowed = {}
    if args.allowlist:
        for entry in json.loads(args.allowlist.read_text(encoding="utf-8")):
            if entry["category"] != "literal_credential":
                raise ValueError("Only weak literal-credential heuristics may be allowlisted.")
            allowed[(entry["path"], entry["category"], entry["line_sha256"])] = entry["reason"]
    scanned = Counter()
    exact = [(v, re.compile(r"(?<!\d)" + "_?".join(re.escape(x) for x in v) + r"(?!\d)")) if v.lstrip("-").isdigit() else (v, None) for v in values]

    def scan(data: bytes, path: str, origin: str):
        scanned[origin] += 1
        norm_path = path.replace("\\", "/")
        if DENIED_PATH.search(norm_path) or Path(norm_path).suffix.lower() in BINARY_SUFFIXES:
            findings.append({"origin": origin, "path": path, "category": "private_or_generated_path", "line": None})
        if data.startswith((b"\xff\xfe", b"\xfe\xff")):
            text = data.decode("utf-16", errors="replace")
        elif b"\x00" in data:
            return
        else:
            text = data.decode("utf-8-sig", errors="replace")
        for line_no, line in enumerate(text.splitlines(), 1):
            categories = set()
            for value, numeric_pattern in exact:
                if numeric_pattern.search(line) if numeric_pattern else value in line:
                    categories.add("exact_private_config_value")
            for category, pattern in PATTERNS.items():
                for match in pattern.finditer(line):
                    literal = match.group(1) if category == "literal_credential" else match.group(0)
                    if category in {"email_address", "literal_credential"} and PLACEHOLDER.search(literal):
                        continue
                    categories.add(category)
            if corpus and any(h in corpus for h in shingle_hashes(line, args.shingle_words)):
                categories.add("private_chat_shingle")
            for category in sorted(categories):
                digest = hashlib.sha256(line.encode()).hexdigest()
                item = {"origin": origin, "path": path, "category": category, "line": line_no, "line_sha256": digest}
                reason = allowed.get((path, category, digest))
                if reason:
                    reviewed.append({**item, "reason": reason})
                else:
                    findings.append(item)

    path_args = ["ls-files", "--cached", "-z"]
    if args.include_untracked:
        path_args.extend(["--others", "--exclude-standard"])
    paths = git(repo, *path_args).decode("utf-8").split("\0")
    for path in filter(None, paths):
        try:
            scan((repo / path).read_bytes(), path, "working_tree")
        except OSError as e:
            errors.append({"stage": "working_tree", "path": path, "error_type": type(e).__name__})
    print(json.dumps({"stage": "working_tree", "files": scanned["working_tree"], "candidate_findings": len(findings)}), flush=True)
    index_fingerprint = None
    if args.index:
        raw_index = git(repo, "ls-files", "--stage", "-z")
        index_fingerprint = hashlib.sha256(raw_index).hexdigest()
        entries = []
        for record in filter(None, raw_index.split(b"\0")):
            metadata, path_bytes = record.split(b"\t", 1)
            mode, oid, stage = metadata.decode("ascii").split()
            path = path_bytes.decode("utf-8")
            if stage != "0":
                errors.append({"stage": "index", "path": path, "error_type": "UnmergedIndexEntry"})
            if mode == "160000":
                errors.append({"stage": "index", "path": path, "error_type": "UnauditedSubmodule"})
                continue
            entries.append((oid, path))
        for oid, path, data in git_blobs(repo, entries):
            scan(data, path, "index")
        # A concurrent git add must not make a clean report look applicable to
        # a different index than the exact snapshot that was inspected.
        if raw_index != git(repo, "ls-files", "--stage", "-z"):
            errors.append({"stage": "index", "error_type": "IndexChangedDuringAudit"})
    commits = []
    if args.history:
        commits = git(repo, "rev-list", "--all").decode().splitlines()
        objects = git(repo, "rev-list", "--objects", "--all").decode("utf-8").splitlines()
        entries = [row.partition(" ") for row in objects]
        entries = [(oid, path) for oid, _, path in entries if path]
        for oid, path, data in git_blobs(repo, entries):
            scan(data, path, "history:" + oid[:12])
    report = {
        "schema_version": 1,
        "head": git(repo, "rev-parse", "HEAD").decode().strip(),
        "commit_count": len(commits),
        "private_config_files": config_count,
        "unique_comparison_values": len(values),
        "chat_messages": message_count,
        "chat_shingles": len(corpus),
        "shingle_words": args.shingle_words,
        "scanned_files_or_blobs": sum(scanned.values()),
        "working_files": scanned["working_tree"],
        "index_files": scanned["index"],
        "index_fingerprint": index_fingerprint,
        "findings_by_category": dict(Counter(x["category"] for x in findings)),
        "findings": findings,
        "reviewed_exception_count": len(reviewed),
        "reviewed_exceptions": reviewed,
        "errors": errors,
        "clean": not findings and not errors,
        "limits": ["Candidates require review; placeholders are filtered conservatively.", "Exact matching covers only supplied recognized private configuration fields.", "Chat matching covers supplied Telegram JSON message bodies and single source lines.", "Binary files are detected by path/extension but their content is not string-scanned."]
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k not in {"findings", "reviewed_exceptions", "errors", "limits"}}, ensure_ascii=True))
    return 0 if report["clean"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
