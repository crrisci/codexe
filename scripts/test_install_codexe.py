"""The Python entrypoint only builds and delegates; shell owns installation."""

from pathlib import Path
import os
import tempfile
import unittest
from unittest.mock import patch

import install_codexe


class BuildDelegationTest(unittest.TestCase):
    def test_existing_package_delegates_without_build_or_prefix_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            prefix = root / "prefix with spaces"
            bindir = root / "commands"
            with patch.object(install_codexe.subprocess, "run") as run:
                self.assertEqual(
                    install_codexe.main(
                        [
                            "--package-dir",
                            str(package),
                            "--prefix",
                            str(prefix),
                            "--bin-dir",
                            str(bindir),
                        ]
                    ),
                    0,
                )
            self.assertEqual(run.call_count, 1)
            argv = run.call_args.args[0]
            self.assertEqual(argv[0], "sh")
            self.assertIn("--codexe", argv)
            self.assertIn(str(package), argv)
            self.assertIn(str(bindir), argv)
            self.assertFalse(prefix.exists())

    def test_source_build_uses_canonical_builder_then_same_shell(self):
        with tempfile.TemporaryDirectory() as temporary:
            calls = []

            def run(args, **kwargs):
                calls.append(args)
                if "--target" in args:
                    Path(args[args.index("--package-dir") + 1]).mkdir()

            with (
                patch.dict(os.environ, {"PREFIX": temporary}),
                patch.object(install_codexe.subprocess, "run", side_effect=run),
            ):
                self.assertEqual(install_codexe.main(["--cargo-profile", "release"]), 0)
            self.assertEqual(len(calls), 2)
            self.assertTrue(calls[0][1].endswith("build_codex_package.py"))
            self.assertIn("release", calls[0])
            self.assertEqual(calls[1][0], "sh")
            self.assertIn("--codexe", calls[1])
            self.assertIn(temporary, calls[1])

    def test_archive_is_delegated_to_shell_without_python_extraction(self):
        with tempfile.NamedTemporaryFile(suffix=".tar.gz") as package:
            with patch.object(install_codexe.subprocess, "run") as run:
                self.assertEqual(
                    install_codexe.main(["--package-archive", package.name]), 0
                )
            self.assertEqual(run.call_count, 1)
            self.assertIn("--package-archive", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
