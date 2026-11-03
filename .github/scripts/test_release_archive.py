"""Build a tiny canonical CodeXe archive and consume it with the real installer."""

from __future__ import annotations

import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
BUILDER = REPO_ROOT / "scripts/build_codex_package.py"
INSTALLER = REPO_ROOT / "scripts/install/install.sh"
WORKFLOW = REPO_ROOT / ".github/workflows/codexe-release.yml"


class ReleaseArchiveTest(unittest.TestCase):
    def setUp(self) -> None:
        if platform.system() != "Linux":
            self.skipTest("The CodeXe local package installer supports Linux/WSL only")
        architecture = {"x86_64": "x86_64", "aarch64": "aarch64"}.get(platform.machine())
        if architecture is None:
            self.skipTest(f"Unsupported fixture host architecture: {platform.machine()}")
        self.target = f"{architecture}-unknown-linux-gnu"
        self.temporary = tempfile.TemporaryDirectory(prefix="codexe-release-archive-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.inputs = self.root / "fixture-inputs"
        self.package = self.root / "codexe-candidate"
        self.archive = self.root / "codexe-v1.2.3-linux-fixture.tar.gz"
        self.prefix = self.root / "prefix"
        self.bindir = self.root / "bin"
        self.home = self.root / "home"
        self.home.mkdir()
        self.guard_bin = self.root / "guard-bin"
        self.guard_bin.mkdir()
        self.forbidden_calls = self.root / "forbidden-network-tools"

        fixture_programs = {
            "codex": "codex-cli 1.2.3",
            "codex-code-mode-host": "fixture code-mode host",
            "codex-exec-server": "fixture exec server",
            "bwrap": "fixture bubblewrap",
            "rg": "fixture ripgrep",
        }
        for name, output in fixture_programs.items():
            path = self.inputs / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"#!/bin/sh\nprintf '%s\\n' {output!r}\n", encoding="utf-8")
            path.chmod(0o755)

        for name in ("curl", "wget"):
            guard = self.guard_bin / name
            guard.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$0" >> "$FORBIDDEN_CALLS"\nexit 99\n',
                encoding="utf-8",
            )
            guard.chmod(0o755)

    def test_workflow_archive_is_flat_and_installs_from_archive(self) -> None:
        build_env = dict(os.environ, CODEX_REPO_ROOT=str(REPO_ROOT))
        subprocess.run(
            [
                sys.executable,
                str(BUILDER),
                "--target",
                self.target,
                "--package-version",
                "1.2.3",
                "--package-dir",
                str(self.package),
                "--entrypoint-bin",
                str(self.inputs / "codex"),
                "--attest-xenolith-native-config",
                "--code-mode-host-bin",
                str(self.inputs / "codex-code-mode-host"),
                "--exec-server-bin",
                str(self.inputs / "codex-exec-server"),
                "--bwrap-bin",
                str(self.inputs / "bwrap"),
                "--rg-bin",
                str(self.inputs / "rg"),
                "--archive-output",
                str(self.archive),
            ],
            cwd=REPO_ROOT,
            env=build_env,
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
        )

        with tarfile.open(self.archive, "r:gz") as archive:
            names = {member.name.rstrip("/") for member in archive.getmembers()}
        for required in (
            "bin/codex",
            "bin/codex-code-mode-host",
            "bin/codex-exec-server",
            "codex-resources/bwrap",
            "codex-path/rg",
            "codex-package.json",
            "codexe-package_SHA256SUMS",
            "install/install.sh",
            "install/codexe-local.sh",
        ):
            self.assertIn(required, names)
        self.assertFalse(
            any(name.startswith("codexe-candidate/") for name in names),
            "the archive must contain the canonical package root, not a wrapper directory",
        )
        metadata = json.loads(self.package.joinpath("codex-package.json").read_text())
        self.assertEqual(metadata["target"], self.target)
        self.assertEqual(metadata["xenolith"], {"nativeConfig": 1})

        installer_env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("CODEX_", "PREFIX"))
        }
        installer_env.update(
            HOME=str(self.home),
            CODEX_HOME=str(self.home / ".codexe"),
            CODEX_NON_INTERACTIVE="true",
            PATH=f"{self.guard_bin}:{os.environ['PATH']}",
            FORBIDDEN_CALLS=str(self.forbidden_calls),
        )
        result = subprocess.run(
            [
                "sh",
                str(INSTALLER),
                "--codexe",
                "--package-archive",
                str(self.archive),
                "--prefix",
                str(self.prefix),
                "--bin-dir",
                str(self.bindir),
            ],
            cwd=self.root,
            env=installer_env,
            check=False,
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.forbidden_calls.exists(), "archive installation attempted a network tool")
        self.assertTrue((self.bindir / "codexe").is_symlink())
        installed = subprocess.run(
            [str(self.bindir / "codexe"), "--version"],
            cwd=self.root,
            env=installer_env,
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(installed.stdout.strip(), "codex-cli 1.2.3")

        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("--archive-output", workflow)
        self.assertIn("--package-archive \"${archive}\"", workflow)
        self.assertNotIn('tar -C "${GITHUB_WORKSPACE}/dist"', workflow)


if __name__ == "__main__":
    unittest.main()
