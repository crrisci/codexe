#!/usr/bin/env python3
"""Build the canonical CodeXe Linux package in an isolated Ubuntu 24.04 container."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def run(argv, **kwargs):
    subprocess.run(list(map(str, argv)), check=True, **kwargs)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1])
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--package-version', help='Derive workspace and CLI version in the isolated source snapshot')
    p.add_argument('--image', default='codexe-build:ubuntu24-local')
    p.add_argument('--build-image', action='store_true')
    p.add_argument('--cargo-profile', choices=('dev', 'release'), default='dev')
    args = p.parse_args()
    source = args.source.resolve(strict=True)
    out = args.output.resolve()
    if not source.is_dir() or out == source or out.is_relative_to(source):
        p.error('--source must be a directory and --output must be outside it')
    if args.package_version:
        import sys
        os.environ['CODEX_REPO_ROOT'] = str(source)
        sys.path.insert(0, str(source / 'scripts'))
        from codex_package.cli import parse_package_version
        parse_package_version(args.package_version)
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        p.error('--output must be empty; preserve previous build evidence separately')
    if args.build_image:
        packaging = Path(__file__).resolve().parents[1] / 'packaging'
        run(['docker', 'build', '-t', args.image, '-f', packaging / 'Dockerfile.ubuntu24', packaging])
    work = out / 'source'
    shutil.copytree(source, work, symlinks=True, ignore=shutil.ignore_patterns(
        '.git', 'target', 'node_modules', '.venv', '__pycache__',
        '.test-artifacts', '.ruff_cache', '.pytest_cache', 'dist'))
    # All derivation occurs in a disposable copy. The canonical builder retains
    # ownership of package checksums: never strip or mutate its output afterward.
    prepare = 'python3 scripts/prepare_codexe_release.py "$PACKAGE_VERSION"\ncargo update --manifest-path codex-rs/Cargo.toml --workspace\n' if args.package_version else ''
    version_option = ' --package-version "$PACKAGE_VERSION"' if args.package_version else ''
    strip_option = ' --strip' if args.cargo_profile == 'release' else ''
    archive_name = f'codexe-v{args.package_version}-linux-x86_64-unknown-linux-gnu.tar.gz' if args.package_version else 'codexe-linux-x86_64-unknown-linux-gnu.tar.gz'
    script = f'''set -eu
cd /work/source
export CODEX_REPO_ROOT=/work/source
{prepare}python3 scripts/build_codex_package.py --target x86_64-unknown-linux-gnu --cargo-profile {args.cargo_profile}{strip_option}{version_option} --package-dir /work/artifact --archive-output /work/{archive_name}
'''
    commit = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    (out / 'build.sh').write_text(script)
    manifest = {str(path.relative_to(work)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted(work.rglob('*')) if path.is_file() and not path.is_symlink()}
    manifest_path = out / 'source-manifest.json'
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    image = json.loads(subprocess.check_output(['docker', 'image', 'inspect', args.image], text=True))[0]
    provenance = {'schema_version': 1, 'component': 'codexe', 'base_commit': commit,
                  'image_id': image['Id'], 'repo_digests': image.get('RepoDigests', []),
                  'baseline': 'Ubuntu 24.04 userspace', 'model_loaded': False,
                  'cargo_profile': args.cargo_profile, 'package_version': args.package_version,
                  'package_stripped': args.cargo_profile == 'release',
                  'build_script_sha256': hashlib.sha256(script.encode()).hexdigest(),
                  'source_manifest_sha256': hashlib.sha256(manifest_path.read_bytes()).hexdigest()}
    (out / 'build-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    profile_env = ['-e', 'CARGO_PROFILE_DEV_DEBUG=0'] if args.cargo_profile == 'dev' else []
    run(['docker', 'run', '--rm', '--user', f'{os.getuid()}:{os.getgid()}',
         '--cap-drop=ALL', '--security-opt=no-new-privileges', '--cpus=2',
         '--mount', f'type=bind,source={out},target=/work',
         '-e', 'HOME=/work/home', '-e', 'CARGO_HOME=/work/cargo',
         '-e', 'CARGO_TARGET_DIR=/work/target', '-e', 'CARGO_BUILD_JOBS=2',
         *profile_env, '-e', 'CARGO_INCREMENTAL=0',
         '-e', 'RUSTUP_TOOLCHAIN=1.95.0-x86_64-unknown-linux-gnu',
         '-e', f'STABLE_GIT_COMMIT={commit}', '-e', f'PACKAGE_VERSION={args.package_version or ""}',
         args.image, 'sh', '/work/build.sh'])
    archive = out / archive_name
    with archive.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    (out / 'SHA256SUMS').write_text(f'{digest}  {archive_name}\n')
    provenance['archive_sha256'] = digest
    lock = work / 'codex-rs/Cargo.lock'
    if lock.is_file():
        provenance['cargo_lock_sha256'] = hashlib.sha256(lock.read_bytes()).hexdigest()
    (out / 'build-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(f'Built CodeXe: {out / archive_name}. Inspect ABI and runtime requirements before distribution.')


if __name__ == '__main__':
    main()
