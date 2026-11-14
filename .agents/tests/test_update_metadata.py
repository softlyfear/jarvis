"""Regression coverage for fork updates and installer integrity metadata."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import write_update_metadata


class UpdateMetadataTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.installer = Path(self.directory.name) / "JarvisSetup.exe"
        self.installer.write_bytes(b"installer regression fixture")
        self.environment = {
            "JARVIS_UPDATE_REPOSITORY": "example/voice-assistant",
            "JARVIS_VERSION": "0.2.107",
            "JARVIS_BUILD": "a" * 40,
        }

    def test_fork_metadata_matches_legacy_updater_and_actual_installer(self):
        for repository in ("example/voice-assistant", "another/assistant-fork"):
            with self.subTest(repository=repository):
                metadata = write_update_metadata.release_metadata(
                    self.installer, {**self.environment, "JARVIS_UPDATE_REPOSITORY": repository}
                )
                self.assertEqual(metadata["version"], "0.2.107")
                self.assertEqual(metadata["build"], "a" * 40)
                self.assertEqual(metadata["sha256"], hashlib.sha256(self.installer.read_bytes()).hexdigest())
                # Version 0.2.97 permits this exact repository, tag and filename shape.
                prefix = f"https://github.com/{repository}/releases/download/"
                self.assertTrue(metadata["setup"].startswith(prefix))
                self.assertEqual(metadata["setup"][len(prefix):], "latest/JarvisSetup.exe")

    def test_installer_changes_change_digest(self):
        first = write_update_metadata.release_metadata(self.installer, self.environment)
        self.installer.write_bytes(b"replacement installer")
        second = write_update_metadata.release_metadata(self.installer, self.environment)
        self.assertNotEqual(first["sha256"], second["sha256"])

    def test_missing_or_invalid_build_configuration_is_rejected(self):
        for repository in ("", "example", "example/", "/repo", "example/repo/extra", "../repo", "example/..", "example/repo?x", "example/repo\\extra", "example/repo\n"):
            with self.subTest(repository=repository), self.assertRaises(ValueError):
                write_update_metadata.release_metadata(
                    self.installer, {**self.environment, "JARVIS_UPDATE_REPOSITORY": repository}
                )
        for key in self.environment:
            environment = dict(self.environment)
            del environment[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                write_update_metadata.release_metadata(self.installer, environment)

    def test_missing_installer_cannot_produce_metadata(self):
        with self.assertRaises(OSError):
            write_update_metadata.release_metadata(self.installer.with_name("missing.exe"), self.environment)

    def test_ci_entrypoint_writes_metadata_and_fails_without_repository(self):
        output = self.installer.with_name("version.json")
        command = [sys.executable, write_update_metadata.__file__, str(self.installer), str(output)]
        environment = {**os.environ, **self.environment}
        result = subprocess.run(command, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(output.read_text()), write_update_metadata.release_metadata(self.installer, self.environment))
        output.unlink()
        del environment["JARVIS_UPDATE_REPOSITORY"]
        result = subprocess.run(command, env=environment, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
