#!/usr/bin/env python3
"""Offline regression tests for the tracked 2.0 plan validator."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / ".github" / "scripts" / "check-v2-plan.py"
PLAN = pathlib.Path("docs/v2/roadmap/plan.json")
CI_WORKFLOW = pathlib.Path(".github/workflows/ci.yml")
EXACT_REVISION_EXPRESSION = "${{ github.event.pull_request.head.sha || github.sha }}"
PROVENANCE_STEP = (
    "      - name: verify checkout provenance\n"
    '        run: test "$(git rev-parse HEAD)" = "$EXPECTED_REVISION"\n'
)


class PlanContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="selene-v2-plan-")
        self.root = pathlib.Path(self.temporary.name)
        shutil.copytree(ROOT / "docs" / "v2", self.root / "docs" / "v2")
        shutil.copytree(ROOT / ".github", self.root / ".github")
        shutil.copy2(ROOT / "AGENTS.md", self.root / "AGENTS.md")
        shutil.copy2(ROOT / "README.md", self.root / "README.md")
        shutil.copy2(ROOT / ".gitignore", self.root / ".gitignore")
        (self.root / "scripts").mkdir()
        shutil.copy2(ROOT / "scripts" / "v2_baseline.py", self.root / "scripts" / "v2_baseline.py")
        shutil.copy2(ROOT / "scripts" / "v2-baseline.sh", self.root / "scripts" / "v2-baseline.sh")
        subprocess.run(
            ["git", "init", "--quiet"],
            cwd=self.root,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_validator(
        self,
        root: pathlib.Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        command = [sys.executable, "-B", str(CHECKER), "--root", str(root or self.root)]
        return subprocess.run(
            command,
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def read_plan(self) -> dict[str, Any]:
        return json.loads((self.root / PLAN).read_text(encoding="utf-8"))

    def write_plan(self, plan: dict[str, Any]) -> None:
        (self.root / PLAN).write_text(
            json.dumps(plan, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

    def mutate_plan(self, mutation: Callable[[dict[str, Any]], None]) -> subprocess.CompletedProcess[str]:
        plan = self.read_plan()
        mutation(plan)
        self.write_plan(plan)
        return self.run_validator()

    def assert_failure(self, result: subprocess.CompletedProcess[str], message: str) -> None:
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(message, result.stderr)

    def test_current_plan_and_isolated_copy_pass(self) -> None:
        canonical = self.run_validator(ROOT)
        self.assertEqual(canonical.returncode, 0, canonical.stderr)
        self.assertIn("v2 plan validation passed:", canonical.stdout)
        isolated = self.run_validator()
        self.assertEqual(isolated.returncode, 0, isolated.stderr)
        self.assertIn("v2 plan validation passed:", isolated.stdout)

    def test_duplicate_pr_id_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["pull_requests"][1].update(id="PLAN-01"))
        self.assert_failure(result, "duplicate work item IDs")

    def test_unknown_dependency_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: plan["pull_requests"][1]["depends_on"].append("F99-PR99")
        )
        self.assert_failure(result, "F01-PR01: unknown dependency F99-PR99")

    def test_pr_dependency_cycle_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: plan["pull_requests"][0]["depends_on"].append("F01-PR01")
        )
        self.assert_failure(result, "dependency cycle: F01-PR01 -> PLAN-01 -> F01-PR01")

    def test_missing_target_file_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: plan["pull_requests"][0].update(file="missing-plan.md")
        )
        self.assert_failure(result, "PLAN-01: missing file target")

    def test_mismatched_or_missing_issue_owner_fails(self) -> None:
        with self.subTest("unknown closure owner in plan"):
            result = self.mutate_plan(
                lambda plan: plan["issues"][0].update(closure_owner="F99-PR99")
            )
            self.assert_failure(result, "issue #1088: unknown closure_owner F99-PR99")

        self.tearDown()
        self.setUp()

        with self.subTest("mismatched issue owner in issue-ownership.md"):
            issue_file = self.root / "docs" / "v2" / "issue-ownership.md"
            text = issue_file.read_text(encoding="utf-8")
            text = text.replace("F02-PR01", "F05-PR05", 1)
            issue_file.write_text(text, encoding="utf-8")
            self.assert_failure(self.run_validator(), "'issue-1088' section is missing 'F02-PR01'")

    def test_merged_item_with_unmerged_dependency_fails(self) -> None:
        def break_dependency(plan: dict[str, Any]) -> None:
            plan["pull_requests"][0]["state"] = "proposed"
            plan["pull_requests"][1]["state"] = "merged"

        result = self.mutate_plan(break_dependency)
        self.assert_failure(
            result,
            "F01-PR01: merged work item has unmerged dependencies: ['PLAN-01']",
        )

    def test_legacy_work_item_mapping_fails(self) -> None:
        with self.subTest("unknown new_owner"):
            result = self.mutate_plan(
                lambda plan: plan["legacy_work_items"][21].update(new_owners=["F99-PR99"])
            )
            self.assert_failure(result, "new_owner 'F99-PR99' not found in pull_requests")

        self.tearDown()
        self.setUp()

        with self.subTest("unmerged completed legacy item"):
            result = self.mutate_plan(
                lambda plan: plan["legacy_work_items"][0].update(state="unmerged", new_owners=["PLAN-01"])
            )
            self.assert_failure(result, "M00-PR01: completed legacy item must have state 'merged'")

    def test_target_in_forbidden_underscore_directory_fails(self) -> None:
        with self.subTest("plan target in underscore directory"):
            result = self.mutate_plan(
                lambda plan: plan["pull_requests"][0].update(file="_scratch/PLAN-01.md")
            )
            self.assert_failure(result, "PLAN-01: unsafe file target in underscore directory")

        self.tearDown()
        self.setUp()

        with self.subTest("markdown link targets underscore directory"):
            readme = self.root / "docs" / "v2" / "README.md"
            readme.write_text(
                readme.read_text(encoding="utf-8") + "\n[local](../../_scratch/task.md)\n",
                encoding="utf-8",
            )
            self.assert_failure(self.run_validator(), "link targets an underscore working directory")

    def test_agents_policy_drift_fails(self) -> None:
        agents = self.root / "AGENTS.md"
        text = agents.read_text(encoding="utf-8").replace(
            "orchestrator owns commits, pushes, non-draft PR creation and updates",
            "The coordinator handles repository follow-up",
        )
        agents.write_text(text, encoding="utf-8")
        self.assert_failure(self.run_validator(), "missing corrected orchestrator Git/GitHub ownership")

    def test_superseded_repository_role_policy_fails(self) -> None:
        agents = self.root / "AGENTS.md"
        agents.write_text(
            agents.read_text(encoding="utf-8") + "\nAgents never merge 2.0 work.\n",
            encoding="utf-8",
        )
        self.assert_failure(self.run_validator(), "superseded role policy remains")

    def test_workflow_checkout_without_exact_ref_fails(self) -> None:
        workflow = self.root / CI_WORKFLOW
        text = workflow.read_text(encoding="utf-8")
        text = text.replace(
            f"          ref: {EXACT_REVISION_EXPRESSION}",
            "          ref: ${{ github.sha }}",
            1,
        )
        workflow.write_text(text, encoding="utf-8")
        self.assert_failure(self.run_validator(), "checkout must select the exact event revision")

    def test_workflow_required_provenance_assertion_fails(self) -> None:
        workflow = self.root / CI_WORKFLOW
        text = workflow.read_text(encoding="utf-8")
        self.assertIn(PROVENANCE_STEP, text)
        workflow.write_text(text.replace(PROVENANCE_STEP, "", 1), encoding="utf-8")
        self.assert_failure(
            self.run_validator(),
            "job 'rust-compile-and-test' must assert checkout provenance",
        )

    def test_corrupt_baseline_report_hash_fails(self) -> None:
        manifest_path = self.root / "docs" / "v2" / "baseline" / "manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["reports"][0]["sha256"] = "0" * 64
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        self.assert_failure(self.run_validator(), "executable baseline: report hash mismatch")

    def test_corrupt_baseline_schema_fails(self) -> None:
        schema_path = self.root / "docs" / "v2" / "baseline" / "manifest.schema.json"
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        del schema["additionalProperties"]
        schema_path.write_text(json.dumps(schema, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        self.assert_failure(self.run_validator(), "manifest schema differs from the helper's closed schema")


if __name__ == "__main__":
    unittest.main(verbosity=2)
