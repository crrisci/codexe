"""Exercise local CodeXe activation through the upstream-derived shell installer."""

import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import unittest

INSTALL = Path(__file__).with_name("install.sh")


def manifest(package):
    lines = []
    for path in sorted(package.rglob("*")):
        if path.is_file() and path.name != "codexe-package_SHA256SUMS":
            lines.append(
                hashlib.sha256(path.read_bytes()).hexdigest()
                + "  "
                + path.relative_to(package).as_posix()
                + "\n"
            )
    (package / "codexe-package_SHA256SUMS").write_text("".join(lines))


class CodeXeLocalTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.package = self.root / "package"
        self.prefix = self.root / "storage with spaces"
        self.bin = self.root / "commands with spaces"
        self.home = self.root / "home"
        self.home.mkdir()
        self.audit = self.root / "forbidden-calls"
        guards = self.root / "guards"
        guards.mkdir()
        for name in ("curl", "wget", "python", "python3"):
            path = guards / name
            path.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$0" >> "$FORBIDDEN_CALLS"\nexit 99\n'
            )
            path.chmod(0o755)
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("CODEX_", "PREFIX"))
        }
        self.env.update(
            HOME=str(self.home),
            PATH=str(guards) + ":" + self.env["PATH"],
            FORBIDDEN_CALLS=str(self.audit),
        )
        self.metadata = {
            "layoutVersion": 1,
            "version": "1.2.3",
            "target": f"{platform.machine()}-unknown-linux-gnu",
            "variant": "codex",
            "entrypoint": "bin/codex",
            "execServer": "bin/codex-exec-server",
            "resourcesDir": "codex-resources",
            "pathDir": "codex-path",
            "xenolith": {"nativeConfig": 1},
        }
        files = ["LICENSE", "NOTICE"]
        files += [
            "third-party-notices/" + name
            for name in (
                "bubblewrap-COPYING",
                "ripgrep-LICENSE-MIT",
                "v8-LICENSE",
                "rusty-v8-LICENSE",
            )
        ]
        for name in files:
            path = self.package / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture notice\n")
        for name in (
            "bin/codex",
            "bin/codex-code-mode-host",
            "codex-resources/bwrap",
            "bin/codex-exec-server",
            "codex-path/rg",
        ):
            path = self.package / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("#!/bin/sh\nprintf 'codex-cli 1.2.3\\n'\n")
            path.chmod(0o755)
        (self.package / "install").mkdir()
        for name in ("install.sh", "codexe-local.sh"):
            shutil.copyfile(INSTALL.with_name(name), self.package / "install" / name)
        self.refresh()

    def refresh(self):
        (self.package / "codex-package.json").write_text(
            json.dumps(self.metadata, indent=2) + "\n"
        )
        manifest(self.package)

    def run_install(self, archive=None, extra=()):
        input_args = (
            ["--package-archive", str(archive)]
            if archive
            else ["--package-dir", str(self.package)]
        )
        result = subprocess.run(
            [
                "sh",
                str(INSTALL),
                "--codexe",
                *input_args,
                "--prefix",
                str(self.prefix),
                "--bin-dir",
                str(self.bin),
                *extra,
            ],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertFalse(
            self.audit.exists(), "Local install called a network/Python tool"
        )
        return result

    def success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_probes_isolate_client_home_before_and_after_activation(self):
        (self.home / ".codexe").write_text("existing conflicting file")
        original = self.root / "explicit-config"
        self.env["CODEX_HOME"] = str(original)
        for path in (self.package / "bin").iterdir():
            path.write_text(
                '#!/bin/sh\nmkdir -p "$CODEX_HOME" || exit 1\nprintf probe > "$CODEX_HOME/probe"\nprintf "codex-cli 1.2.3\\n"\n'
            )
        self.refresh()
        self.success(self.run_install())
        self.assertFalse(original.exists())
        self.assertEqual(
            (self.home / ".codexe").read_text(), "existing conflicting file"
        )

    def test_directory_install_rerun_preserves_commands_configs_and_profiles(self):
        self.bin.mkdir()
        (self.bin / "codex").write_text("unrelated upstream")
        (self.bin / ".codexe.unrelated").write_text("keep")
        for name in (".codex", ".codexe"):
            (self.home / name).mkdir()
            (self.home / name / "config.toml").write_text('model_provider="openai"\n')
        (self.home / ".profile").write_text("keep profile\n")
        self.success(self.run_install())
        current = self.prefix / "lib/codexe/standalone/current"
        self.assertEqual(os.readlink(self.bin / "codexe"), str(current / "bin/codex"))
        target = current.resolve()
        self.success(self.run_install())
        self.assertEqual(current.resolve(), target)
        self.assertEqual(len(list(current.parent.joinpath("releases").iterdir())), 1)
        self.assertEqual((self.bin / "codex").read_text(), "unrelated upstream")
        self.assertEqual((self.bin / ".codexe.unrelated").read_text(), "keep")
        self.assertEqual((self.home / ".profile").read_text(), "keep profile\n")
        for name in (".codex", ".codexe"):
            self.assertEqual(
                (self.home / name / "config.toml").read_text(),
                'model_provider="openai"\n',
            )

    def test_archive_install_and_changed_content_update_keep_previous_release(self):
        archive = self.root / "package.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for path in self.package.rglob("*"):
                tar.add(path, arcname=path.relative_to(self.package), recursive=False)
        self.success(self.run_install(archive))
        command = self.bin / "codexe"
        previous = command.resolve()
        (self.package / "NOTICE").write_text("updated notice\n")
        self.refresh()
        self.success(self.run_install())
        self.assertNotEqual(command.resolve(), previous)
        self.assertTrue(previous.is_file())

    def test_unrelated_regular_and_dangling_commands_are_never_overwritten(self):
        self.bin.mkdir()
        command = self.bin / "codexe"
        for symlink in (False, True):
            if symlink:
                command.symlink_to("/unrelated/missing")
            else:
                command.write_text("unrelated")
            result = self.run_install()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unrelated command", result.stderr)
            self.assertFalse(self.prefix.exists())
            command.unlink()

    def test_helper_failure_preserves_current_selection(self):
        self.success(self.run_install())
        previous = (self.bin / "codexe").resolve()
        (self.package / "bin/codex-exec-server").write_text(
            "#!/bin/sh\necho missing-library >&2\nexit 127\n"
        )
        self.refresh()
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing-library", result.stderr)
        self.assertEqual((self.bin / "codexe").resolve(), previous)

    def test_unmarked_or_wrong_target_package_is_rejected(self):
        for key, value in (
            ("xenolith", {"nativeConfig": False}),
            ("target", "other-unknown-linux-gnu"),
            ("entrypoint", "../foreign"),
        ):
            original = self.metadata[key]
            self.metadata[key] = value
            self.refresh()
            result = self.run_install()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.prefix.exists())
            self.metadata[key] = original

    def test_modified_installed_file_is_not_replaced(self):
        self.success(self.run_install())
        command = (self.bin / "codexe").resolve()
        command.write_text("user modification")
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Checksum mismatch", result.stderr)
        self.assertEqual(command.read_text(), "user modification")

    def test_payload_tampering_and_extra_files_fail_before_install(self):
        (self.package / "NOTICE").write_text("tampered")
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Checksum mismatch", result.stderr)
        self.refresh()
        (self.package / "unlisted").write_text("extra")
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("files differ", result.stderr)

    def test_traversal_and_links_in_archives_are_rejected_before_extraction(self):
        for index, name in enumerate(("../outside", "/absolute", "linked")):
            archive = self.root / f"bad{index}.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                member = tarfile.TarInfo(name)
                if name == "linked":
                    member.type = tarfile.SYMTYPE
                    member.linkname = "/tmp"
                    tar.addfile(member)
                else:
                    member.size = 3
                    tar.addfile(member, io.BytesIO(b"bad"))
            result = self.run_install(archive)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.prefix.exists())
        self.assertFalse((self.root / "outside").exists())

    def test_local_arguments_without_mode_do_not_download(self):
        result = subprocess.run(
            ["sh", str(INSTALL), "--package-dir", str(self.package)],
            env=self.env,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("require --codexe", result.stderr)
        self.assertFalse(self.audit.exists())


if __name__ == "__main__":
    unittest.main()
