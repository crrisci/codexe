"""Capability attestation must reflect the built source or explicit provenance."""

from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from codex_package import cli
from codex_package.native import write_native_checksums


class NativePackageTest(unittest.TestCase):
    def package(self, *, prebuilt, attested, native_source=True):
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            binary = root / "binary"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            output = root / "package"
            arguments = [
                "builder",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--package-dir",
                str(output),
                "--package-version",
                "1.2.3",
            ]
            if prebuilt:
                arguments += ["--entrypoint-bin", str(binary)]
            if attested:
                arguments += ["--attest-xenolith-native-config"]
            stack.enter_context(mock.patch("sys.argv", arguments))
            stack.enter_context(
                mock.patch.object(
                    cli, "source_supports_native_config", return_value=native_source
                )
            )
            source = SimpleNamespace(
                entrypoint_bin=binary,
                code_mode_host_bin=binary,
                exec_server_bin=binary,
                bwrap_bin=binary,
                codex_command_runner_bin=None,
                codex_windows_sandbox_setup_bin=None,
            )
            stack.enter_context(
                mock.patch.object(cli, "build_source_binaries", return_value=source)
            )
            stack.enter_context(
                mock.patch.object(cli, "resolve_rg_bin", return_value=binary)
            )
            cli.main()
            metadata = json.loads((output / "codex-package.json").read_text())
            manifest = output / "codexe-package_SHA256SUMS"
            if "xenolith" in metadata:
                hashes = dict(
                    line.split("  ", 1)[::-1]
                    for line in manifest.read_text().splitlines()
                )
                self.assertEqual(
                    set(hashes),
                    {
                        str(path.relative_to(output))
                        for path in output.rglob("*")
                        if path.is_file() and path != manifest
                    },
                )
                for name, digest in hashes.items():
                    self.assertEqual(
                        hashlib.sha256((output / name).read_bytes()).hexdigest(), digest
                    )
                self.assertIn("install/install.sh", hashes)
                self.assertIn("install/codexe-local.sh", hashes)
            else:
                self.assertFalse(manifest.exists())
            return metadata

    def test_source_built_native_integration_declares_capability(self):
        self.assertEqual(
            self.package(prebuilt=False, attested=False)["xenolith"],
            {"nativeConfig": 1},
        )

    def test_prebuilt_does_not_imply_native_capability(self):
        self.assertNotIn("xenolith", self.package(prebuilt=True, attested=False))

    def test_attested_prebuilt_adds_capability_and_complete_installer_manifest(self):
        self.assertEqual(
            self.package(prebuilt=True, attested=True)["xenolith"], {"nativeConfig": 1}
        )

    def test_attestation_requires_native_source_and_prebuilt_input(self):
        for prebuilt, native_source in ((False, True), (True, False)):
            with (
                self.subTest(prebuilt=prebuilt, native_source=native_source),
                self.assertRaises(RuntimeError),
            ):
                self.package(
                    prebuilt=prebuilt, attested=True, native_source=native_source
                )

    def test_checksum_builder_rejects_directory_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "directory").mkdir()
            (root / "alias").symlink_to("directory")
            with self.assertRaises(RuntimeError):
                write_native_checksums(root)


if __name__ == "__main__":
    unittest.main()
