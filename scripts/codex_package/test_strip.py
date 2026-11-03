"""Exercise staged Linux ELF stripping without Cargo, downloads or inference."""

import contextlib
import hashlib
import io
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from codex_package import cli


class PackageStripTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="codexe-strip-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.output = self.root / "package"

    def args(self, binary, *extra):
        argv = [
            "builder",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--package-version",
            "1.2.3",
            "--package-dir",
            str(self.output),
            "--attest-xenolith-native-config",
        ]
        for option in (
            "--entrypoint-bin",
            "--code-mode-host-bin",
            "--exec-server-bin",
            "--bwrap-bin",
            "--rg-bin",
        ):
            argv += [option, str(binary)]
        return [*argv, *extra]

    def invoke(self, argv):
        with mock.patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
            return cli.main()

    def tiny_elf(self):
        binary = self.root / "original"
        subprocess.run(
            ["cc", "-g", "-O0", "-x", "c", "-o", str(binary), "-"],
            input=b'#include <stdio.h>\nint main(void) { puts("tiny package fixture"); return 0; }\n',
            check=True,
            capture_output=True,
        )
        return binary

    @unittest.skipUnless(
        shutil.which("cc") and shutil.which("strip") and shutil.which("readelf"),
        "needs local C compiler and binutils",
    )
    def test_real_elf_copies_stripped_before_checksums_original_preserved(self):
        binary = self.tiny_elf()
        original = binary.read_bytes()
        self.assertIn(
            b".debug_info", subprocess.check_output(["readelf", "-S", str(binary)])
        )
        for strip in (False, True):
            self.output = self.root / ("stripped" if strip else "default")
            self.invoke(self.args(binary, *(["--strip"] if strip else [])))
            manifest = self.output / "codexe-package_SHA256SUMS"
            for line in manifest.read_text().splitlines():
                digest, name = line.split("  ", 1)
                self.assertEqual(
                    digest,
                    hashlib.sha256((self.output / name).read_bytes()).hexdigest(),
                )
            for name in (
                "bin/codex",
                "bin/codex-code-mode-host",
                "bin/codex-exec-server",
                "codex-resources/bwrap",
                "codex-path/rg",
            ):
                staged = self.output / name
                if strip:
                    self.assertLess(staged.stat().st_size, len(original))
                    self.assertNotIn(
                        b".debug_info",
                        subprocess.check_output(["readelf", "-S", str(staged)]),
                    )
                else:
                    self.assertEqual(staged.read_bytes(), original)
                self.assertEqual(
                    subprocess.check_output([str(staged)]), b"tiny package fixture\n"
                )
            self.assertEqual(binary.read_bytes(), original)

    def test_missing_strip_fails_before_build_or_output(self):
        with (
            mock.patch.object(cli.shutil, "which", return_value=None),
            mock.patch.object(cli, "build_source_binaries") as build,
            mock.patch.object(cli.tempfile, "mkdtemp") as mkdir,
        ):
            with self.assertRaisesRegex(RuntimeError, "requires the binutils strip"):
                self.invoke(
                    ["builder", "--target", "x86_64-unknown-linux-gnu", "--strip"]
                )
            build.assert_not_called()
            mkdir.assert_not_called()

    def test_non_linux_target_fails_before_build_or_output(self):
        with (
            mock.patch.object(cli, "build_source_binaries") as build,
            mock.patch.object(cli.tempfile, "mkdtemp") as mkdir,
        ):
            with self.assertRaisesRegex(RuntimeError, "Linux package targets only"):
                self.invoke(["builder", "--target", "aarch64-apple-darwin", "--strip"])
            build.assert_not_called()
            mkdir.assert_not_called()

    @unittest.skipUnless(shutil.which("cc"), "needs local C compiler")
    def test_failed_strip_does_not_emit_success_manifest_or_modify_inputs(self):
        binary = self.tiny_elf()
        original = binary.read_bytes()
        failing = self.root / "failing-strip"
        failing.write_text('#!/bin/sh\nprintf "fixture strip failed\\n" >&2\nexit 2\n')
        failing.chmod(0o755)
        with mock.patch.object(cli.shutil, "which", return_value=str(failing)):
            with self.assertRaisesRegex(
                RuntimeError, "Cannot strip staged package file.*fixture strip failed"
            ):
                self.invoke(self.args(binary, "--strip"))
        self.assertFalse((self.output / "codexe-package_SHA256SUMS").exists())
        self.assertEqual(binary.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
