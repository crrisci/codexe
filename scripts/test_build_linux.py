"""Exercise container orchestration with fixtures; no Docker or Rust build."""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('build_linux.py')
spec = importlib.util.spec_from_file_location('build_linux', SCRIPT)
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class ContainerRecipeTests(unittest.TestCase):
    def test_snapshot_mount_provenance_and_canonical_builder(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'checkout'
            source.mkdir()
            (source / 'tracked.txt').write_text('source fixture\n')
            for name in ('.git', 'target', 'node_modules', 'dist'):
                (source / name).mkdir()
                (source / name / 'excluded').write_text('not source')
            out = root / 'output'
            def output(argv, **kwargs):
                if argv[0] == 'git':
                    return 'a' * 40 + '\n'
                return json.dumps([{'Id': 'sha256:fixture', 'RepoDigests': []}])
            def fake_run(argv, **kwargs):
                (out / 'codexe-linux-x86_64-unknown-linux-gnu.tar.gz').write_bytes(b'fixture package')
            with patch('sys.argv', [str(SCRIPT), '--source', str(source), '--output', str(out), '--cargo-profile', 'release']), patch.object(builder.subprocess, 'check_output', side_effect=output), patch.object(builder, 'run', side_effect=fake_run) as run:
                builder.main()
            self.assertEqual((out / 'source/tracked.txt').read_text(), 'source fixture\n')
            for name in ('.git', 'target', 'node_modules', 'dist'):
                self.assertFalse((out / 'source' / name).exists())
            script = (out / 'build.sh').read_text()
            self.assertIn('scripts/build_codex_package.py', script)
            self.assertIn('--cargo-profile release --strip', script)
            argv = [str(arg) for arg in run.call_args.args[0]]
            self.assertIn('--cap-drop=ALL', argv)
            self.assertIn(f'type=bind,source={out},target=/work', argv)
            self.assertEqual(sum(arg == '--mount' for arg in argv), 1)
            self.assertNotIn('/var/run/docker.sock', ' '.join(argv))
            # A dedicated build directory may itself be under the host home.
            # Only that directory may be mounted, with a separate container home.
            self.assertNotIn('-v', argv)
            self.assertNotIn('--volume', argv)
            container_env = [argv[i + 1] for i, arg in enumerate(argv) if arg == '-e']
            self.assertIn('HOME=/work/home', container_env)
            provenance = json.loads((out / 'build-provenance.json').read_text())
            self.assertEqual(provenance['base_commit'], 'a' * 40)
            self.assertEqual(provenance['image_id'], 'sha256:fixture')
            self.assertFalse(provenance['model_loaded'])
            self.assertEqual(set(json.loads((out / 'source-manifest.json').read_text())), {'tracked.txt'})

    def test_version_derivation_uses_only_disposable_checkout(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            scripts = root / 'scripts'
            scripts.mkdir()
            shutil.copy2(SCRIPT.with_name('prepare_codexe_release.py'), scripts)
            shutil.copytree(SCRIPT.parent / 'codex_package', scripts / 'codex_package')
            cargo = root / 'codex-rs'
            cargo.mkdir()
            manifest = cargo / 'Cargo.toml'
            manifest.write_text('[workspace.package]\nversion = "0.0.0" # upstream development\n')
            result = subprocess.run([sys.executable, str(scripts / 'prepare_codexe_release.py'), '1.2.3-rc.1'], env={**os.environ, 'STABLE_GIT_COMMIT': 'b' * 40}, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(tomllib.loads(manifest.read_text())['workspace']['package']['version'], '1.2.3-rc.1')
            self.assertIn('# upstream development', manifest.read_text())
            provenance = json.loads((root / 'dist/codexe-release-provenance.json').read_text())
            self.assertEqual(provenance['sourceWorkspaceVersion'], '0.0.0')
            self.assertEqual(provenance['baseGitCommit'], 'b' * 40)

    def test_nonempty_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'checkout'
            source.mkdir()
            out = root / 'out'
            out.mkdir()
            (out / 'evidence').write_text('preserve')
            with patch('sys.argv', [str(SCRIPT), '--source', str(source), '--output', str(out)]), self.assertRaises(SystemExit):
                builder.main()
            self.assertEqual((out / 'evidence').read_text(), 'preserve')


if __name__ == '__main__':
    unittest.main()
