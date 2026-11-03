#!/usr/bin/env python3
"""Build CodeXe's canonical package and delegate to its offline shell installer."""

import argparse
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile

REPO_ROOT = Path(__file__).resolve().parents[1]


def host_target() -> str:
    if platform.system() != "Linux":
        raise RuntimeError(
            "CodeXe source installation currently supports Linux/WSL only"
        )
    architecture = {"x86_64": "x86_64", "aarch64": "aarch64"}.get(platform.machine())
    if architecture is None:
        raise RuntimeError(f"Unsupported host architecture: {platform.machine()}")
    return f"{architecture}-unknown-linux-gnu"


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--prefix",
        type=Path,
        default=Path(os.environ.get("PREFIX") or str(Path.home() / ".local")),
    )
    parser.add_argument(
        "--bin-dir", type=Path, help="Command directory; defaults to PREFIX/bin"
    )
    package = parser.add_mutually_exclusive_group()
    package.add_argument(
        "--package-dir",
        type=Path,
        help="Delegate an already assembled package without building",
    )
    package.add_argument(
        "--package-archive",
        type=Path,
        help="Delegate a canonical .tar.gz archive without building",
    )
    parser.add_argument(
        "--cargo-profile",
        default="dev-small",
        help="Source build profile (default: dev-small)",
    )
    return parser.parse_args(argv)


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        target = host_target()
        env = dict(os.environ, CODEX_REPO_ROOT=str(REPO_ROOT))
        env.setdefault("CARGO_BUILD_JOBS", "2")
        with tempfile.TemporaryDirectory(prefix="codexe-build-") as temporary:
            source = args.package_dir or args.package_archive
            option = "--package-archive" if args.package_archive else "--package-dir"
            if source is None:
                source = Path(temporary) / "package"
                subprocess.run(
                    [
                        sys.executable,
                        str(REPO_ROOT / "scripts/build_codex_package.py"),
                        "--target",
                        target,
                        "--cargo-profile",
                        args.cargo_profile,
                        "--package-dir",
                        str(source),
                    ],
                    env=env,
                    check=True,
                )
            command = [
                "sh",
                str(REPO_ROOT / "scripts/install/install.sh"),
                "--codexe",
                option,
                str(source.expanduser().resolve(strict=True)),
                "--prefix",
                str(args.prefix.expanduser().resolve()),
            ]
            if args.bin_dir:
                command += ["--bin-dir", str(args.bin_dir.expanduser().resolve())]
            subprocess.run(command, env=env, check=True)
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"CodeXe build/install failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
