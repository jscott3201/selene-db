#!/usr/bin/env python3
"""Regression tests for the executable 1.x baseline harness."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import shutil
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


def tree_bytes(root: pathlib.Path) -> dict[str, bytes]:
    return {
        str(path.relative_to(root)): path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def publication_debris(destination: pathlib.Path) -> list[pathlib.Path]:
    prefix = f".{destination.name}."
    return sorted(path for path in destination.parent.iterdir() if path.name.startswith(prefix))


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

    def test_api_inventory_rejects_incomplete_or_mismatched_content(self) -> None:
        manifest = json.loads((ROOT / "docs/v2/baseline/manifest.json").read_text())
        inventory = json.loads((ROOT / "docs/v2/baseline/api-inventory.json").read_text())

        def contract_errors(changed_manifest: dict, changed_inventory: dict) -> list[str]:
            errors = baseline.api_inventory_errors(changed_inventory)
            if not errors:
                digest = baseline.sha256_bytes(baseline.canonical_json(changed_inventory).encode())
                errors.extend(
                    baseline.cross_file_inventory_errors(changed_manifest, changed_inventory, digest)
                )
            return errors

        def mutated() -> tuple[dict, dict]:
            return json.loads(json.dumps(manifest)), json.loads(json.dumps(inventory))

        cases = []
        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"] = []
        cases.append(("empty inventory", changed_manifest, changed_inventory, "crate identities differ"))

        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"][0]["paths"] = []
        cases.append(("empty paths", changed_manifest, changed_inventory, "has no public paths"))

        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"].pop()
        cases.append(("missing crate", changed_manifest, changed_inventory, "crate identities differ"))

        changed_manifest, changed_inventory = mutated()
        extra_crate = json.loads(json.dumps(changed_inventory["crates"][0]))
        extra_crate["package"] = "unexpected-package"
        extra_crate["crate"] = "unexpected_crate"
        changed_inventory["crates"].append(extra_crate)
        cases.append(("extra crate", changed_manifest, changed_inventory, "unexpected crate identity"))

        for field, value in (("package", "wrong-package"), ("crate", "wrong_crate"), ("crate_version", "0.0.0")):
            changed_manifest, changed_inventory = mutated()
            changed_inventory["crates"][0][field] = value
            cases.append((f"wrong {field}", changed_manifest, changed_inventory, field.replace("crate_", "")))

        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"].append(json.loads(json.dumps(changed_inventory["crates"][0])))
        cases.append(("duplicate crate", changed_manifest, changed_inventory, "duplicate crate identity"))

        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"][0]["paths"].append(
            json.loads(json.dumps(changed_inventory["crates"][0]["paths"][0]))
        )
        cases.append(("duplicate path", changed_manifest, changed_inventory, "duplicate paths path"))

        changed_manifest, changed_inventory = mutated()
        changed_inventory["crates"][0]["paths"][0]["path"] = "not a rust path"
        cases.append(("malformed path", changed_manifest, changed_inventory, "malformed paths path"))

        changed_manifest, changed_inventory = mutated()
        changed_manifest["deterministic"]["public_api"]["crates"][0]["path_count"] += 1
        cases.append(("per-crate count", changed_manifest, changed_inventory, "path_count mismatch"))

        changed_manifest, changed_inventory = mutated()
        changed_manifest["deterministic"]["public_api"]["path_count"] += 1
        cases.append(("aggregate count", changed_manifest, changed_inventory, "aggregate path_count mismatch"))

        for name, changed_manifest, changed_inventory, expected in cases:
            with self.subTest(name=name):
                self.assertTrue(
                    any(expected in error for error in contract_errors(changed_manifest, changed_inventory)),
                    contract_errors(changed_manifest, changed_inventory),
                )

    def test_capture_publication_failure_preserves_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-capture-publish-") as raw:
            root = pathlib.Path(raw)
            destination = root / "target" / "v2-baseline" / baseline.ARCHIVE_SHA
            destination.mkdir(parents=True)
            (destination / "manifest.json").write_text("authoritative\n")
            before = tree_bytes(destination)
            staged = baseline._staged_sibling(destination, "capture")
            (staged / "manifest.json").write_text("partial replacement\n")

            def fail_after_publish(phase: str) -> None:
                if phase == "after_publish":
                    raise RuntimeError("injected capture publication failure")

            with self.assertRaisesRegex(RuntimeError, "injected capture publication failure"):
                baseline._publish_directory(staged, destination, lambda: [], fail_after_publish)
            self.assertEqual(tree_bytes(destination), before)
            self.assertEqual(publication_debris(destination), [])

    def test_install_publication_failure_preserves_existing_package(self) -> None:
        with tempfile.TemporaryDirectory(prefix="selene-baseline-install-publish-") as raw:
            root = pathlib.Path(raw)
            baseline.copy_baseline_package(ROOT, root)
            destination = root / baseline.BASELINE_RELATIVE
            before = tree_bytes(destination)
            evidence = root / "evidence"
            reports = evidence / "reports"
            reports.mkdir(parents=True)
            for name in ("manifest.json", "manifest.schema.json", "api-inventory.json"):
                shutil.copyfile(ROOT / baseline.BASELINE_RELATIVE / name, evidence / name)
            for report in baseline.REPORT_PATHS:
                shutil.copyfile(ROOT / baseline.BASELINE_RELATIVE / report, reports / report)

            def fail_after_publish(phase: str) -> None:
                if phase == "after_publish":
                    raise RuntimeError("injected install publication failure")

            with self.assertRaisesRegex(RuntimeError, "injected install publication failure"):
                baseline.install(root, evidence, fail_after_publish)
            self.assertEqual(tree_bytes(destination), before)
            self.assertEqual(publication_debris(destination), [])

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
