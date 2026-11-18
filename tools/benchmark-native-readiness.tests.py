"""Regression checks for the readiness driver's staging and Docker handoffs."""

import hashlib
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "benchmark_native_readiness", Path(__file__).with_name("benchmark-native-readiness.py"),
)
readiness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(readiness)


class NativeReadinessTests(unittest.TestCase):
    def test_rejects_a_different_task_even_when_staging_digests_match(self):
        with tempfile.TemporaryDirectory() as directory:
            lab = Path(directory)
            run_id = "fixture"
            root = lab / "pilot" / run_id
            (root / "task").mkdir(parents=True)
            image = "sha256:" + "a" * 64
            task = f'[environment]\ndocker_image = "{image}"\n'.encode()
            config = json.dumps({
                "job_name": run_id, "n_attempts": 1, "environment": {"type": "docker"},
                "agents": [{"name": "acp"}], "tasks": [{"path": str(lab / "other-task")}],
            }).encode()
            checkpoint = {
                "run_id": run_id, "task_id": "fixture-task", "profile": "jecode-openai",
                "scope": "official_task_round", "phase": "staged", "agent_uid": 0,
                "agent_image": image, "config_sha256": hashlib.sha256(config).hexdigest(),
                "modified_task_toml_sha256": hashlib.sha256(task).hexdigest(),
            }
            (root / "task/task.toml").write_bytes(task)
            (root / "config.json").write_bytes(config)
            (root / "checkpoint.json").write_text(json.dumps(checkpoint), encoding="utf-8")
            with patch.object(readiness, "LAB", lab):
                with self.assertRaisesRegex(readiness.ReadinessError, "reference the checked task"):
                    readiness.staged_runtime(run_id, "run", {})

    def test_lost_create_response_retires_only_the_verified_container(self):
        container_id = "b" * 64
        image = "sha256:" + "c" * 64
        name = "jecode-readiness-fixture"
        for owner in (readiness.PROBE_OWNER, "different-owner"):
            with self.subTest(owner=owner):
                report = {"receipt": "/tmp/fixture/receipt.json", "image": image}
                identity = f"{container_id} /{name} {image} {owner}".encode()
                inspection = subprocess.CompletedProcess([], 0, stdout=identity)
                removal = subprocess.CompletedProcess([], 0)
                timeout = readiness.ReadinessError("Process could not complete (TimeoutExpired).")
                with patch.object(readiness, "invoke", side_effect=[timeout, inspection, removal]) as docker:
                    with self.assertRaisesRegex(readiness.ReadinessError, "TimeoutExpired"):
                        readiness.check_container(report, {}, {"cpus": 2, "memory_mb": 4096})

                commands = [call.args[0][len(readiness.DOCKER):] for call in docker.call_args_list]
                self.assertEqual(commands[0][0], "create")
                self.assertEqual(commands[1][:3], ["container", "inspect", "--format"])
                self.assertEqual(commands[1][-1], name)
                if owner == readiness.PROBE_OWNER:
                    self.assertEqual(commands[2:], [["rm", "-fv", container_id]])
                    self.assertEqual(report["container_id"], container_id)
                    self.assertTrue(report["container_removed"])
                else:
                    self.assertEqual(commands[2:], [])
                    self.assertFalse(report["container_removed"])
                    self.assertIn("ownership", report["container_cleanup_error"])


if __name__ == "__main__":
    unittest.main()
