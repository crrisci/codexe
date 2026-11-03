"""Ensure package notices follow the pinned helper versions."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from codex_package.notices import stage_package_notices


class NoticesTest(unittest.TestCase):
    def test_package_includes_project_and_bundled_helper_licenses(self):
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary)
            stage_package_notices(package)
            for name in ("LICENSE", "NOTICE"):
                self.assertTrue((package / name).read_text().strip())
            self.assertIn("Francesco Pio Crisci", (package / "NOTICE").read_text())
            self.assertIn("Simone Iacomino", (package / "NOTICE").read_text())
            for name in (
                "bubblewrap-COPYING",
                "ripgrep-LICENSE-MIT",
                "rusty-v8-LICENSE",
                "v8-LICENSE",
            ):
                self.assertTrue(
                    (package / "third-party-notices" / name).read_text().strip()
                )

    def test_changed_v8_pin_requires_review_before_packaging(self):
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary)
            with patch(
                "codex_package.notices.resolved_v8_crate_version",
                return_value="999.0.0",
            ):
                with self.assertRaisesRegex(RuntimeError, "versions changed"):
                    stage_package_notices(package)
            self.assertEqual(list(package.iterdir()), [])
