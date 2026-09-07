#!/usr/bin/env python3

import base64
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import review_transition as selector
import review_transition_materialize as materializer
import review_transition_oracle as oracle_runner


class ReviewTransitionOracleReplayTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="review-transition-oracle-test-")
        self.root = Path(self.temporary.name)
        source = self.root / "source"
        subprocess.run(["git", "init", "--quiet", source], check=True)
        subprocess.run(["git", "-C", source, "config", "user.name", "test"], check=True)
        subprocess.run(["git", "-C", source, "config", "user.email", "test@example.com"], check=True)
        (source / "file.txt").write_text("base\n", encoding="utf-8")
        subprocess.run(["git", "-C", source, "add", "file.txt"], check=True)
        subprocess.run(["git", "-C", source, "commit", "--quiet", "-m", "base"], check=True)
        q = self.git_stdout(source, "rev-parse", "HEAD")
        (source / "file.txt").write_text("reviewed\n", encoding="utf-8")
        subprocess.run(["git", "-C", source, "commit", "--all", "--quiet", "-m", "reviewed"], check=True)
        b = self.git_stdout(source, "rev-parse", "HEAD")
        subprocess.run(["git", "-C", source, "branch", "checkpoint", b], check=True)
        subprocess.run(["git", "-C", source, "checkout", "--quiet", "--detach", q], check=True)
        (source / "file.txt").write_text("current\n", encoding="utf-8")
        subprocess.run(["git", "-C", source, "commit", "--all", "--quiet", "-m", "current"], check=True)
        d = self.git_stdout(source, "rev-parse", "HEAD")
        subprocess.run(["git", "-C", source, "branch", "current", d], check=True)

        repositories = self.root / "materialization" / "repositories"
        repositories.mkdir(parents=True)
        self.repository = repositories / "fixture-one.git"
        subprocess.run(["git", "clone", "--bare", "--quiet", source, self.repository], check=True)
        subprocess.run(["git", "-C", self.repository, "remote", "remove", "origin"], check=True)
        snapshots = {"Q": q, "A": q, "B": b, "C": q, "D": d}
        snapshot_values = {
            label: {
                "commit_oid": commit,
                "tree_oid": self.git_stdout(self.repository, "rev-parse", f"{commit}^{{tree}}"),
            }
            for label, commit in snapshots.items()
        }
        changes = []
        checkpoint = materializer.REFERENCE_MODULE.raw_diff(self.repository, q, b)[2]
        for left, right in ((q, b), (q, d), (q, q), (b, d)):
            changes.extend(materializer.REFERENCE_MODULE.raw_diff(self.repository, left, right)[2])
        blobs = materializer.REFERENCE_MODULE.change_blob_ids(changes)
        blobs.update(
            materializer.REFERENCE_MODULE.review_delta_snapshot_blob_ids(
                self.repository,
                snapshots,
                checkpoint,
            )
        )
        self.case = {
            "case_id": "fixture-one",
            "repository": "local/fixture-one",
            "status": "materialized",
            "path": "repositories/fixture-one.git",
            "snapshots": snapshot_values,
            "required_blob_oids": sorted(blobs),
            "repository_bytes": materializer.repository_size(self.repository),
        }
        self.second_case = {
            "case_id": "fixture-two",
            "repository": "local/fixture-two",
            "status": "observed",
        }
        cases = [self.case, self.second_case]
        self.manifest = {
            "schema": materializer.MATERIALIZATION_SCHEMA,
            "observation_sha256": "0" * 64,
            "complete": False,
            "summary": materializer.materialization_summary(cases),
            "cases": cases,
        }
        self.materialization_raw = selector.canonical_json_bytes(self.manifest)

    def tearDown(self):
        self.temporary.cleanup()

    @staticmethod
    def git_stdout(repository, *arguments):
        return subprocess.check_output(["git", "-C", repository, *arguments], text=True).strip()

    def observation(self):
        commits = self.case["snapshots"]
        rows = []
        for case_id, repository in (("fixture-one", "local/fixture-one"), ("fixture-two", "local/fixture-two")):
            rows.append(
                {
                    "case_id": case_id,
                    "repository": repository,
                    "observation_status": "observed",
                    "pull_request": {"base_oid": commits["Q"]["commit_oid"]},
                    "checkpoint_oid": commits["B"]["commit_oid"],
                    "final_head_oid": commits["D"]["commit_oid"],
                }
            )
        return {"cases": rows}

    def copy_case(self, row, destination):
        shutil.copytree(self.repository, destination, copy_function=shutil.copy2)
        value = copy.deepcopy(self.case)
        value["case_id"] = row["case_id"]
        value["repository"] = row["repository"]
        value["path"] = destination.name
        value["repository_bytes"] = materializer.repository_size(destination)
        del value["status"]
        return value

    def make_fake_binary(self, oracle):
        identity = oracle["current_identities"][0]
        status_names = {"A": "added", "D": "deleted", "M": "modified", "R": "renamed", "T": "type_changed"}
        file = {
            "status": status_names[identity["status"]],
            "checkpoint_state": oracle["classification"][0]["checkpoint_state"],
        }
        for side in ("before", "after"):
            encoded = identity[f"{side}_path_base64"]
            if encoded is not None:
                file[f"{side}_path"] = base64.b64decode(encoded).decode("utf-8")
                file[f"{side}_path_encoding"] = "utf8"
            mode = identity[f"{side}_mode"]
            if mode is not None:
                file[f"{side}_mode"] = mode
            object_id = identity[f"{side}_object_id"]
            if object_id is not None:
                file[f"{side}_blob"] = object_id
        classification = oracle["classification"][0]
        if "checkpoint_match_basis" in classification:
            file["checkpoint_match_basis"] = classification["checkpoint_match_basis"]
        summary = oracle["summary"]
        report = {
            "requested_base": oracle["snapshots"]["Q"],
            "base_commit": oracle["snapshots"]["C"],
            "requested_head": oracle["snapshots"]["D"],
            "head_commit": oracle["snapshots"]["D"],
            "checkpoint": {
                "commit": oracle["snapshots"]["B"],
                "base_commit": oracle["snapshots"]["A"],
                "match_basis": "exact_git_change_identity",
            },
            "summary": {
                "changed_files": summary["current_change_identities"],
                "checkpoint": {
                    "unchanged_since_checkpoint_files": summary["exact_identity_carries"]
                    + summary["four_way_replay_carries"],
                    "needs_review_now_files": summary["needs_review_identities"],
                    "retired_change_count": summary["retired_checkpoint_changes"],
                },
            },
            "files": [file],
        }
        delta = {
            "old_base_commit": oracle["snapshots"]["A"],
            "checkpoint_commit": oracle["snapshots"]["B"],
            "current_base_commit": oracle["snapshots"]["C"],
            "head_commit": oracle["snapshots"]["D"],
            "summary": {"displayable_files": 1, "unresolved_retired_changes": 0, "needs_review_files": 1, "gate_passed": False},
        }
        build_info = {
            "schema": oracle_runner.BUILD_INFO_SCHEMA,
            "engine_version": "fixture",
            "git_revision": "1" * 40,
            "git_dirty": False,
            "build_profile": "release",
        }
        binary = self.root / "fake-stratadiff.py"
        script = f"""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys

REPORT = json.loads({json.dumps(json.dumps(report, sort_keys=True))})
DELTA = json.loads({json.dumps(json.dumps(delta, sort_keys=True))})
BUILD_INFO = json.loads({json.dumps(json.dumps(build_info, sort_keys=True))})

if sys.argv[1] == "build-info":
    print(json.dumps(BUILD_INFO, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)
if sys.argv[1] != "review":
    raise SystemExit(9)
repo = Path(sys.argv[sys.argv.index("--repo") + 1])
if subprocess.check_output(["git", "-C", repo, "remote"]):
    raise SystemExit(10)
if (repo / "objects" / "info" / "alternates").exists():
    raise SystemExit(11)
with open(os.environ["RT30_FAKE_REPLAY_LOG"], "a", encoding="utf-8") as stream:
    stream.write(str(repo.resolve()) + "\\n")
report_path = Path(sys.argv[sys.argv.index("--output") + 1])
delta_path = Path(sys.argv[sys.argv.index("--review-delta-output") + 1])
report_path.write_text(json.dumps(REPORT, sort_keys=True, separators=(",", ":")), encoding="utf-8")
delta_path.write_text(json.dumps(DELTA, sort_keys=True, separators=(",", ":")), encoding="utf-8")
sys.stderr.write(f"wrote repository review to {{report_path}}\\n")
sys.stderr.write(f"wrote review delta to {{delta_path}}\\n")
"""
        binary.write_text(script, encoding="utf-8")
        binary.chmod(0o755)
        return binary

    def test_materialization_is_resumable_and_preserves_completed_case(self):
        observation = self.observation()
        output = self.root / "resumable"
        calls = []

        def first_pass(row, destination, token, timeout, max_bytes):
            calls.append(row["case_id"])
            if row["case_id"] == "fixture-two":
                raise RuntimeError("injected acquisition failure")
            return self.copy_case(row, destination)

        with mock.patch.object(materializer, "materialize_case", side_effect=first_pass):
            first = materializer.materialize(observation, output, "token", 30, 100_000_000)
        self.assertFalse(first["complete"])
        self.assertEqual(first["summary"], {"selected_cases": 2, "observed": 0, "materialized": 1, "failed": 1})
        self.assertTrue((output / self.case["path"]).is_dir())
        first_size = materializer.repository_size(output / self.case["path"])
        with self.assertRaisesRegex(ValueError, "materialization is incomplete"):
            materializer.verify(first, output, observation, require_complete=True)
        materializer.verify(first, output, observation, require_complete=False)

        legacy = copy.deepcopy(first)
        del legacy["cases"][1]["attempts"]
        materializer.write_manifest_atomic(output, legacy)
        with self.assertRaisesRegex(ValueError, "explicit prior attempt timeout"):
            materializer.materialize(observation, output, "token", 60, 100_000_000)

        calls.clear()
        with mock.patch.object(
            materializer,
            "materialize_case",
            side_effect=lambda row, destination, token, timeout, max_bytes: self.copy_case(row, destination),
        ) as resumed:
            second = materializer.materialize(
                observation,
                output,
                "token",
                60,
                100_000_000,
                legacy_attempt_timeout=30,
            )
        self.assertTrue(second["complete"])
        self.assertEqual(resumed.call_count, 1)
        self.assertEqual(resumed.call_args.args[0]["case_id"], "fixture-two")
        self.assertEqual(materializer.repository_size(output / self.case["path"]), first_size)
        self.assertEqual(
            [(attempt["attempt"], attempt["fetch_timeout_seconds"], attempt["status"]) for attempt in second["cases"][1]["attempts"]],
            [(1, 30, "failed"), (2, 60, "materialized")],
        )
        materializer.verify(second, output, observation, require_complete=True)
        tampered = copy.deepcopy(second)
        tampered["cases"][0]["snapshots"]["Q"]["commit_oid"] = self.case["snapshots"]["B"]["commit_oid"]
        with self.assertRaisesRegex(ValueError, "Q commit differs from observation"):
            materializer.verify(tampered, output, observation, require_complete=True)

    def test_batch_object_type_check_is_complete_and_fail_closed(self):
        blob = self.case["required_blob_oids"][0]
        commit = self.case["snapshots"]["Q"]["commit_oid"]
        self.assertEqual(
            materializer.object_types(self.repository, [blob, commit]),
            {blob: "blob", commit: "commit"},
        )
        with self.assertRaisesRegex(ValueError, "required object unavailable"):
            materializer.object_types(self.repository, ["f" * 40])

    def test_offline_repository_and_manifest_tampering_are_rejected(self):
        remote_copy = self.root / "remote-copy.git"
        shutil.copytree(self.repository, remote_copy, copy_function=shutil.copy2)
        subprocess.run(["git", "-C", remote_copy, "remote", "add", "origin", "https://example.invalid/repo.git"], check=True)
        with self.assertRaisesRegex(ValueError, "retained a remote"):
            materializer.require_offline_repository(remote_copy)

        shallow_copy = self.root / "shallow-copy.git"
        shutil.copytree(self.repository, shallow_copy, copy_function=shutil.copy2)
        (shallow_copy / "shallow").write_text(
            self.case["snapshots"]["Q"]["commit_oid"] + "\n",
            encoding="ascii",
        )
        with self.assertRaisesRegex(ValueError, "is shallow"):
            materializer.require_offline_repository(shallow_copy)

        observation = self.observation()
        manifest = copy.deepcopy(self.manifest)
        manifest["observation_sha256"] = selector.sha256_bytes(selector.canonical_json_bytes(observation))
        manifest["cases"][0]["path"] = "../escape.git"
        with self.assertRaisesRegex(ValueError, "path differs"):
            materializer.verify(
                manifest,
                self.root / "materialization",
                observation,
                require_complete=False,
            )

    def test_oracle_and_double_replay_are_independent_canonical_and_tamper_evident(self):
        bundle = oracle_runner.generate_oracle_bundle(
            self.manifest,
            self.root / "materialization",
            self.materialization_raw,
        )
        self.assertFalse(bundle["complete"])
        self.assertEqual(bundle["summary"], {"selected_cases": 2, "evaluated": 1, "not_evaluated": 1})
        self.assertEqual(bundle["cases"][1]["reason"], "materialization_observed")
        oracle_runner.verify_oracle_bundle(
            bundle,
            self.manifest,
            self.root / "materialization",
            self.materialization_raw,
        )
        unrelated_product_output = self.root / "materialization" / "product-output.json"
        unrelated_product_output.write_text('{"forged":true}', encoding="utf-8")
        regenerated = oracle_runner.generate_oracle_bundle(
            self.manifest,
            self.root / "materialization",
            self.materialization_raw,
        )
        self.assertEqual(regenerated, bundle)

        tampered_oracle = copy.deepcopy(bundle)
        tampered_oracle["cases"][0]["oracle"]["summary"]["needs_review_identities"] += 1
        with self.assertRaisesRegex(ValueError, "independent Git recomputation"):
            oracle_runner.verify_oracle_bundle(
                tampered_oracle,
                self.manifest,
                self.root / "materialization",
                self.materialization_raw,
            )

        oracle_raw = selector.canonical_json_bytes(bundle)
        binary = self.make_fake_binary(bundle["cases"][0]["oracle"])
        replay_root = self.root / "replay"
        log = self.root / "replay-paths.log"
        with mock.patch.dict(os.environ, {"RT30_FAKE_REPLAY_LOG": str(log)}):
            replay = oracle_runner.replay_product(
                bundle,
                self.manifest,
                self.root / "materialization",
                self.materialization_raw,
                oracle_raw,
                binary,
                replay_root,
            )
            oracle_runner.verify_replay_bundle(
                replay,
                replay_root,
                bundle,
                self.manifest,
                self.materialization_raw,
                oracle_raw,
                binary,
            )
        self.assertFalse(replay["complete"])
        self.assertEqual(replay["summary"]["evaluated"], 1)
        self.assertEqual(replay["summary"]["deterministic"], 1)
        self.assertEqual(replay["summary"]["not_evaluated"], 1)
        self.assertTrue(replay["cases"][0]["conformance"]["passed"])
        replay_paths = log.read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(replay_paths), 2)
        self.assertEqual(len(set(replay_paths)), 2)
        self.assertNotIn(str(self.repository.resolve()), replay_paths)

        report_path = replay_root / replay["cases"][0]["runs"][1]["report"]["path"]
        report = json.loads(report_path.read_text(encoding="utf-8"))
        report["summary"]["changed_files"] += 1
        report_path.write_bytes(selector.canonical_json_bytes(report))
        with mock.patch.dict(os.environ, {"RT30_FAKE_REPLAY_LOG": str(log)}):
            with self.assertRaisesRegex(ValueError, "digest differs"):
                oracle_runner.verify_replay_bundle(
                    replay,
                    replay_root,
                    bundle,
                    self.manifest,
                    self.materialization_raw,
                    oracle_raw,
                    binary,
                )


if __name__ == "__main__":
    unittest.main()
