"""CodeXe capability declaration and offline install assets for native builds."""

import hashlib
import shutil
import subprocess
from pathlib import Path

from .targets import REPO_ROOT


def source_supports_native_config() -> bool:
    discovery = REPO_ROOT / "codex-rs/xenolith-wire/src/native_config.rs"
    runtime = REPO_ROOT / "codex-rs/core/src/xenolith/config.rs"
    return (
        discovery.is_file()
        and runtime.is_file()
        and ("ensure_service_resolved_cancellable" in runtime.read_text())
    )


def stage_native_installer(package_dir: Path) -> None:
    destination = package_dir / "install"
    destination.mkdir()
    for name in ("install.sh", "codexe-local.sh"):
        shutil.copyfile(REPO_ROOT / "scripts/install" / name, destination / name)


def write_native_checksums(package_dir: Path) -> None:
    manifest = package_dir / "codexe-package_SHA256SUMS"
    lines = []
    for path in sorted(package_dir.rglob("*")):
        if path == manifest or (path.is_dir() and not path.is_symlink()):
            continue
        relative = path.relative_to(package_dir).as_posix()
        if (
            path.is_symlink()
            or not path.is_file()
            or any(
                character
                not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_./+-"
                for character in relative
            )
        ):
            raise RuntimeError(f"Unsupported native package entry: {relative}")
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        lines.append(f"{digest}  {relative}\n")
    manifest.write_text("".join(lines))


def strip_staged_elf_files(package_dir: Path, strip_tool: str) -> None:
    """Strip copies inside the new package, before its integrity manifest exists."""
    for path in sorted(package_dir.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Refusing to strip package symlink: {path}")
        if not path.is_file():
            continue
        with path.open("rb") as stream:
            if stream.read(4) != b"\x7fELF":
                continue
        try:
            subprocess.run(
                [strip_tool, "--strip-unneeded", str(path)],
                check=True,
                capture_output=True,
                text=True,
            )
        except (OSError, subprocess.CalledProcessError) as error:
            detail = (
                error.stderr.strip()
                if isinstance(error, subprocess.CalledProcessError)
                else str(error)
            )
            raise RuntimeError(
                f"Cannot strip staged package file {path.relative_to(package_dir)}: {detail}"
            ) from error
