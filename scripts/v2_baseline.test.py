#!/usr/bin/env python3
"""Regression tests for the executable 1.x baseline harness."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts" / "v2_baseline.py"
SPEC = importlib.util.spec_from_file_location("v2_baseline", HELPER)
assert SPEC is not None and SPEC.loader is not None
baseline = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = baseline
SPEC.loader.exec_module(baseline)


class BaselineHarnessTests(unittest.TestCase):
    def test_wrong_source_sha_is_rejected(self) -> None:
        with self.assertRaisesRegex(baseline.BaselineError, "source SHA must be"):
            baseline.validate_source_sha("0" * 40)

    def test_absent_archive_commit_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-no-commit-") as raw:
            root = pathlib.Path(raw)
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            with self.assertRaisesRegex(baseline.BaselineError, "does not resolve"):
                baseline.resolve_archive_commit(root, baseline.ARCHIVE_SHA)

    def test_canonical_checkout_cannot_be_used_as_clone(self) -> None:
        with self.assertRaisesRegex(baseline.BaselineError, "canonical repository"):
            baseline.validate_clone_location(ROOT, ROOT)

    def test_output_must_stay_under_controlled_directory(self) -> None:
        expected = ROOT / "target" / "v2-baseline" / baseline.ARCHIVE_SHA
        self.assertEqual(baseline.controlled_output_dir(ROOT, None), expected)
        with self.assertRaisesRegex(baseline.BaselineError, "controlled directory"):
            baseline.controlled_output_dir(ROOT, ROOT.parent / "baseline-output")

    def test_capture_rejects_only_unrelated_worktree_paths(self) -> None:
        baseline.validate_capture_worktree(ROOT, ["docs/v2/baseline/README.md", "scripts/v2_baseline.py"])
        with self.assertRaisesRegex(baseline.BaselineError, "unrelated worktree paths"):
            baseline.validate_capture_worktree(ROOT, ["crates/selene-core/src/lib.rs"])

    def test_failed_command_is_retained_with_log_hash(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-command-") as raw:
            output = pathlib.Path(raw)
            result = baseline.run_command(
                command_id="expected-failure",
                argv=[sys.executable, "-c", "print('kept output'); raise SystemExit(7)"],
                cwd=ROOT,
                logs_dir=output,
                environment={},
                secret_values=set(),
            )
            self.assertEqual(result["disposition"], "failed")
            self.assertEqual(result["exit_code"], 7)
            self.assertEqual(len(result["output_sha256"]), 64)
            self.assertIn("kept output", (output / "expected-failure.log").read_text())

    def test_secret_output_is_redacted_and_fails_command(self) -> None:
        secret = "baseline-super-secret-token"
        with tempfile.TemporaryDirectory(prefix="selene-baseline-secret-") as raw:
            output = pathlib.Path(raw)
            result = baseline.run_command(
                command_id="secret-output",
                argv=[sys.executable, "-c", f"print({secret!r})"],
                cwd=ROOT,
                logs_dir=output,
                environment={},
                secret_values={secret},
            )
            log = (output / "secret-output.log").read_text()
            self.assertEqual(result["disposition"], "failed")
            self.assertIn("secret material", result["reason"])
            self.assertNotIn(secret, log)
            self.assertIn("[REDACTED]", log)

    def test_public_api_uses_only_local_rustdoc_paths(self) -> None:
        rustdoc = {
            "format_version": 60,
            "root": "0",
            "crate_version": "1.4.0",
            "includes_private": False,
            "paths": {
                "0": {"crate_id": 0, "path": ["demo"], "kind": "module"},
                "1": {"crate_id": 0, "path": ["demo", "Thing"], "kind": "struct"},
                "2": {"crate_id": 1, "path": ["std", "vec", "Vec"], "kind": "struct"},
            },
            "index": {
                "0": {"crate_id": 0, "docs": "```rust\nlet _ = 1;\n```"},
                "1": {"crate_id": 0, "docs": None},
                "2": {"crate_id": 1, "docs": "```rust\nexternal();\n```"},
            },
        }
        inventory = baseline.inventory_rustdoc_crate("demo-package", "demo", rustdoc)
        self.assertEqual([item["path"] for item in inventory["paths"]], ["demo", "demo::Thing"])
        self.assertEqual(len(inventory["examples"]), 1)

    def test_public_api_bounds_declared_items_to_local_traits_and_inherent_impls(self) -> None:
        rustdoc = {
            "format_version": 61,
            "crate_version": "1.4.0",
            "includes_private": False,
            "paths": {
                "1": {"crate_id": 0, "path": ["demo", "Thing"], "kind": "struct"},
                "5": {"crate_id": 0, "path": ["demo", "Behavior"], "kind": "trait"},
                "20": {"crate_id": 1, "path": ["dependency", "Foreign"], "kind": "struct"},
            },
            "index": {
                "1": {"crate_id": 0, "docs": None, "inner": {"struct": {"impls": ["3", "4", "10"]}}},
                "3": {"crate_id": 0, "inner": {"impl": {"trait": None, "is_synthetic": False, "items": ["7", "8"]}}},
                "4": {"crate_id": 0, "inner": {"impl": {"trait": {"path": ["std", "fmt", "Debug"]}, "items": ["9"]}}},
                "5": {"crate_id": 0, "docs": None, "inner": {"trait": {"items": ["6"]}}},
                "6": {"crate_id": 0, "name": "act", "visibility": "default", "docs": None, "inner": {"function": {}}},
                "7": {"crate_id": 0, "name": "new", "visibility": "public", "docs": None, "inner": {"function": {}}},
                "8": {"crate_id": 0, "name": "private", "visibility": "crate", "docs": None, "inner": {"function": {}}},
                "9": {"crate_id": 0, "name": "fmt", "visibility": "public", "docs": None, "inner": {"function": {}}},
                "10": {"crate_id": 1, "inner": {"impl": {"trait": None, "is_synthetic": False, "items": ["11"]}}},
                "11": {"crate_id": 1, "name": "foreign", "visibility": "public", "docs": None, "inner": {"function": {}}},
                "20": {"crate_id": 1, "docs": None, "inner": {"struct": {"impls": []}}},
            },
        }
        inventory = baseline.inventory_rustdoc_crate("demo-package", "demo", rustdoc)
        self.assertEqual(
            inventory["declared_items"],
            [
                {"path": "demo::Behavior::act", "kind": "function"},
                {"path": "demo::Thing::new", "kind": "function"},
            ],
        )

    def test_zero_benchmark_matches_is_failure(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-bench-") as raw:
            with self.assertRaisesRegex(baseline.BaselineError, "matched zero Criterion results"):
                baseline.collect_criterion_results(pathlib.Path(raw), {})

    def test_criterion_annotations_are_limited_to_printed_facts(self) -> None:
        result = {
            "id": "group/case",
            "p_value": None,
            "comparison_assessment": None,
            "outlier_count": 0,
            "outlier_percent": 0.0,
        }
        with tempfile.TemporaryDirectory(prefix="selene-baseline-criterion-") as raw:
            log = pathlib.Path(raw) / "run.log"
            log.write_text(
                "group/case\n  change: time: [+1% +2% +3%] (p = 0.33 > 0.05)\n"
                "  No change in performance detected.\n"
                "Found 4 outliers among 100 measurements (4.00%)\n"
            )
            baseline.annotate_criterion_log([result], log)
        self.assertEqual(result["p_value"], 0.33)
        self.assertEqual(result["comparison_assessment"], "No change in performance detected.")
        self.assertEqual((result["outlier_count"], result["outlier_percent"]), (4, 4.0))

    def test_manifest_schema_is_closed(self) -> None:
        schema = json.loads((ROOT / "docs/v2/baseline/manifest.schema.json").read_text())
        errors = baseline.closed_schema_errors(schema)
        self.assertEqual(errors, [])

    def test_missing_and_extra_manifest_fields_fail(self) -> None:
        manifest_path = ROOT / "docs/v2/baseline/manifest.json"
        manifest = json.loads(manifest_path.read_text())
        schema = json.loads((ROOT / "docs/v2/baseline/manifest.schema.json").read_text())

        missing = json.loads(json.dumps(manifest))
        del missing["archive"]["source_sha"]
        self.assertTrue(any("missing required property 'source_sha'" in error for error in baseline.schema_errors(missing, schema)))

        extra = json.loads(json.dumps(manifest))
        extra["archive"]["unexpected"] = True
        self.assertTrue(any("unexpected property 'unexpected'" in error for error in baseline.schema_errors(extra, schema)))

    def test_bad_report_hash_fails_validation(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-hash-") as raw:
            root = pathlib.Path(raw)
            baseline.copy_baseline_package(ROOT, root)
            manifest_path = root / "docs/v2/baseline/manifest.json"
            manifest = json.loads(manifest_path.read_text())
            manifest["reports"][0]["sha256"] = "0" * 64
            manifest_path.write_text(baseline.canonical_json(manifest))
            errors = baseline.validate_tracked_baseline(root)
            self.assertTrue(any("report hash mismatch" in error for error in errors))

    def test_deterministic_render_repeats_byte_for_byte(self) -> None:
        manifest = json.loads((ROOT / "docs/v2/baseline/manifest.json").read_text())
        with tempfile.TemporaryDirectory(prefix="selene-baseline-render-a-") as a_raw, tempfile.TemporaryDirectory(
            prefix="selene-baseline-render-b-"
        ) as b_raw:
            a = pathlib.Path(a_raw)
            b = pathlib.Path(b_raw)
            baseline.render_reports(manifest, a)
            baseline.render_reports(manifest, b)
            for report in baseline.REPORT_PATHS:
                self.assertEqual((a / report.name).read_bytes(), (b / report.name).read_bytes())


if __name__ == "__main__":
    unittest.main(verbosity=2)
