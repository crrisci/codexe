#!/usr/bin/env python3
"""Derive a candidate version in a disposable build checkout only."""
import json
import hashlib
import os
from pathlib import Path
import re
import sys
import tomllib

root = Path(__file__).resolve().parents[1]
os.environ["CODEX_REPO_ROOT"] = str(root)
sys.path.insert(0, str(root / "scripts"))
from codex_package.cli import parse_package_version

version = parse_package_version(sys.argv[1])
manifest = root / "codex-rs" / "Cargo.toml"
original = manifest.read_text(encoding="utf-8")
source_version = tomllib.loads(original)["workspace"]["package"]["version"]
lines = original.splitlines(keepends=True)
in_workspace_package = False
matches = []
for index, line in enumerate(lines):
    stripped = line.strip()
    if stripped == "[workspace.package]":
        in_workspace_package = True
        continue
    if in_workspace_package and stripped.startswith("["):
        in_workspace_package = False
    if in_workspace_package and re.match(r"^\s*version\s*=", line):
        matches.append(index)
if len(matches) != 1:
    raise SystemExit("expected exactly one [workspace.package].version line")

index = matches[0]
line = lines[index]
ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
body = line[:-len(ending)] if ending else line
match = re.fullmatch(r'(?P<prefix>\s*version\s*=\s*)"[^"]+"(?P<suffix>\s*(?:#.*)?)', body)
if match is None:
    raise SystemExit("could not safely replace workspace package version")
lines[index] = f'{match.group("prefix")}{json.dumps(version)}{match.group("suffix")}{ending}'
manifest.write_text("".join(lines), encoding="utf-8")
derived_version = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"]["package"]["version"]
if derived_version != version:
    raise SystemExit("workspace version replacement did not match the validated input")

provenance = {
    "baseGitCommit": os.environ.get("STABLE_GIT_COMMIT", os.environ.get("GITHUB_SHA")),
    "sourceWorkspaceVersion": source_version,
    "derivedPackageVersion": derived_version,
    "preparedCargoTomlSha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
    "target": "x86_64-unknown-linux-gnu",
}
provenance_path = root / "dist" / "codexe-release-provenance.json"
provenance_path.parent.mkdir(parents=True, exist_ok=True)
provenance_path.write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
