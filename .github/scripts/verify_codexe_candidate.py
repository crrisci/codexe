#!/usr/bin/env python3
"""Verify a previously built CodeXe release candidate without network access."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


WORKFLOW_PATH = ".github/workflows/codexe-release.yml"
ARCHIVE_SUFFIX = "-linux-x86_64-unknown-linux-gnu.tar.gz"
SEMVER = re.compile(
    r"(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)"
    r"(?:-(?:[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)
SHA256 = re.compile(r"[0-9a-f]{64}")
RUN_ID = re.compile(r"[1-9][0-9]*")
COMMIT = re.compile(r"[0-9a-f]{40}")


class CandidateError(ValueError):
    """Candidate metadata or artifact does not meet the promotion contract."""


def expected_archive_name(package_version: str) -> str:
    validate_package_version(package_version)
    return f"codexe-v{package_version}{ARCHIVE_SUFFIX}"


def validate_package_version(value: str) -> None:
    if not isinstance(value, str) or SEMVER.fullmatch(value) is None:
        raise CandidateError("package version must be a valid CodeXe semantic version")
    core = value.split("+", 1)[0].split("-", 1)[0]
    if any(int(part) > 2**64 - 1 for part in core.split(".")):
        raise CandidateError("package version numeric components exceed the supported range")
    prerelease = value.split("+", 1)[0].partition("-")[2]
    if any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in prerelease.split(".")):
        raise CandidateError("numeric prerelease identifiers must not have leading zeroes")


def validate_inputs(package_version: str, candidate_run_id: str, approved_archive_sha256: str) -> None:
    validate_package_version(package_version)
    if not isinstance(candidate_run_id, str) or RUN_ID.fullmatch(candidate_run_id) is None:
        raise CandidateError("candidate run ID must be a positive decimal integer without leading zeroes")
    if not isinstance(approved_archive_sha256, str) or SHA256.fullmatch(approved_archive_sha256) is None:
        raise CandidateError("approved archive SHA-256 must be 64 lowercase hexadecimal characters")


def _read_json(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise CandidateError(f"could not read {label} JSON") from exc
    if not isinstance(value, dict):
        raise CandidateError(f"{label} JSON must contain an object")
    return value


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise CandidateError(message)


def verify_candidate(
    *,
    run_metadata_path: Path,
    provenance_path: Path,
    archive_path: Path,
    checksums_path: Path,
    expected_repository: str,
    expected_sha: str,
    expected_run_id: str,
    package_version: str,
    approved_archive_sha256: str,
    expected_workflow: str = WORKFLOW_PATH,
) -> str:
    """Validate API metadata, candidate provenance, archive identity and digest."""
    validate_inputs(package_version, expected_run_id, approved_archive_sha256)
    _require(bool(expected_repository) and "/" in expected_repository, "expected repository is invalid")
    _require(COMMIT.fullmatch(expected_sha) is not None, "expected source SHA must be a full commit SHA")
    _require(expected_workflow == WORKFLOW_PATH, "expected workflow path is not the CodeXe release workflow")

    run = _read_json(run_metadata_path, "workflow run")
    _require(str(run.get("id")) == expected_run_id, "workflow run ID does not match the selected candidate")
    repository = run.get("repository")
    head_repository = run.get("head_repository")
    _require(
        isinstance(repository, dict) and repository.get("full_name") == expected_repository,
        "candidate run belongs to a different repository",
    )
    _require(
        isinstance(head_repository, dict) and head_repository.get("full_name") == expected_repository,
        "candidate source repository does not match this repository",
    )
    _require(run.get("event") == "workflow_dispatch", "candidate must come from a manual workflow dispatch")
    path = run.get("path")
    _require(
        isinstance(path, str) and path.split("@", 1)[0] == expected_workflow,
        "candidate was not built by the CodeXe release workflow",
    )
    _require(run.get("status") == "completed", "candidate workflow run is not completed")
    _require(run.get("conclusion") == "success", "candidate workflow run did not succeed")
    _require(run.get("head_sha") == expected_sha, "candidate source SHA differs from this publish dispatch")

    provenance = _read_json(provenance_path, "candidate provenance")
    _require(provenance.get("baseGitCommit") == expected_sha, "candidate provenance source SHA does not match")
    _require(
        provenance.get("derivedPackageVersion") == package_version,
        "candidate provenance package version does not match",
    )
    _require(
        provenance.get("target") == "x86_64-unknown-linux-gnu",
        "candidate provenance target is not the supported Linux GNU target",
    )

    archive_name = expected_archive_name(package_version)
    _require(archive_path.name == archive_name, "candidate archive name does not match the requested version")
    _require(not archive_path.is_symlink() and archive_path.is_file(), "candidate archive must be a regular file")
    _require(
        not checksums_path.is_symlink() and checksums_path.is_file(),
        "candidate checksum manifest must be a regular file",
    )
    try:
        manifest = checksums_path.read_text(encoding="utf-8")
        archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    except (OSError, UnicodeError) as exc:
        raise CandidateError("could not read candidate archive or checksum manifest") from exc

    lines = manifest.splitlines()
    _require(len(lines) == 1, "checksum manifest must contain exactly the expected archive")
    match = re.fullmatch(r"([0-9a-f]{64})[ \t]+([^\s]+)", lines[0])
    _require(match is not None, "checksum manifest line is malformed")
    manifest_digest, manifest_name = match.groups()
    _require(manifest_name == archive_name, "checksum manifest contains an unexpected path or filename")
    _require(manifest_digest == archive_digest, "candidate archive does not match its checksum manifest")
    _require(
        archive_digest == approved_archive_sha256,
        "candidate archive does not match the explicitly approved SHA-256",
    )
    return archive_digest


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    inputs = subparsers.add_parser("validate-inputs", help="validate user-supplied promotion inputs")
    inputs.add_argument("--package-version", required=True)
    inputs.add_argument("--candidate-run-id", required=True)
    inputs.add_argument("--approved-archive-sha256", required=True)

    verify = subparsers.add_parser("verify-candidate", help="verify downloaded candidate metadata and files")
    verify.add_argument("--run-metadata", type=Path, required=True)
    verify.add_argument("--provenance", type=Path, required=True)
    verify.add_argument("--archive", type=Path, required=True)
    verify.add_argument("--checksums", type=Path, required=True)
    verify.add_argument("--expected-repository", required=True)
    verify.add_argument("--expected-sha", required=True)
    verify.add_argument("--expected-run-id", required=True)
    verify.add_argument("--package-version", required=True)
    verify.add_argument("--approved-archive-sha256", required=True)
    verify.add_argument("--expected-workflow", default=WORKFLOW_PATH)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "validate-inputs":
            validate_inputs(args.package_version, args.candidate_run_id, args.approved_archive_sha256)
        else:
            digest = verify_candidate(
                run_metadata_path=args.run_metadata,
                provenance_path=args.provenance,
                archive_path=args.archive,
                checksums_path=args.checksums,
                expected_repository=args.expected_repository,
                expected_sha=args.expected_sha,
                expected_run_id=args.expected_run_id,
                package_version=args.package_version,
                approved_archive_sha256=args.approved_archive_sha256,
                expected_workflow=args.expected_workflow,
            )
            print(f"Verified candidate archive SHA-256: {digest}")
    except CandidateError as exc:
        print(f"candidate verification failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
