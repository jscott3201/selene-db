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
    '      - name: verify checkout provenance\n'
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

    def run_validator(self, root: pathlib.Path | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(CHECKER), "--root", str(root or self.root)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def read_plan(self) -> dict[str, Any]:
        return json.loads((self.root / PLAN).read_text(encoding="utf-8"))

    def write_plan(self, plan: dict[str, Any]) -> None:
        (self.root / PLAN).write_text(json.dumps(plan, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

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
        isolated = self.run_validator()
        self.assertEqual(isolated.returncode, 0, isolated.stderr)

    def test_unknown_dependency_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["pull_requests"][2]["dependencies"].append("M10-PR99"))
        self.assert_failure(result, "M00-PR03: unknown dependency M10-PR99")

    def test_duplicate_pr_id_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["pull_requests"][1].update(id="M00-PR01"))
        self.assert_failure(result, "duplicate work item IDs")

    def test_pr_dependency_cycle_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["pull_requests"][0]["dependencies"].append("M00-PR02"))
        self.assert_failure(result, "dependency cycle: M00-PR01 -> M00-PR02 -> M00-PR01")

    def test_milestone_dependency_cycle_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["milestones"][0]["dependencies"].append("M01"))
        self.assert_failure(result, "dependency cycle: M00 -> M01 -> M00")

    def test_cross_domain_dependency_cycle_fails(self) -> None:
        result = self.mutate_plan(lambda plan: plan["milestones"][0]["dependencies"].append("M00-PR03"))
        self.assert_failure(result, "dependency cycle: M00 -> M00-PR03 -> M00")

    def test_merged_item_with_unmerged_dependency_fails(self) -> None:
        def break_dependency(plan: dict[str, Any]) -> None:
            plan["pull_requests"][2]["status"] = "Unmerged"
            plan["pull_requests"][3]["status"] = "Merged"

        result = self.mutate_plan(break_dependency)
        self.assert_failure(result, "M00-PR04: merged work item has unmerged dependencies: ['M00-PR03']")

    def test_stale_completed_pr_status_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: next(item for item in plan["pull_requests"] if item["id"] == "M00-PR04").update(
                status="Unmerged"
            )
        )
        self.assert_failure(result, "M00-PR04: prerequisite status must be Merged")

    def test_missing_markdown_target_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: plan["milestones"][0].update(file="docs/v2/roadmap/missing.md#m00")
        )
        self.assert_failure(result, "M00: missing file target")

    def test_stale_generated_projection_fails(self) -> None:
        projection = self.root / "docs" / "v2" / "roadmap" / "work-items-00-04.md"
        projection.write_text(projection.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        self.assert_failure(self.run_validator(), "projection is stale")

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

    def test_duplicate_and_mismatched_issue_ownership_fail(self) -> None:
        with self.subTest("duplicate reference"):
            result = self.mutate_plan(lambda plan: plan["pull_requests"][2]["issues"].append(1088))
            self.assert_failure(result, "issue #1088: owner=M09-PR01 work-item references=['M00-PR03', 'M09-PR01']")
        self.tearDown()
        self.setUp()
        with self.subTest("mismatched owner"):
            result = self.mutate_plan(lambda plan: plan["issues"][0].update(owner="M00-PR03"))
            self.assert_failure(result, "issue #1088: owner=M00-PR03 work-item references=['M09-PR01']")

    def test_swapped_issue_labels_fail(self) -> None:
        issue_file = self.root / "docs" / "v2" / "issue-ownership.md"
        text = issue_file.read_text(encoding="utf-8")
        first = "[#1088]"
        second = "[#1092]"
        text = text.replace(first, "ISSUE_SECTION_PLACEHOLDER").replace(second, first).replace("ISSUE_SECTION_PLACEHOLDER", second)
        issue_file.write_text(text, encoding="utf-8")
        self.assert_failure(self.run_validator(), "'issue-1088' section is missing '#1088'")

    def test_swapped_issue_owners_fail(self) -> None:
        issue_file = self.root / "docs" / "v2" / "issue-ownership.md"
        text = issue_file.read_text(encoding="utf-8")
        first = "[M09-PR01]"
        second = "[M08-PR02]"
        text = text.replace(first, "ISSUE_OWNER_PLACEHOLDER").replace(second, first).replace("ISSUE_OWNER_PLACEHOLDER", second)
        issue_file.write_text(text, encoding="utf-8")
        self.assert_failure(self.run_validator(), "'issue-1088' section is missing 'M09-PR01'")

    def test_swapped_decision_bodies_fail(self) -> None:
        plan = self.read_plan()
        first = plan["decisions"][0]["decision"]
        second = plan["decisions"][1]["decision"]
        decision_file = self.root / "docs" / "v2" / "decisions" / "finalized.md"
        text = decision_file.read_text(encoding="utf-8")
        text = text.replace(first, "DECISION_BODY_PLACEHOLDER").replace(second, first).replace("DECISION_BODY_PLACEHOLDER", second)
        decision_file.write_text(text, encoding="utf-8")
        self.assert_failure(self.run_validator(), "'d-001' section does not contain its decision body")

    def test_local_only_plan_target_and_link_fail(self) -> None:
        with self.subTest("plan target"):
            result = self.mutate_plan(
                lambda plan: plan["milestones"][0].update(file="docs/v2/_scratch/local.md#m00")
            )
            self.assert_failure(result, "M00: unsafe file target")
        self.tearDown()
        self.setUp()
        with self.subTest("Markdown link"):
            readme = self.root / "docs" / "v2" / "README.md"
            readme.write_text(readme.read_text(encoding="utf-8") + "\n[local](../../_scratch/task.md)\n", encoding="utf-8")
            self.assert_failure(self.run_validator(), "link targets an underscore working directory")

    def test_local_only_execution_dependency_fails(self) -> None:
        result = self.mutate_plan(
            lambda plan: plan["pull_requests"][2]["tests"].append("Run _scratch/check.sh")
        )
        self.assert_failure(result, "local-only source name '_scratch' is forbidden")

    def test_stale_repository_role_policy_fails(self) -> None:
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
        self.assert_failure(self.run_validator(), "job 'rust-compile-and-test' must assert checkout provenance")


if __name__ == "__main__":
    unittest.main(verbosity=2)
