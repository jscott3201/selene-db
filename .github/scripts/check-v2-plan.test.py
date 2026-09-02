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
M04_PART_3A_PRODUCTION_PATHS = [
    "crates/selene-graph/src/candidate_set.rs",
    "crates/selene-graph/src/graph.rs",
    "crates/selene-graph/src/lib.rs",
    "crates/selene-graph/src/store.rs",
    "crates/selene-graph/src/text_index.rs",
    "crates/selene-graph/src/vector_search.rs",
    "crates/selene-graph/src/json_search.rs",
    "crates/selene-graph/src/json_search_candidates.rs",
    "crates/selene-graph/src/text_search.rs",
    "crates/selene-graph/src/vector_search/approx_turbo_quant.rs",
    "crates/selene-graph/src/vector_search/score.rs",
    "crates/selene-graph/src/vector_search/exact_batch.rs",
    "crates/selene-graph/src/vector_search/score_candidate_batch.rs",
]
M04_PART_3B_PRODUCTION_PATHS = [
    "crates/selene-graph/src/graph.rs",
    "crates/selene-graph/src/lib.rs",
    "crates/selene-graph/src/store.rs",
    "crates/selene-algorithms/src/projection.rs",
    "crates/selene-algorithms/src/projection/csr.rs",
    "crates/selene-algorithms/src/projection/row_index.rs",
    "crates/selene-algorithms/src/snapshot_summary.rs",
    "crates/selene-gql/src/plan/optimize/index_catalog.rs",
    "crates/selene-gql/src/plan/optimize/live_index_catalog.rs",
    "crates/selene-gql/src/runtime/edge_access.rs",
    "crates/selene-gql/src/runtime/expand.rs",
    "crates/selene-gql/src/runtime/property_filter_rows.rs",
    "crates/selene-gql/src/runtime/questioned.rs",
    "crates/selene-gql/src/runtime/scan.rs",
    "crates/selene-gql/src/runtime/scan_seed.rs",
    "crates/selene-gql/src/runtime/builtins/retrieval_filter.rs",
    "crates/selene-gql/src/runtime/builtins/text_search.rs",
    "crates/selene-gql/src/runtime/builtins/vector_search_ann.rs",
    "crates/selene-gql/src/runtime/builtins/verify/checks.rs",
    "crates/selene-gql/src/runtime/native_algorithms/centrality/pagerank_filter.rs",
    "crates/selene-testing/src/algo_corpus/fixtures.rs",
    "crates/selene-testing/src/bench_fixtures.rs",
    "crates/selene-testing/src/local_omlx/corpus.rs",
    "crates/selene-testing/src/local_omlx/corpus/code_alias.rs",
    "crates/selene-db/src/lib.rs",
]


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
        *,
        write_projections: bool = False,
        delivery_part: str | None = None,
        diff_base: str | None = None,
        diff_head: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        command = [sys.executable, "-B", str(CHECKER), "--root", str(root or self.root)]
        if write_projections:
            command.append("--write-projections")
        for option, value in (
            ("--delivery-part", delivery_part),
            ("--diff-base", diff_base),
            ("--diff-head", diff_head),
        ):
            if value is not None:
                command.extend((option, value))
        return subprocess.run(
            command,
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def git(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def commit_all(self, message: str) -> str:
        self.git("add", "--all")
        self.git(
            "-c",
            "user.name=Selene Plan Test",
            "-c",
            "user.email=plan-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            message,
        )
        return self.git("rev-parse", "HEAD").stdout.strip()

    def write_production_file(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

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
        regenerated = self.run_validator(write_projections=True)
        self.assertEqual(regenerated.returncode, 0, regenerated.stderr)
        current = self.run_validator()
        self.assertEqual(current.returncode, 0, current.stderr)

    def test_m04_pr02_missing_delivery_parts_fails(self) -> None:
        def remove_parts(plan: dict[str, Any]) -> None:
            next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02").pop("delivery_parts")

        result = self.mutate_plan(remove_parts)
        self.assert_failure(result, "M04-PR02: delivery_parts must contain exactly 4 parts; got 0")

    def test_m04_pr02_fewer_delivery_parts_fails(self) -> None:
        def remove_part(plan: dict[str, Any]) -> None:
            next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")["delivery_parts"].pop()

        result = self.mutate_plan(remove_part)
        self.assert_failure(result, "M04-PR02: delivery_parts must contain exactly 4 parts; got 3")

    def test_delivery_part_numbers_must_be_sequential(self) -> None:
        def skip_number(plan: dict[str, Any]) -> None:
            parts = next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")["delivery_parts"]
            parts[1]["number"] = 4

        result = self.mutate_plan(skip_number)
        self.assert_failure(result, "delivery part numbers must be sequential starting at 1; got [1, 4, 3, 4]")

    def test_delivery_part_path_count_cannot_exceed_file_budget(self) -> None:
        def lower_budget(plan: dict[str, Any]) -> None:
            parts = next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")["delivery_parts"]
            parts[0]["max_production_files"] = len(parts[0]["production_paths"]) - 1

        result = self.mutate_plan(lower_budget)
        self.assert_failure(result, "production path count 17 exceeds max_production_files 16")

    def test_m04_pr02_structured_transitions_are_pinned(self) -> None:
        original = self.read_plan()
        cases: tuple[tuple[int, str, Any, str], ...] = (
            (0, "work_item_status_after", "Merged", "work_item_status_after must be 'Unmerged'"),
            (0, "issue_state_after", "Closed", "issue_state_after must be 'Open'"),
            (0, "dependents_unblocked_after", True, "dependents_unblocked_after must be False"),
            (0, "bridge_state_after", "Deleted", "bridge_state_after must be 'Retained'"),
            (1, "work_item_status_after", "Merged", "work_item_status_after must be 'Unmerged'"),
            (1, "issue_state_after", "Closed", "issue_state_after must be 'Open'"),
            (1, "dependents_unblocked_after", True, "dependents_unblocked_after must be False"),
            (1, "bridge_state_after", "Deleted", "bridge_state_after must be 'Retained'"),
            (2, "work_item_status_after", "Merged", "work_item_status_after must be 'Unmerged'"),
            (2, "issue_state_after", "Closed", "issue_state_after must be 'Open'"),
            (2, "dependents_unblocked_after", True, "dependents_unblocked_after must be False"),
            (2, "bridge_state_after", "Retained", "bridge_state_after must be 'Deleted'"),
            (3, "work_item_status_after", "Unmerged", "work_item_status_after must be 'Merged'"),
            (3, "issue_state_after", "Open", "issue_state_after must be 'Closed'"),
            (3, "dependents_unblocked_after", False, "dependents_unblocked_after must be True"),
        )
        for part_index, field, value, message in cases:
            with self.subTest(part=part_index + 1, field=field):
                plan = json.loads(json.dumps(original))
                work_item = next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")
                work_item["delivery_parts"][part_index][field] = value
                self.write_plan(plan)
                self.assert_failure(self.run_validator(), message)
        self.write_plan(original)

    def test_m04_pr02_bridge_deletion_cannot_regress(self) -> None:
        def restore_bridge(plan: dict[str, Any]) -> None:
            parts = next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")["delivery_parts"]
            parts[3]["bridge_state_after"] = "Retained"

        result = self.mutate_plan(restore_bridge)
        self.assert_failure(result, "bridge_state_after regresses at delivery part 4")

    def test_m04_pr02_part_3a_and_3b_production_inventories_are_pinned(self) -> None:
        original = self.read_plan()
        parts = next(item for item in original["pull_requests"] if item["id"] == "M04-PR02")["delivery_parts"]
        self.assertEqual(parts[2]["title"], "Part 3A: Graph-internal bridge deletion")
        self.assertEqual(parts[3]["title"], "Part 3B: Downstream migration and final public-row deletion")
        self.assertEqual(len(parts[2]["production_paths"]), 13)
        self.assertEqual(len(parts[3]["production_paths"]), 25)
        self.assertEqual(parts[2]["production_paths"], M04_PART_3A_PRODUCTION_PATHS)
        self.assertEqual(parts[3]["production_paths"], M04_PART_3B_PRODUCTION_PATHS)
        for part_index, label in ((2, "Part 3A"), (3, "Part 3B")):
            with self.subTest(part=label):
                plan = json.loads(json.dumps(original))
                work_item = next(item for item in plan["pull_requests"] if item["id"] == "M04-PR02")
                work_item["delivery_parts"][part_index]["production_paths"][-1] = (
                    "crates/selene-graph/src/unlisted.rs"
                )
                self.write_plan(plan)
                self.assert_failure(self.run_validator(), f"must match the exact {label} inventory")
        self.write_plan(original)

    def test_delivery_diff_allows_listed_production_path(self) -> None:
        base = self.commit_all("base")
        self.write_production_file("crates/selene-graph/src/candidate_set.rs", "pub struct CandidateSet;\n")
        self.write_production_file("crates/selene-graph/src/tests/unlisted.rs", "compile_error!(\"test-only\");\n")
        self.write_production_file("crates/selene-graph/src/ignored_tests.rs", "compile_error!(\"test-only\");\n")
        for directory in ("benches", "examples", "docs", "generated"):
            self.write_production_file(
                f"crates/selene-graph/src/{directory}/unlisted.rs",
                "compile_error!(\"non-production\");\n",
            )
        self.write_production_file("crates/selene-graph/src/generated.rs", "compile_error!(\"generated\");\n")
        head = self.commit_all("listed production path")

        result = self.run_validator(delivery_part="M04-PR02:1", diff_base=base, diff_head=head)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("delivery part diff passed: M04-PR02:1", result.stdout)
        self.assertIn("production_files=1 net_lines=1", result.stdout)

    def test_delivery_diff_allows_part_3b_graph_api_owner_paths(self) -> None:
        base = self.commit_all("base")
        for path in (
            "crates/selene-graph/src/graph.rs",
            "crates/selene-graph/src/lib.rs",
            "crates/selene-graph/src/store.rs",
        ):
            self.write_production_file(path, "pub struct RemovedRowApi;\n")
        head = self.commit_all("part 3b graph API owners")

        result = self.run_validator(delivery_part="M04-PR02:4", diff_base=base, diff_head=head)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("delivery part diff passed: M04-PR02:4", result.stdout)
        self.assertIn("production_files=3 net_lines=3", result.stdout)

    def test_delivery_diff_rejects_unlisted_production_path(self) -> None:
        base = self.commit_all("base")
        self.write_production_file("crates/selene-graph/src/unlisted.rs", "pub struct Unlisted;\n")
        head = self.commit_all("unlisted production path")

        result = self.run_validator(delivery_part="M04-PR02:1", diff_base=base, diff_head=head)
        self.assert_failure(result, "unlisted production paths: ['crates/selene-graph/src/unlisted.rs']")

    def test_delivery_diff_rejects_1501_net_production_lines(self) -> None:
        base = self.commit_all("base")
        self.write_production_file("crates/selene-graph/src/candidate_set.rs", "// counted line\n" * 1501)
        head = self.commit_all("over-budget production path")

        result = self.run_validator(delivery_part="M04-PR02:1", diff_base=base, diff_head=head)
        self.assert_failure(result, "net production line change 1501 exceeds declared limit 1500")
        self.assertIn("net production line change 1501 exceeds D-021 default 1500", result.stderr)

    def test_delivery_diff_arguments_are_complete_and_well_formed(self) -> None:
        partial = self.run_validator(delivery_part="M04-PR02:1")
        self.assert_failure(partial, "--delivery-part, --diff-base, and --diff-head are required together")
        malformed = self.run_validator(delivery_part="M04-PR02", diff_base="HEAD", diff_head="HEAD")
        self.assert_failure(malformed, "malformed selector 'M04-PR02'; expected WORK-ITEM:PART")
        unknown_item = self.run_validator(delivery_part="M10-PR99:1", diff_base="HEAD", diff_head="HEAD")
        self.assert_failure(unknown_item, "unknown work item M10-PR99")
        unknown = self.run_validator(delivery_part="M04-PR02:5", diff_base="HEAD", diff_head="HEAD")
        self.assert_failure(unknown, "unknown delivery part M04-PR02:5")

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
