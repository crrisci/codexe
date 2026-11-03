"""Stage project and pinned bundled-helper notices without network access."""

import hashlib
import json
import shutil
from pathlib import Path

from .targets import REPO_ROOT
from .v8 import resolved_v8_crate_version

NOTICES = Path(__file__).with_name("notices")


def stage_package_notices(package_dir: Path) -> None:
    pins = json.loads((NOTICES / "pins.json").read_text())
    actual = {
        "v8_crate_version": resolved_v8_crate_version(),
        "rg_manifest_sha256": hashlib.sha256(
            Path(__file__).with_name("rg").read_bytes()
        ).hexdigest(),
    }
    if pins != actual:
        raise RuntimeError(
            "Bundled helper versions changed; review scripts/codex_package/notices "
            "and update their license texts and pins before packaging."
        )
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(REPO_ROOT / name, package_dir / name)
    destination = package_dir / "third-party-notices"
    shutil.copytree(NOTICES, destination)
    for source, name in (
        ("codex-rs/vendor/bubblewrap/COPYING", "bubblewrap-COPYING"),
        ("third_party/wezterm/LICENSE", "wezterm-LICENSE"),
        ("third_party/voice/NOTICE.md", "voice-NOTICE.md"),
    ):
        shutil.copyfile(REPO_ROOT / source, destination / name)
