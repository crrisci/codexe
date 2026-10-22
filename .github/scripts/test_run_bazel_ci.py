"""Exercise CI argument selection without invoking a compiler or remote service."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


class RunBazelCiTest(unittest.TestCase):
    def arguments(self, runner, *, key="", options=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "run-bazel-ci.sh"
            shutil.copyfile(Path(__file__).with_name(script.name), script)
            wrapper = root / "run_bazel_with_buildbuddy.py"
            wrapper.write_text(
                '#!/bin/bash\nprintf "%s\\n" "$@" > "$BAZEL_TEST_CAPTURE"\n'
            )
            wrapper.chmod(0o755)
            capture = root / "arguments"
            env = dict(os.environ)
            for name in list(env):
                if name.startswith(("BAZEL_", "CODEX_BAZEL_", "BUILDBUDDY_")):
                    del env[name]
            env.update(
                RUNNER_OS=runner,
                BUILDBUDDY_API_KEY=key,
                CODEX_BAZEL_WINDOWS_PATH="C:/Build Tools/bin",
                BAZEL_TEST_CAPTURE=str(capture),
            )
            subprocess.run(
                ["bash", str(script), *options, "--", "test", "--", "//test:fixture"],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            return capture.read_text().splitlines()

    def test_keyless_windows_uses_matching_native_c_and_rust_toolchains(self):
        args = self.arguments("Windows", options=("--windows-cross-compile",))
        self.assertIn("--host_platform=//:local_windows_msvc", args)
        self.assertIn("--repo_env=BAZEL_DO_NOT_DETECT_CPP_TOOLCHAIN=0", args)
        self.assertIn(
            "--extra_toolchains=@local_config_cc//:cc-toolchain-x64_windows", args
        )
        self.assertIn("--disk_cache=", args)
        self.assertEqual(args[-2:], ["--", "//test:fixture"])

    def test_keyed_cross_build_keeps_remote_toolchains(self):
        args = self.arguments(
            "Windows", key="test-key", options=("--windows-cross-compile",)
        )
        self.assertIn("--config=ci-windows-cross", args)
        self.assertIn("--host_platform=//:rbe", args)
        self.assertFalse(any(arg.startswith("--extra_toolchains=") for arg in args))
        self.assertNotIn("--disk_cache=", args)

    def test_linux_local_build_does_not_select_windows_toolchains(self):
        args = self.arguments("Linux")
        self.assertIn("--disk_cache=", args)
        self.assertFalse(any(arg.startswith("--extra_toolchains=") for arg in args))


if __name__ == "__main__":
    unittest.main()
