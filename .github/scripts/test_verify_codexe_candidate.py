"""Offline tests for CodeXe candidate promotion validation."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import subprocess
import re
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
VERIFIER_PATH = ROOT / ".github/scripts/verify_codexe_candidate.py"
SPEC = importlib.util.spec_from_file_location("verify_codexe_candidate", VERIFIER_PATH)
assert SPEC is not None and SPEC.loader is not None
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class CandidateVerifierTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.version = "1.2.3-beta.1"
        self.run_id = "123456"
        self.repository = "crrisci/codexe"
        self.sha = "a" * 40
        self.archive_name = verifier.expected_archive_name(self.version)
        self.archive = self.root / self.archive_name
        self.archive_bytes = b"trusted candidate archive fixture\n"
        self.archive.write_bytes(self.archive_bytes)
        self.digest = hashlib.sha256(self.archive_bytes).hexdigest()
        self.run_metadata = {
            "id": int(self.run_id),
            "repository": {"full_name": self.repository},
            "head_repository": {"full_name": self.repository},
            "event": "workflow_dispatch",
            "path": ".github/workflows/codexe-release.yml@refs/heads/main",
            "status": "completed",
            "conclusion": "success",
            "head_sha": self.sha,
        }
        self.provenance = {
            "baseGitCommit": self.sha,
            "derivedPackageVersion": self.version,
            "target": "x86_64-unknown-linux-gnu",
        }
        self.run_path = self.root / "run.json"
        self.provenance_path = self.root / "provenance.json"
        self.checksums_path = self.root / "SHA256SUMS"
        self._write_json(self.run_path, self.run_metadata)
        self._write_json(self.provenance_path, self.provenance)
        self.checksums_path.write_text(f"{self.digest}  {self.archive_name}\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    @staticmethod
    def _write_json(path: Path, value: dict) -> None:
        path.write_text(json.dumps(value), encoding="utf-8")

    def _verify(self, **overrides) -> str:
        args = {
            "run_metadata_path": self.run_path,
            "provenance_path": self.provenance_path,
            "archive_path": self.archive,
            "checksums_path": self.checksums_path,
            "expected_repository": self.repository,
            "expected_sha": self.sha,
            "expected_run_id": self.run_id,
            "package_version": self.version,
            "approved_archive_sha256": self.digest,
        }
        args.update(overrides)
        return verifier.verify_candidate(**args)

    def _assert_run_rejected(self, **changes) -> None:
        metadata = dict(self.run_metadata)
        metadata.update(changes)
        self._write_json(self.run_path, metadata)
        with self.assertRaises(verifier.CandidateError):
            self._verify()

    def test_accepts_exact_candidate(self) -> None:
        self.assertEqual(self._verify(), self.digest)

    def test_rejects_wrong_repository(self) -> None:
        self._assert_run_rejected(repository={"full_name": "other/project"})

    def test_rejects_wrong_source_repository(self) -> None:
        self._assert_run_rejected(head_repository={"full_name": "other/project"})

    def test_rejects_wrong_workflow(self) -> None:
        self._assert_run_rejected(path=".github/workflows/python-sdk-release.yml")

    def test_rejects_wrong_source_sha(self) -> None:
        self._assert_run_rejected(head_sha="b" * 40)

    def test_rejects_wrong_run_id(self) -> None:
        self._assert_run_rejected(id=654321)

    def test_rejects_wrong_status_or_conclusion(self) -> None:
        for field, value in (("status", "in_progress"), ("conclusion", "failure")):
            with self.subTest(field=field):
                metadata = dict(self.run_metadata)
                metadata[field] = value
                self._write_json(self.run_path, metadata)
                with self.assertRaises(verifier.CandidateError):
                    self._verify()

    def test_rejects_non_dispatch_run(self) -> None:
        self._assert_run_rejected(event="push")

    def test_rejects_wrong_provenance_sha_or_version(self) -> None:
        for field, value in (("baseGitCommit", "b" * 40), ("derivedPackageVersion", "9.9.9")):
            with self.subTest(field=field):
                provenance = dict(self.provenance)
                provenance[field] = value
                self._write_json(self.provenance_path, provenance)
                with self.assertRaises(verifier.CandidateError):
                    self._verify()

    def test_rejects_wrong_target(self) -> None:
        provenance = dict(self.provenance, target="aarch64-unknown-linux-gnu")
        self._write_json(self.provenance_path, provenance)
        with self.assertRaises(verifier.CandidateError):
            self._verify()

    def test_rejects_wrong_approved_digest(self) -> None:
        with self.assertRaises(verifier.CandidateError):
            self._verify(approved_archive_sha256="0" * 64)

    def test_rejects_manifest_digest_mismatch(self) -> None:
        self.checksums_path.write_text(f"{'0' * 64}  {self.archive_name}\n", encoding="utf-8")
        with self.assertRaises(verifier.CandidateError):
            self._verify()

    def test_rejects_extra_manifest_entries_and_paths(self) -> None:
        for manifest in (
            f"{self.digest}  {self.archive_name}\n{self.digest}  another.tar.gz\n",
            f"{self.digest}  ../{self.archive_name}\n",
            f"{self.digest}  /tmp/{self.archive_name}\n",
        ):
            with self.subTest(manifest=manifest):
                self.checksums_path.write_text(manifest, encoding="utf-8")
                with self.assertRaises(verifier.CandidateError):
                    self._verify()

    def test_rejects_wrong_archive_name_and_symlink(self) -> None:
        wrong = self.root / "codexe-v9.9.9-linux-x86_64-unknown-linux-gnu.tar.gz"
        wrong.write_bytes(self.archive_bytes)
        with self.assertRaises(verifier.CandidateError):
            self._verify(archive_path=wrong)
        link = self.root / self.archive_name
        link.unlink()
        link.symlink_to(wrong)
        with self.assertRaises(verifier.CandidateError):
            self._verify(archive_path=link)

    def test_rejects_invalid_promotion_inputs(self) -> None:
        invalid = [
            ("1.02.3", self.run_id, self.digest),
            (self.version, "0", self.digest),
            (self.version, "00123", self.digest),
            (self.version, self.run_id, "xyz"),
        ]
        for version, run_id, digest in invalid:
            with self.subTest(version=version, run_id=run_id, digest=digest):
                with self.assertRaises(verifier.CandidateError):
                    verifier.validate_inputs(version, run_id, digest)


class WorkflowContractTests(unittest.TestCase):
    def test_workflow_promotes_only_a_prior_run_candidate(self) -> None:
        workflow = (ROOT / ".github/workflows/codexe-release.yml").read_text(encoding="utf-8")
        build = self._job_section(workflow, "build-linux-gnu")
        publish = self._job_section(workflow, "publish")
        self.assertIn("if: ${{ !inputs.publish }}", build)
        self.assertIn("if: ${{ inputs.publish && (github.ref == 'refs/heads/main' || startsWith(github.ref, 'refs/heads/release/')) }}", publish)
        self.assertIn("environment: release", publish)
        self.assertNotRegex(publish, r"(?m)^    needs:.*build")
        self.assertIn("actions: read", publish)
        self.assertIn("contents: write", publish)
        self.assertIn("ref: ${{ github.sha }}", publish)
        self.assertIn("persist-credentials: false", publish)
        self.assertIn("run-id: ${{ inputs.candidate_run_id }}", publish)
        self.assertIn("repository: ${{ github.repository }}", publish)
        self.assertIn("github-token: ${{ github.token }}", publish)
        self.assertIn("approved_archive_sha256:", workflow)
        self.assertIn("candidate_run_id:", workflow)
        self.assertNotIn("${{ inputs.", "\n".join(self._run_lines(publish)))
        self.assertNotRegex(workflow, r"(?m)^\s+run:.*\$\{\{\s*inputs\.")
        self.assertIn("CANDIDATE_RUN_ID: ${{ inputs.candidate_run_id }}", publish)
        self.assertIn("APPROVED_ARCHIVE_SHA256: ${{ inputs.approved_archive_sha256 }}", publish)
        self.assertIn("--latest=false", publish)
        self.assertIn('gh release create "${tag}" --draft', publish)
        self.assertIn('gh release upload "${tag}"', publish)
        self.assertIn('gh release edit "${tag}" --draft=false', publish)
        self.assertLess(publish.index('gh release create'), publish.index('gh release upload'))
        self.assertLess(publish.index('gh release upload'), publish.index('gh release edit'))
        self.assertNotIn('--clobber', publish)

    def test_failed_asset_upload_leaves_release_draft(self) -> None:
        workflow = (ROOT / ".github/workflows/codexe-release.yml").read_text()
        publish = self._job_section(workflow, "publish")
        final_step = publish[publish.index("      - name: Create draft,"):]
        shell = final_step.split("        run: |\n", 1)[1]
        shell = "\n".join(line[10:] for line in shell.splitlines())
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            gh = root / "gh"
            gh.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$GH_CALL_LOG"\nif [ "$1 $2" = "release upload" ] && [ "$GH_FAIL_UPLOAD" = 1 ]; then exit 71; fi\n')
            gh.chmod(0o755)
            log = root / "calls"
            env = {**os.environ, "PATH": str(root) + ":" + os.environ["PATH"],
                   "GH_CALL_LOG": str(log), "GH_FAIL_UPLOAD": "1", "PACKAGE_VERSION": "1.2.3-rc.1",
                   "GITHUB_WORKSPACE": str(root), "GITHUB_SHA": "a" * 40,
                   "GITHUB_REPOSITORY": "owner/codexe"}
            result = subprocess.run(["bash", "-c", shell], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 71, result.stderr)
            calls = log.read_text().splitlines()
            self.assertEqual(len(calls), 2)
            self.assertIn("release create", calls[0])
            self.assertIn("--draft", calls[0])
            self.assertIn("release upload", calls[1])
            self.assertNotIn("release edit", log.read_text())
            log.unlink()
            env["GH_FAIL_UPLOAD"] = "0"
            result = subprocess.run(["bash", "-c", shell], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = log.read_text().splitlines()
            self.assertEqual(len(calls), 3)
            self.assertIn("release create", calls[0])
            self.assertIn("release upload", calls[1])
            self.assertIn("release edit", calls[2])
            self.assertIn("--draft=false", calls[2])

    @staticmethod
    def _job_section(workflow: str, name: str) -> str:
        match = re.search(rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:|\Z)", workflow)
        if match is None:
            raise AssertionError(f"workflow job {name!r} not found")
        return match.group(1)

    @staticmethod
    def _run_lines(job: str) -> list[str]:
        lines: list[str] = []
        in_run = False
        for line in job.splitlines():
            if re.match(r"^      - name:|^      - uses:", line):
                in_run = False
            if re.match(r"^        run:", line):
                in_run = True
                continue
            if in_run:
                lines.append(line)
        return lines

    def test_verifier_has_no_network_client_or_process_execution(self) -> None:
        source = VERIFIER_PATH.read_text(encoding="utf-8")
        self.assertNotRegex(
            source,
            r"(?m)^\s*(?:import|from)\s+(?:urllib|requests|httpx|http|socket|subprocess)\b",
        )


if __name__ == "__main__":
    unittest.main()
