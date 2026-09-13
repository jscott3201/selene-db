#!/usr/bin/env python3
"""Capture, render, and validate the final 1.x executable baseline."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import platform
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from collections.abc import Callable
from typing import Any


ARCHIVE_SHA = "b8782bec34ff0b815b62711ac7e33cac09d8ea71"
ARCHIVE_VERSION = "1.4.0"
EXPECTED_BASE_SHA = "b7ea652bbf79b48efb6c9ae63deb485f26a69bb9"
SCHEMA_VERSION = 1
REPORT_PATHS = tuple(
    pathlib.Path(name)
    for name in ("README.md", "gates.md", "public-api.md", "formats.md", "benchmarks.md")
)
BASELINE_RELATIVE = pathlib.Path("docs/v2/baseline")
OWNED_CAPTURE_PATHS = (
    ".github/scripts/check-v2-plan.py",
    ".github/scripts/check-v2-plan.test.py",
    "docs/v2/README.md",
    "docs/v2/baseline/",
    "docs/v2/roadmap/plan.json",
    "docs/v2/roadmap/milestones.md",
    "docs/v2/roadmap/work-items-00-04.md",
    "docs/v2/roadmap/work-items-05-10.md",
    "scripts/v2-baseline.sh",
    "scripts/v2_baseline.py",
    "scripts/v2_baseline.test.py",
)
SERVICE_ENV_PREFIXES = (
    "OPENAI_",
    "OPENROUTER_",
    "SELENE_EMBEDDING_",
    "SELENE_OMLX_",
    "OMLX_",
)
DISPOSITIONS = {"passed", "failed", "unavailable", "skipped", "not_applicable"}
EXPECTED_SCHEMA_SHA256 = "464cfc99a93c16677d95393a7238df273a4f9ca0517beb570a5ffddfc8b9535a"
API_INVENTORY_SCOPE = (
    "The inventory contains local public rustdoc paths, items declared by local traits, and public inherent impl items only; "
    "trait and blanket impl entries and dependency-associated surfaces are excluded."
)
RUST_PATH = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*$")


class BaselineError(RuntimeError):
    """Raised when baseline evidence cannot satisfy its contract."""


def canonical_json(value: Any) -> str:
    """Return the repository's canonical JSON representation."""

    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def validate_source_sha(source_sha: str) -> None:
    if source_sha != ARCHIVE_SHA:
        raise BaselineError(f"source SHA must be {ARCHIVE_SHA}, got {source_sha}")


def git_output(root: pathlib.Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=root, check=True, capture_output=True, text=True
    )
    return completed.stdout.strip()


def resolve_archive_commit(root: pathlib.Path, source_sha: str) -> str:
    validate_source_sha(source_sha)
    completed = subprocess.run(
        ["git", "rev-parse", "--verify", f"{source_sha}^{{commit}}"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0 or completed.stdout.strip() != source_sha:
        raise BaselineError(f"archive commit {source_sha} does not resolve exactly in {root}")
    return completed.stdout.strip()


def validate_clone_location(root: pathlib.Path, clone: pathlib.Path) -> None:
    if root.resolve() == clone.resolve():
        raise BaselineError("the canonical repository cannot be used as the archival checkout")
    if root.resolve() in clone.resolve().parents:
        raise BaselineError("the archival checkout must not live inside the canonical repository")


def controlled_output_dir(root: pathlib.Path, requested: pathlib.Path | None) -> pathlib.Path:
    expected_parent = (root / "target" / "v2-baseline").resolve()
    candidate = (requested or expected_parent / ARCHIVE_SHA).resolve()
    if candidate != expected_parent / ARCHIVE_SHA:
        raise BaselineError(
            f"baseline output must use the controlled directory {expected_parent / ARCHIVE_SHA}"
        )
    return candidate


def _staged_sibling(destination: pathlib.Path, purpose: str) -> pathlib.Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    return pathlib.Path(
        tempfile.mkdtemp(prefix=f".{destination.name}.{purpose}-", dir=destination.parent)
    )


def _publish_directory(
    staged: pathlib.Path,
    destination: pathlib.Path,
    validate: Callable[[], list[str]],
    failure_hook: Callable[[str], None] | None = None,
) -> None:
    """Replace one directory while retaining the prior tree until validation passes."""

    backup = _staged_sibling(destination, "backup")
    backup.rmdir()
    had_destination = destination.exists()
    published = False
    try:
        if had_destination:
            os.replace(destination, backup)
        if failure_hook:
            failure_hook("after_backup")
        os.replace(staged, destination)
        published = True
        if failure_hook:
            failure_hook("after_publish")
        errors = validate()
        if errors:
            raise BaselineError("published package failed validation: " + "; ".join(errors))
        if failure_hook:
            failure_hook("after_validation")
    except BaseException as error:
        try:
            if published and destination.exists():
                shutil.rmtree(destination)
            if had_destination and backup.exists():
                os.replace(backup, destination)
        except OSError as rollback_error:
            raise BaselineError(
                f"publication failed and rollback also failed: {rollback_error}"
            ) from error
        raise
    else:
        if backup.exists():
            shutil.rmtree(backup)


def validate_capture_worktree(root: pathlib.Path, dirty_paths: list[str] | None = None) -> None:
    if dirty_paths is None:
        output = subprocess.run(
            ["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"],
            cwd=root,
            check=True,
            capture_output=True,
        ).stdout
        records = [record.decode(errors="replace") for record in output.split(b"\0") if record]
        dirty_paths = [record[3:] for record in records if len(record) > 3 and not record.startswith("R ")]
    unrelated = sorted(
        path for path in dirty_paths
        if not any(path == owned or (owned.endswith("/") and path.startswith(owned)) for owned in OWNED_CAPTURE_PATHS)
    )
    if unrelated:
        raise BaselineError(f"unrelated worktree paths are dirty: {', '.join(unrelated)}")


def _redact(raw: bytes, secret_values: set[str]) -> tuple[bytes, bool]:
    text = raw.decode("utf-8", errors="replace")
    found = False
    for secret in sorted((secret for secret in secret_values if secret), key=len, reverse=True):
        if secret in text:
            text = text.replace(secret, "[REDACTED]")
            found = True
    return text.encode("utf-8"), found


def _command_text(argv: list[str]) -> str:
    return " ".join(shlex.quote(part) for part in argv)


def run_command(
    *,
    command_id: str,
    argv: list[str],
    cwd: pathlib.Path,
    logs_dir: pathlib.Path,
    environment: dict[str, str],
    secret_values: set[str],
    lane: str = "archive",
    timeout: int | None = None,
) -> dict[str, Any]:
    """Run one evidence command and retain redacted output regardless of result."""

    logs_dir.mkdir(parents=True, exist_ok=True)
    log_path = logs_dir / f"{command_id}.log"
    started_at = utc_now()
    start = time.monotonic()
    exit_code: int | None
    disposition: str
    reason: str | None = None
    try:
        completed = subprocess.run(
            argv,
            cwd=cwd,
            env=environment or None,
            capture_output=True,
            timeout=timeout,
        )
        raw = completed.stdout + completed.stderr
        exit_code = completed.returncode
        disposition = "passed" if exit_code == 0 else "failed"
    except FileNotFoundError as error:
        raw = f"unavailable: {error}\n".encode()
        exit_code = None
        disposition = "unavailable"
        reason = str(error)
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout or b""
        stderr = error.stderr or b""
        if isinstance(stdout, str):
            stdout = stdout.encode()
        if isinstance(stderr, str):
            stderr = stderr.encode()
        raw = stdout + stderr + f"\ntimed out after {timeout} seconds\n".encode()
        exit_code = 124
        disposition = "failed"
        reason = f"timed out after {timeout} seconds"

    sanitized, leaked = _redact(raw, secret_values)
    if leaked:
        disposition = "failed"
        reason = "command output contained secret material and was redacted"
        if exit_code == 0:
            exit_code = 1
    log_path.write_bytes(sanitized)
    if reason is None and disposition == "failed":
        lines = [line.strip() for line in sanitized.decode(errors="replace").splitlines() if line.strip()]
        reason = lines[-1][:300] if lines else f"command exited {exit_code}"

    return {
        "id": command_id,
        "lane": lane,
        "command": _command_text(argv),
        "cwd": str(cwd),
        "disposition": disposition,
        "exit_code": exit_code,
        "reason": reason,
        "started_at": started_at,
        "duration_seconds": round(time.monotonic() - start, 3),
        "output_log": f"logs/{log_path.name}",
        "output_sha256": sha256_file(log_path),
        "output_bytes": log_path.stat().st_size,
    }


def unavailable_command(
    command_id: str, command: str, cwd: pathlib.Path, reason: str, lane: str = "archive",
    disposition: str = "unavailable",
) -> dict[str, Any]:
    return {
        "id": command_id,
        "lane": lane,
        "command": command,
        "cwd": str(cwd),
        "disposition": disposition,
        "exit_code": None,
        "reason": reason,
        "started_at": utc_now(),
        "duration_seconds": 0.0,
        "output_log": None,
        "output_sha256": None,
        "output_bytes": None,
    }


def _rust_fences(docs: str | None) -> list[str]:
    if not docs:
        return []
    examples: list[str] = []
    for match in re.finditer(r"```([^\n`]*)\n(.*?)```", docs, flags=re.DOTALL):
        attributes = {part.strip() for part in match.group(1).split(",") if part.strip()}
        explicitly_other = attributes.intersection(
            {"bash", "console", "gql", "json", "markdown", "sh", "text", "toml", "yaml"}
        )
        if not explicitly_other:
            examples.append(match.group(2))
    return examples


def _item_inner(item: dict[str, Any], kind: str) -> dict[str, Any]:
    return item.get("inner", {}).get(kind, {})


def inventory_rustdoc_crate(
    package: str, crate_name: str, rustdoc: dict[str, Any]
) -> dict[str, Any]:
    """Extract local public paths, locally declared items, and Rust examples."""

    if rustdoc.get("includes_private") is not False:
        raise BaselineError(f"rustdoc JSON for {package} unexpectedly includes private items")
    index = rustdoc.get("index", {})
    local_paths: list[dict[str, Any]] = []
    examples: list[dict[str, Any]] = []
    seen_examples: set[tuple[str, str]] = set()
    declared: list[dict[str, Any]] = []
    seen_declared: set[tuple[str, str]] = set()

    paths_by_id = {
        str(item_id): entry
        for item_id, entry in rustdoc.get("paths", {}).items()
        if entry.get("crate_id") == 0
    }
    for item_id, entry in paths_by_id.items():
        path = "::".join(entry["path"])
        local_paths.append({"path": path, "kind": entry["kind"]})
        item = index.get(item_id, {})
        for example in _rust_fences(item.get("docs")):
            digest = sha256_bytes(example.encode())
            key = (path, digest)
            if key not in seen_examples:
                examples.append({"item": path, "sha256": digest, "lines": len(example.splitlines())})
                seen_examples.add(key)

        kind = entry.get("kind")
        owner = path
        child_ids: list[tuple[Any, bool]] = []
        if kind == "trait":
            child_ids.extend((child_id, True) for child_id in _item_inner(item, "trait").get("items", []))
        if kind in {"struct", "enum", "union", "primitive"}:
            for impl_id in _item_inner(item, kind).get("impls", []):
                impl_item = index.get(str(impl_id), {})
                if impl_item.get("crate_id") != 0:
                    continue
                impl_data = _item_inner(impl_item, "impl")
                if impl_data.get("trait") is None and not impl_data.get("is_synthetic", False):
                    child_ids.extend((child_id, False) for child_id in impl_data.get("items", []))
        for child_id, trait_item in child_ids:
            child = index.get(str(child_id), {})
            if child.get("crate_id") != 0:
                continue
            if not trait_item and child.get("visibility") != "public":
                continue
            name = child.get("name")
            if not name:
                continue
            symbol_path = f"{owner}::{name}"
            symbol_kind = next(iter(child.get("inner", {})), "unknown")
            key = (symbol_path, symbol_kind)
            if key not in seen_declared:
                declared.append({"path": symbol_path, "kind": symbol_kind})
                seen_declared.add(key)
            for example in _rust_fences(child.get("docs")):
                digest = sha256_bytes(example.encode())
                example_key = (symbol_path, digest)
                if example_key not in seen_examples:
                    examples.append({"item": symbol_path, "sha256": digest, "lines": len(example.splitlines())})
                    seen_examples.add(example_key)

    local_paths.sort(key=lambda item: (item["path"], item["kind"]))
    declared.sort(key=lambda item: (item["path"], item["kind"]))
    examples.sort(key=lambda item: (item["item"], item["sha256"]))
    return {
        "package": package,
        "crate": crate_name,
        "crate_version": rustdoc.get("crate_version"),
        "rustdoc_format_version": rustdoc.get("format_version"),
        "paths": local_paths,
        "declared_items": declared,
        "examples": examples,
    }


def _criterion_snapshot(criterion_dir: pathlib.Path) -> dict[str, str]:
    if not criterion_dir.exists():
        return {}
    return {
        str(path.relative_to(criterion_dir)): sha256_file(path)
        for path in criterion_dir.glob("**/new/estimates.json")
    }


def collect_criterion_results(
    criterion_dir: pathlib.Path, before: dict[str, str]
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for path in sorted(criterion_dir.glob("**/new/estimates.json")):
        relative = str(path.relative_to(criterion_dir))
        digest = sha256_file(path)
        if before.get(relative) == digest:
            continue
        estimates = json.loads(path.read_text())
        result_dir = path.parent
        sample_path = result_dir / "sample.json"
        benchmark_path = result_dir / "benchmark.json"
        sample_count = None
        if sample_path.exists():
            sample = json.loads(sample_path.read_text())
            sample_count = len(sample.get("times", []))
        benchmark_id = str(path.parents[1].relative_to(criterion_dir))
        if benchmark_path.exists():
            benchmark_id = json.loads(benchmark_path.read_text()).get("full_id", benchmark_id)
        mean = estimates.get("mean", {})
        median = estimates.get("median", {})
        std_dev = estimates.get("std_dev", {})
        mean_point = mean.get("point_estimate")
        std_point = std_dev.get("point_estimate")
        coefficient = None
        if isinstance(mean_point, (int, float)) and mean_point:
            coefficient = std_point / mean_point if isinstance(std_point, (int, float)) else None
        results.append(
            {
                "id": benchmark_id,
                "sample_count": sample_count,
                "unit": "ns",
                "mean": mean,
                "median": median,
                "std_dev": std_dev,
                "coefficient_of_variation": coefficient,
                "p_value": None,
                "comparison_assessment": None,
                "outlier_count": 0,
                "outlier_percent": 0.0,
                "estimates_sha256": digest,
            }
        )
    if not results:
        raise BaselineError("benchmark command matched zero Criterion results")
    return results


def maximum_cv(results: list[dict[str, Any]]) -> float:
    values = [result["coefficient_of_variation"] for result in results if result["coefficient_of_variation"] is not None]
    return max(values, default=0.0)


def annotate_criterion_log(results: list[dict[str, Any]], log_path: pathlib.Path) -> None:
    """Attach only comparison and outlier facts printed by Criterion."""

    if not log_path.is_file():
        return
    text = log_path.read_text(errors="replace")
    positions = sorted((text.find(f"{result['id']}\n"), result) for result in results)
    positions = [(position, result) for position, result in positions if position >= 0]
    for index, (start, result) in enumerate(positions):
        end = positions[index + 1][0] if index + 1 < len(positions) else len(text)
        block = text[start:end]
        p_value = re.search(r"\(p = ([0-9.]+) [<>] 0\.05\)", block)
        outliers = re.search(r"Found (\d+) outliers among \d+ measurements \(([0-9.]+)%\)", block)
        assessment = next((value for value in ("No change in performance detected.", "Performance has improved.",
                                                "Performance has regressed.") if value in block), None)
        result["p_value"] = float(p_value.group(1)) if p_value else None
        result["comparison_assessment"] = assessment
        result["outlier_count"] = int(outliers.group(1)) if outliers else 0
        result["outlier_percent"] = float(outliers.group(2)) if outliers else 0.0


def _type_matches(value: Any, expected: str) -> bool:
    return {
        "array": isinstance(value, list),
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "null": value is None,
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "object": isinstance(value, dict),
        "string": isinstance(value, str),
    }.get(expected, False)


def schema_errors(value: Any, schema: dict[str, Any], path: str = "$") -> list[str]:
    """Validate the closed baseline schema subset without third-party packages."""

    errors: list[str] = []
    expected_type = schema.get("type")
    if expected_type is not None:
        choices = expected_type if isinstance(expected_type, list) else [expected_type]
        if not any(_type_matches(value, choice) for choice in choices):
            return [f"{path}: expected type {expected_type!r}"]
    if "const" in schema and value != schema["const"]:
        errors.append(f"{path}: expected constant {schema['const']!r}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path}: value {value!r} is not in enum")
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            errors.append(f"{path}: string is shorter than minLength")
        if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None:
            errors.append(f"{path}: string does not match pattern {schema['pattern']!r}")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            errors.append(f"{path}: number is below minimum {schema['minimum']}")
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            errors.append(f"{path}: array has fewer than minItems")
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            errors.append(f"{path}: array has more than maxItems")
        item_schema = schema.get("items")
        if item_schema:
            for index, item in enumerate(value):
                errors.extend(schema_errors(item, item_schema, f"{path}[{index}]"))
    if isinstance(value, dict):
        properties = schema.get("properties", {})
        for required in schema.get("required", []):
            if required not in value:
                errors.append(f"{path}: missing required property '{required}'")
        for name, item in value.items():
            if name in properties:
                errors.extend(schema_errors(item, properties[name], f"{path}.{name}"))
            elif schema.get("additionalProperties") is False:
                errors.append(f"{path}: unexpected property '{name}'")
    return errors


def closed_schema_errors(schema: Any, path: str = "$") -> list[str]:
    errors: list[str] = []
    if isinstance(schema, dict):
        schema_type = schema.get("type")
        types = schema_type if isinstance(schema_type, list) else [schema_type]
        if "object" in types and schema.get("additionalProperties") is not False:
            errors.append(f"{path}: object schema must set additionalProperties=false")
        for key, child in schema.items():
            errors.extend(closed_schema_errors(child, f"{path}.{key}"))
    elif isinstance(schema, list):
        for index, child in enumerate(schema):
            errors.extend(closed_schema_errors(child, f"{path}[{index}]"))
    return errors


def api_inventory_errors(value: Any) -> list[str]:
    errors: list[str] = []
    if not isinstance(value, dict):
        return ["API inventory root is not an object"]
    expected = {"schema_version", "source_sha", "inventory_scope", "disposition_meaning", "crates"}
    if set(value) != expected:
        errors.append(f"API inventory fields differ: expected {sorted(expected)}, got {sorted(value)}")
        return errors
    if value["schema_version"] != 1 or value["source_sha"] != ARCHIVE_SHA:
        errors.append("API inventory version or source SHA is invalid")
    if value["inventory_scope"] != API_INVENTORY_SCOPE:
        errors.append("API inventory scope is invalid")
    crates = value.get("crates")
    if not isinstance(crates, list):
        return [*errors, "API inventory crates is not an array"]
    expected_identities = {(package, crate) for package, crate, _ in PUBLISHED_CRATES}
    seen_identities: set[tuple[str, str]] = set()
    crate_fields = {
        "package",
        "crate",
        "crate_version",
        "rustdoc_format_version",
        "disposition",
        "owner",
        "paths",
        "declared_items",
        "examples",
    }
    symbol_fields = {"path", "kind"}
    example_fields = {"item", "sha256", "lines"}
    for index, crate in enumerate(crates):
        if not isinstance(crate, dict) or set(crate) != crate_fields:
            errors.append(f"API inventory crate {index} has missing or extra fields")
            continue
        identity = (crate["package"], crate["crate"])
        if identity in seen_identities:
            errors.append(f"API inventory has duplicate crate identity {identity!r}")
        seen_identities.add(identity)
        if identity not in expected_identities:
            errors.append(f"API inventory has unexpected crate identity {identity!r}")
        if crate["crate_version"] != ARCHIVE_VERSION:
            errors.append(
                f"API inventory crate {identity!r} has version {crate['crate_version']!r}, expected {ARCHIVE_VERSION!r}"
            )
        if crate.get("disposition") not in {"preserve", "replace", "remove", "internalize"}:
            errors.append(f"API inventory crate {index} has an invalid disposition")
        for collection in ("paths", "declared_items"):
            items = crate.get(collection)
            if not isinstance(items, list):
                errors.append(f"API inventory crate {index} {collection} is not an array")
                continue
            if collection == "paths" and not items:
                errors.append(f"API inventory crate {identity!r} has no public paths")
            seen_paths: set[str] = set()
            for item in items:
                if not isinstance(item, dict) or set(item) != symbol_fields:
                    errors.append(f"API inventory crate {index} has an invalid {collection} item")
                    continue
                path = item["path"]
                if not isinstance(path, str) or RUST_PATH.fullmatch(path) is None:
                    errors.append(f"API inventory crate {index} has a malformed {collection} path")
                elif path != crate["crate"] and not path.startswith(f"{crate['crate']}::"):
                    errors.append(f"API inventory crate {index} has a non-local {collection} path {path!r}")
                if isinstance(path, str):
                    if path in seen_paths:
                        errors.append(f"API inventory crate {index} has duplicate {collection} path {path!r}")
                    seen_paths.add(path)
                if not isinstance(item["kind"], str) or not item["kind"]:
                    errors.append(f"API inventory crate {index} has an invalid {collection} kind")
        examples = crate.get("examples")
        if not isinstance(examples, list):
            errors.append(f"API inventory crate {index} examples is not an array")
            continue
        seen_examples: set[tuple[str, str]] = set()
        for item in examples:
            if not isinstance(item, dict) or set(item) != example_fields:
                errors.append(f"API inventory crate {index} has an invalid example")
                continue
            example_key = (item["item"], item["sha256"])
            if all(isinstance(value, str) for value in example_key):
                if example_key in seen_examples:
                    errors.append(f"API inventory crate {index} has a duplicate example")
                seen_examples.add(example_key)
            if (
                not isinstance(item["item"], str)
                or RUST_PATH.fullmatch(item["item"]) is None
                or not item["item"].startswith(f"{crate['crate']}::")
                or not isinstance(item["sha256"], str)
                or re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) is None
                or not isinstance(item["lines"], int)
                or isinstance(item["lines"], bool)
                or item["lines"] < 1
            ):
                errors.append(f"API inventory crate {index} has malformed example metadata")
    if seen_identities != expected_identities:
        errors.append(
            f"API inventory crate identities differ: expected {sorted(expected_identities)}, got {sorted(seen_identities)}"
        )
    return errors


PUBLISHED_CRATES = (
    ("selene-db-core", "selene_core", "crates/selene-core"),
    ("selene-db-graph", "selene_graph", "crates/selene-graph"),
    ("selene-db-persist", "selene_persist", "crates/selene-persist"),
    ("selene-db-algorithms", "selene_algorithms", "crates/selene-algorithms"),
    ("selene-db-gql", "selene_gql", "crates/selene-gql"),
)
SUPPORT_CRATE = ("selene-db-testing", "selene_testing", "crates/selene-testing")


def _tracked_paths(root: pathlib.Path) -> list[str]:
    raw = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True
    ).stdout
    return sorted(path.decode() for path in raw.split(b"\0") if path)


def _revision_paths(root: pathlib.Path, revision: str) -> list[str]:
    raw = subprocess.run(
        ["git", "ls-tree", "-r", "-z", "--name-only", revision], cwd=root, check=True, capture_output=True
    ).stdout
    return sorted(path.decode() for path in raw.split(b"\0") if path)


def _source_file(root: pathlib.Path, relative: str) -> dict[str, Any]:
    return {"path": relative, "sha256": sha256_file(root / relative)}


def _package_facts(root: pathlib.Path) -> tuple[list[dict[str, Any]], dict[str, str]]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text())
    workspace_package = workspace["workspace"]["package"]
    facts: list[dict[str, Any]] = []
    expected = (*PUBLISHED_CRATES, SUPPORT_CRATE)
    for expected_package, expected_crate, relative in expected:
        manifest_path = root / relative / "Cargo.toml"
        manifest = tomllib.loads(manifest_path.read_text())
        package = manifest["package"]
        package_name = package["name"]
        crate_name = manifest.get("lib", {}).get("name", package_name.replace("-", "_"))
        publish = package.get("publish", True) is not False
        if (package_name, crate_name) != (expected_package, expected_crate):
            raise BaselineError(f"archive package identity changed at {relative}")
        if relative == SUPPORT_CRATE[2] and publish:
            raise BaselineError("selene-testing must remain unpublished support")
        if relative != SUPPORT_CRATE[2] and not publish:
            raise BaselineError(f"published archive crate {package_name} is marked unpublished")
        inherited_version = package.get("version", {}).get("workspace") is True
        version = workspace_package["version"] if inherited_version else package["version"]
        relative_manifest = f"{relative}/Cargo.toml"
        facts.append(
            {
                "package": package_name,
                "crate": crate_name,
                "version": version,
                "publish": publish,
                "manifest_path": relative_manifest,
                "manifest_sha256": sha256_file(manifest_path),
            }
        )
    return facts, {
        "version": workspace_package["version"],
        "edition": workspace_package["edition"],
        "rust_version": workspace_package["rust-version"],
        "repository": workspace_package["repository"],
        "criterion": workspace["workspace"]["dependencies"]["criterion"],
        "mimalloc": workspace["workspace"]["dependencies"]["mimalloc"]["version"],
    }


def _procedure_group(root: pathlib.Path, relative: str, declaration: str) -> dict[str, Any]:
    text = (root / relative).read_text()
    count_match = re.search(rf"const\s+{declaration}:\s*\[[^;]+;\s*(\d+)\s*\]", text)
    if count_match is None:
        raise BaselineError(f"could not read {declaration} count from {relative}")
    names = []
    for body in re.findall(r"name:\s*&\[([^\]]+)\]", text):
        parts = re.findall(r'"([^"]+)"', body)
        if parts:
            names.append(".".join(parts))
    count = int(count_match.group(1))
    if len(names) != count:
        raise BaselineError(
            f"{declaration} declares {count} entries but source extraction found {len(names)}"
        )
    return {"count": count, "names": names, "source": _source_file(root, relative)}


def _feature_facts(root: pathlib.Path) -> dict[str, Any]:
    relative = "crates/selene-core/src/feature_register.rs"
    text = (root / relative).read_text()
    macro = re.search(r"feature_ids!\s*\{(.*?)\n\}", text, flags=re.DOTALL)
    supported = re.search(
        r"pub const SUPPORTED_FEATURES:[^=]+=\s*&\[(.*?)\n\];", text, flags=re.DOTALL
    )
    unsupported = re.search(
        r"pub const NOT_SUPPORTED_RATIONALE:[^=]+=\s*&\[(.*?)\n\];", text, flags=re.DOTALL
    )
    if not macro or not supported or not unsupported:
        raise BaselineError("feature register arrays could not be extracted")
    referenced_ids = re.findall(r'^\s*([A-Z][A-Z0-9_]*)\s*=\s*"', macro.group(1), flags=re.MULTILINE)
    supported_ids = re.findall(r"FeatureId::([A-Z][A-Z0-9_]*)", supported.group(1))
    unsupported_ids = re.findall(r"FeatureId::([A-Z][A-Z0-9_]*)", unsupported.group(1))
    generator = "build/regen_feature_docs.sh"
    generator_text = (root / generator).read_text()
    if "TODO" not in generator_text or "exit 0" not in generator_text:
        raise BaselineError("feature documentation generator is no longer the expected placeholder")
    return {
        "source": _source_file(root, relative),
        "referenced_count": len(referenced_ids),
        "supported_count": len(supported_ids),
        "not_supported_rationale_count": len(unsupported_ids),
        "generated_doc": {
            "path": generator,
            "sha256": sha256_file(root / generator),
            "status": "placeholder",
        },
    }


def _corpus_facts(paths: list[str]) -> list[dict[str, Any]]:
    definitions = (
        ("parser-positive", "crates/selene-testing/corpus/positive/", ".gql", "GQL files"),
        ("parser-negative", "crates/selene-testing/corpus/negative/", ".gql", "GQL files"),
        ("planner-snapshots", "crates/selene-gql/tests/snapshots/plan_snapshot_corpus__", ".snap", "snapshots"),
        ("executor-snapshots", "crates/selene-gql/tests/snapshots/executor_snapshot_corpus__", ".snap", "snapshots"),
        ("algorithm-snapshots", "crates/selene-algorithms/tests/snapshots/algo_snapshot_corpus__", ".snap", "snapshots"),
        ("mutation-executor-test-files", "crates/selene-gql/tests/exec_pipeline_mutation", ".rs", "Rust test files"),
        ("mutation-plan-test-file", "crates/selene-gql/tests/plan_mutation.rs", ".rs", "Rust test files"),
    )
    return [
        {
            "name": name,
            "path": prefix,
            "count": sum(path.startswith(prefix) and path.endswith(suffix) for path in paths),
            "unit": unit,
        }
        for name, prefix, suffix, unit in definitions
    ]


def _fuzz_facts(paths: list[str]) -> list[dict[str, Any]]:
    targets = (("selene-gql", "parse_gql"), ("selene-gql", "parse_many_gql"), ("selene-gql", "round_trip"),
               ("selene-persist", "decode_manifest"), ("selene-persist", "decode_wal"),
               ("selene-persist", "decode_audit"), ("selene-persist", "decode_snapshot"))
    facts = []
    for crate, name in targets:
        relative = f"crates/{crate}/fuzz/fuzz_targets/{name}.rs"
        if relative not in paths:
            raise BaselineError(f"required fuzz target is absent: {relative}")
        seed_prefix = f"crates/{crate}/fuzz/corpus/{name}/"
        facts.append(
            {
                "crate": crate,
                "name": name,
                "path": relative,
                "tracked_seed_count": sum(path.startswith(seed_prefix) for path in paths),
            }
        )
    return facts


def _persistence_facts(root: pathlib.Path) -> list[dict[str, Any]]:
    identities = (("WAL", "SLDB", "3.0", "file_header.rs"),
                  ("snapshot", "SLSN", "1.5", "snapshot_file_header.rs"),
                  ("MANIFEST", "SLMF", "1", "manifest.rs"), ("audit", "SLAU", "2", "audit.rs"))
    return [{"artifact": artifact, "magic": magic, "version": version,
             "source": _source_file(root, f"crates/selene-persist/src/{source}")}
            for artifact, magic, version, source in identities]


def _disposition(package: str) -> tuple[str, str]:
    return {
        "selene-db-core": ("internalize", "M02-PR05 and capability milestones M04/M10"),
        "selene-db-graph": ("internalize", "M02-PR05 and retrieval milestones M07/M10"),
        "selene-db-persist": ("replace", "M09-PR08"),
        "selene-db-algorithms": ("preserve", "M10-PR01"),
        "selene-db-gql": ("replace", "M05 and M06"),
    }[package]


def cross_file_inventory_errors(
    manifest: dict[str, Any], inventory: dict[str, Any], inventory_sha256: str
) -> list[str]:
    """Check inventory identities and counts against the manifest package facts."""

    errors: list[str] = []
    expected = {(package, crate) for package, crate, _ in PUBLISHED_CRATES}
    public_api = manifest["deterministic"]["public_api"]
    package_facts = [
        package for package in manifest["deterministic"]["packages"] if package["publish"]
    ]

    def index_by_identity(items: list[dict[str, Any]], label: str) -> dict[tuple[str, str], dict[str, Any]]:
        indexed: dict[tuple[str, str], dict[str, Any]] = {}
        for item in items:
            identity = (item["package"], item["crate"])
            if identity in indexed:
                errors.append(f"{label} has duplicate crate identity {identity!r}")
            indexed[identity] = item
        if set(indexed) != expected:
            errors.append(
                f"{label} crate identities differ: expected {sorted(expected)}, got {sorted(indexed)}"
            )
        return indexed

    inventory_crates = index_by_identity(inventory["crates"], "API inventory")
    summaries = index_by_identity(public_api["crates"], "public API summary")
    packages = index_by_identity(package_facts, "published package facts")
    if manifest["archive"]["workspace_version"] != ARCHIVE_VERSION:
        errors.append("archive workspace version differs from the exact historical version")
    if public_api["inventory_path"] != "docs/v2/baseline/api-inventory.json":
        errors.append("public API inventory path is not canonical")
    if public_api["inventory_scope"] != inventory["inventory_scope"]:
        errors.append("public API inventory scope mismatch")
    if public_api["inventory_sha256"] != inventory_sha256:
        errors.append("public API inventory hash mismatch")
    if public_api["published_crate_count"] != len(expected):
        errors.append("public API published crate count mismatch")

    for identity in sorted(expected & set(inventory_crates) & set(summaries) & set(packages)):
        crate = inventory_crates[identity]
        summary = summaries[identity]
        package = packages[identity]
        disposition, owner = _disposition(identity[0])
        if crate["crate_version"] != package["version"] or package["version"] != ARCHIVE_VERSION:
            errors.append(f"{identity!r}: inventory/package version mismatch")
        if (crate["disposition"], crate["owner"]) != (disposition, owner):
            errors.append(f"{identity!r}: inventory disposition or owner mismatch")
        for field, collection in (
            ("path_count", "paths"),
            ("declared_item_count", "declared_items"),
            ("example_count", "examples"),
        ):
            actual = len(crate[collection])
            if summary[field] != actual:
                errors.append(f"{identity!r}: {field} mismatch, expected {actual}, got {summary[field]}")
        for field in ("package", "crate", "disposition", "owner"):
            if summary[field] != crate[field]:
                errors.append(f"{identity!r}: summary {field} differs from inventory")

    aggregates = {
        "path_count": sum(len(crate["paths"]) for crate in inventory_crates.values()),
        "declared_item_count": sum(
            len(crate["declared_items"]) for crate in inventory_crates.values()
        ),
        "example_count": sum(len(crate["examples"]) for crate in inventory_crates.values()),
    }
    for field, actual in aggregates.items():
        if public_api[field] != actual:
            errors.append(f"public API aggregate {field} mismatch, expected {actual}, got {public_api[field]}")
    return errors


def _inventory_document(crates: list[dict[str, Any]]) -> dict[str, Any]:
    enriched = []
    for crate in crates:
        disposition, owner = _disposition(crate["package"])
        enriched.append({**crate, "disposition": disposition, "owner": owner})
    return {
        "schema_version": 1,
        "source_sha": ARCHIVE_SHA,
        "inventory_scope": API_INVENTORY_SCOPE,
        "disposition_meaning": (
            "Semantic capability intent only; no 2.0 source path, signature, reader, alias, or migration is promised."
        ),
        "crates": enriched,
    }


def _host_facts() -> dict[str, Any]:
    def sysctl(name: str) -> str | None:
        completed = subprocess.run(["sysctl", "-n", name], capture_output=True, text=True)
        return completed.stdout.strip() if completed.returncode == 0 else None

    logical = int(sysctl("hw.logicalcpu") or os.cpu_count() or 1)
    physical = int(sysctl("hw.physicalcpu") or logical)
    return {
        "platform": platform.system(),
        "platform_release": platform.release(),
        "os_version": platform.mac_ver()[0] or platform.version(),
        "os_build": sysctl("kern.osversion") or "unknown",
        "architecture": platform.machine(),
        "processor": sysctl("machdep.cpu.brand_string") or platform.processor() or "unknown",
        "physical_cpu_count": physical,
        "logical_cpu_count": logical,
        "memory_bytes": int(sysctl("hw.memsize") or 0) or None,
    }


def _tool_version(argv: list[str]) -> str:
    completed = subprocess.run(argv, capture_output=True, text=True)
    if completed.returncode != 0:
        return f"unavailable (exit {completed.returncode})"
    return "; ".join((completed.stdout or completed.stderr).strip().splitlines())


def _observation_notes(commands: list[dict[str, Any]]) -> list[str]:
    persistence_passed = all(
        command["disposition"] == "passed" for command in commands if command["id"].startswith("fuzz-persist-")
    )
    return [
        "Archive execution used an isolated local clone with a detached checkout and its own .git directory.",
        "Cargo network access was disabled for every canonical command; service and embedding variables were removed from child environments.",
        "git diff --check is intentionally absent from the archive lane and belongs to the current harness worktree gate.",
        "The archive did not pin cargo-about. The current sanctioned cargo-about 0.9.2 reports attribution drift; immutable archive output is retained as failed evidence and is not repaired by M00-PR04.",
        ("All four persistence fuzz targets built and completed their short runs on macOS despite the archive persistence fuzz README saying Linux-only; no Linux run is claimed."
         if persistence_passed else "At least one persistence fuzz command did not pass on macOS; no Linux run is claimed."),
        "No tracked fuzz seed corpus exists for the seven required targets; cargo-fuzz starts from its built-in seed for short runs.",
        "The machine was intentionally busy. Benchmark numbers are non-green observations only; no guard, threshold, optimization claim, or future percentage comparison derives from them. Issue #1137 / M08-PR06 owns stable measurement and guard selection.",
        "Initial absolute benchmark runs have no comparison p-value. Criterion repeat p-values are internal same-tree run-to-run signals and are not product regression evidence.",
        "The required mixed filter matched the intended non-WAL rows and their WAL-suffixed companions; both are disclosed.",
        "A coefficient of variation above 0.25 triggered one sanctioned 100-sample, 10-second-measurement write repeat. Higher fidelity did not stabilize the rows.",
    ]


def _archive_notice(title: str, manifest: dict[str, Any]) -> list[str]:
    return [f"# {title}", "", f"> Historical evidence for source `{manifest['archive']['source_sha']}` only.",
            "> Not a 2.0 compatibility, signature, format-reader, alias, or migration contract.",
            "> Benchmarks ran on an intentionally busy machine. They are non-green observations, not guards, comparisons, thresholds, or stable percentage baselines; issue #1137 / M08-PR06 owns future stable measurement.", ""]


def _table(headers: tuple[str, ...], rows: list[tuple[Any, ...]]) -> list[str]:
    return ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|",
            *("| " + " | ".join(str(value).replace("|", "\\|") for value in row) + " |" for row in rows)]


def _status_counts(commands: list[dict[str, Any]]) -> str:
    counts = {disposition: 0 for disposition in sorted(DISPOSITIONS)}
    for command in commands:
        counts[command["disposition"]] += 1
    return ", ".join(f"{name}: {count}" for name, count in counts.items())


def _render_readme(manifest: dict[str, Any]) -> str:
    archive = manifest["archive"]
    harness = manifest["harness"]
    public_api = manifest["deterministic"]["public_api"]
    commands = manifest["observations"]["commands"]
    lines = _archive_notice("Final 1.x executable baseline", manifest)
    rows = [("Source commit", f"`{archive['source_sha']}`"), ("Source tree", f"`{archive['source_tree_sha']}`"),
            ("Source commit time", f"`{archive['source_commit_time']}`"), ("1.x coordinate", f"`{archive['workspace_version']}`"),
            ("Initial harness base", f"`{harness['initial_base_sha']}`"), ("Initial base tree", f"`{harness['initial_base_tree_sha']}`"),
            ("Capture HEAD", f"`{harness['capture_head_sha']}`"), ("Capture HEAD tree", f"`{harness['capture_head_tree_sha']}`"),
            ("Runner SHA-256", f"`{harness['script']['sha256']}`"), ("Helper SHA-256", f"`{harness['helper']['sha256']}`"),
            ("Archive refs", f"`{archive['archive_refs']}`")]
    lines += ["This package fixes deterministic inventories and one observed run. Raw evidence stays under ignored `target/v2-baseline/`.",
              "", "## Provenance", "", *_table(("Identity", "Value"), rows), "",
              "Harness file hashes are separate from source provenance; no self-referential final harness commit is claimed.", "",
              "## Reports", "", "- [`gates.md`](gates.md): commands, tests, corpora, and fuzz.",
              "- [`public-api.md`](public-api.md): published APIs and semantic dispositions.",
              "- [`formats.md`](formats.md): persistence, packages, procedures, and feature register.",
              "- [`benchmarks.md`](benchmarks.md): absolute Criterion observations.", "",
              f"Inventory: {public_api['path_count']} paths, {public_api['declared_item_count']} local trait/inherent items, "
              f"{public_api['example_count']} doc examples, {public_api['cargo_example_target_count']} Cargo example targets.",
              f"Command dispositions: {_status_counts(commands)}.", "", "## Commands", "", "```bash",
              "scripts/v2-baseline.sh capture", "scripts/v2-baseline.sh install", "scripts/v2-baseline.sh verify", "```", "",
              "Capture uses an isolated detached local clone, disables Cargo network access, removes service variables, and retains redacted logs."]
    return "\n".join(lines) + "\n"


def _render_gates(manifest: dict[str, Any]) -> str:
    deterministic = manifest["deterministic"]
    observations = manifest["observations"]
    lines = _archive_notice("Archival gates and corpora", manifest)
    lines += [f"Captured `{observations['captured_at']}` on `{observations['host']['platform']} "
              f"{observations['host']['platform_release']} {observations['host']['architecture']}`.", "",
              "Each command stands alone; non-passing results remain non-green. Raw redacted logs are ignored evidence.", "",
              "| ID | Lane | Result | Exit | Seconds | Output SHA-256 | Command |", "|---|---|---|---:|---:|---|---|"]
    for command in observations["commands"]:
        exit_code = "—" if command["exit_code"] is None else str(command["exit_code"])
        digest = "—" if command["output_sha256"] is None else f"`{command['output_sha256']}`"
        command_text = command["command"].replace("|", "\\|")
        lines.append(f"| `{command['id']}` | {command['lane']} | **{command['disposition']}** | {exit_code} | {command['duration_seconds']:.3f} | {digest} | `{command_text}` |")
        if command["reason"]:
            lines.append(f"|  |  | Reason |  |  |  | {command['reason'].replace('|', '\\|')} |")
    count = observations["nextest_test_count"]
    lines += ["", "## Test and corpus identities", "", f"Nextest reported **{count if count is not None else 'no parsed'} tests**.", "",
              "| Corpus | Tracked path/prefix | Count | Unit |", "|---|---|---:|---|"]
    for corpus in deterministic["corpora"]:
        lines.append(
            f"| {corpus['name']} | `{corpus['path']}` | {corpus['count']} | {corpus['unit']} |"
        )
    lines += ["", "## Fuzz targets", "", "| Crate | Target | Source | Tracked seeds |", "|---|---|---|---:|"]
    for target in deterministic["fuzz_targets"]:
        lines.append(
            f"| `{target['crate']}` | `{target['name']}` | `{target['path']}` | {target['tracked_seed_count']} |"
        )
    lines += ["", "## Known ignored and slow tests", ""]
    for category in ("ignored", "slow"):
        lines.append(f"### {category.capitalize()}")
        lines.append("")
        for test in deterministic["known_tests"][category]:
            lines.append(f"- `{test['name']}` in `{test['path']}`: {test['reason']}")
        lines.append("")
    lines += ["## Observation notes", ""]
    lines.extend(f"- {note}" for note in observations["notes"])
    return "\n".join(lines) + "\n"


def _render_public_api(manifest: dict[str, Any]) -> str:
    deterministic = manifest["deterministic"]
    api = deterministic["public_api"]
    packages = deterministic["packages"]
    lines = _archive_notice("Public API and examples", manifest)
    lines += ["Dispositions are semantic capability intent, not a 2.0 Rust path or signature promise.", "",
              f"Nightly rustdoc JSON (`includes_private=false`) supplies the inventory. {api['inventory_scope']}", "",
              "| Published package | Crate | Disposition | Owner | Paths | Declared items | Examples |", "|---|---|---|---|---:|---:|---:|"]
    for crate in api["crates"]:
        lines.append(
            f"| `{crate['package']}` | `{crate['crate']}` | `{crate['disposition']}` | {crate['owner']} | {crate['path_count']} | {crate['declared_item_count']} | {crate['example_count']} |"
        )
    support = next(package for package in packages if package["package"] == "selene-db-testing")
    lines += ["", "## Unpublished support", "", f"`{support['package']}` (`{support['crate']}`) has `publish = false`; it owns test, corpus, fixture, embedding-client, and benchmark support.",
              "", "## Generated inventory", "", f"`{api['inventory_path']}` SHA-256 `{api['inventory_sha256']}`. "
              f"It holds {api['path_count']} paths, {api['declared_item_count']} local declared items, and {api['example_count']} doc examples.", "",
              "This generated file is the bounded D-021 exception; this report is its review entry point."]
    return "\n".join(lines) + "\n"


def _render_formats(manifest: dict[str, Any]) -> str:
    deterministic = manifest["deterministic"]
    lines = _archive_notice("Archive formats and registries", manifest)
    lines += ["Archive implementation identities only; no 2.0 reader or reopen test is authorized.", "", "## Persistence", "",
              "| Artifact | Magic | Version | Source | Source SHA-256 |", "|---|---|---|---|---|"]
    for artifact in deterministic["persistence"]:
        source = artifact["source"]
        lines.append(
            f"| {artifact['artifact']} | `{artifact['magic']}` | `{artifact['version']}` | `{source['path']}` | `{source['sha256']}` |"
        )
    lines += ["", "## Packages", "", "| Package | Crate | Version | Published | Manifest SHA-256 |", "|---|---|---|---:|---|"]
    for package in deterministic["packages"]:
        lines.append(
            f"| `{package['package']}` | `{package['crate']}` | `{package['version']}` | {'yes' if package['publish'] else 'no'} | `{package['manifest_sha256']}` |"
        )
    lines.extend(["", "## Procedure registries", ""])
    for label in ("builtin", "algorithm"):
        group = deterministic["procedures"][label]
        lines += [f"### {label.capitalize()} procedures ({group['count']})", "",
                  f"Source `{group['source']['path']}` (`{group['source']['sha256']}`).", "",
                  ", ".join(f"`{name}`" for name in group["names"]), ""]
    feature = deterministic["feature_register"]
    lines += ["## Feature register", "", f"`{feature['source']['path']}` (`{feature['source']['sha256']}`): {feature['referenced_count']} referenced, "
              f"{feature['supported_count']} supported, {feature['not_supported_rationale_count']} non-support rationales.", "",
              f"`{feature['generated_doc']['path']}` is a **{feature['generated_doc']['status']}** (`{feature['generated_doc']['sha256']}`), not generated authority."]
    return "\n".join(lines) + "\n"


def _render_benchmarks(manifest: dict[str, Any]) -> str:
    observations = manifest["observations"]
    host = observations["host"]
    lines = _archive_notice("Benchmark observations", manifest)
    lines += ["Absolute Criterion observations only; no comparison, threshold, or regression verdict.", "", "## Environment", "",
              f"Captured `{observations['captured_at']}`; `{host['platform']} {host['os_version']} ({host['os_build']})`, kernel `{host['platform_release']}`, `{host['architecture']}`; "
              f"processor `{host['processor']}`; {host['physical_cpu_count']} physical/{host['logical_cpu_count']} logical cores; `{host['memory_bytes']}` memory bytes.",
              *(f"- `{name}`: `{value}`" for name, value in observations["tools"].items()), "",
              "Initial absolute runs have no comparison p-value. Repeat p-values are Criterion's internal same-tree run-to-run signals, not product regression evidence. CV is standard deviation divided by mean.", ""]
    commands = {command["id"]: command for command in observations["commands"]}
    for benchmark in observations["benchmarks"]:
        command = commands[benchmark["command_id"]]
        lines += [f"## `{benchmark['command_id']}`", "", "```bash", command["command"], "```", "",
                   f"**{command['disposition']}** in `{command['duration_seconds']:.3f}s`; {benchmark['result_count']} results; `{benchmark['allocator']}`. "
                  f"Max CV `{maximum_cv(benchmark['results']):.3f}`; {sum((result['coefficient_of_variation'] or 0) > 0.25 for result in benchmark['results'])} rows exceed `0.25`.", "",
                  "| Criterion ID | Samples | Median ns (95% CI) | Mean ns (95% CI) | Std. dev. ns | CV | Outliers | Repeat p |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
        for result in benchmark["results"]:
            median = result["median"]
            mean = result["mean"]
            std_dev = result["std_dev"]
            median_ci = median["confidence_interval"]
            mean_ci = mean["confidence_interval"]
            coefficient = result["coefficient_of_variation"]
            cv = "—" if coefficient is None else f"{coefficient:.6f}"
            p_value = "—" if result["p_value"] is None else f"{result['p_value']:.6g}"
            samples = "—" if result["sample_count"] is None else str(result["sample_count"])
            lines.append(
                f"| `{result['id']}` | {samples} | {median['point_estimate']:.3f} ({median_ci['lower_bound']:.3f}–{median_ci['upper_bound']:.3f}) | {mean['point_estimate']:.3f} ({mean_ci['lower_bound']:.3f}–{mean_ci['upper_bound']:.3f}) | {std_dev['point_estimate']:.3f} | {cv} | {result['outlier_count']} ({result['outlier_percent']:.2f}%) | {p_value} |"
            )
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def render_reports(manifest: dict[str, Any], output_dir: pathlib.Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    rendered = {
        "README.md": _render_readme(manifest),
        "gates.md": _render_gates(manifest),
        "public-api.md": _render_public_api(manifest),
        "formats.md": _render_formats(manifest),
        "benchmarks.md": _render_benchmarks(manifest),
    }
    for name, content in rendered.items():
        (output_dir / name).write_text(content)


def _scrubbed_environment() -> tuple[dict[str, str], set[str]]:
    environment = dict(os.environ)
    secrets = {
        value
        for key, value in environment.items()
        if value and any(key.startswith(prefix) for prefix in SERVICE_ENV_PREFIXES)
    }
    for key in list(environment):
        if any(key.startswith(prefix) for prefix in SERVICE_ENV_PREFIXES):
            del environment[key]
    environment.update(
        {
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TERM_COLOR": "never",
            "NO_COLOR": "1",
        }
    )
    return environment, secrets


def _run_required(
    commands: list[dict[str, Any]],
    *,
    command_id: str,
    argv: list[str],
    cwd: pathlib.Path,
    logs_dir: pathlib.Path,
    environment: dict[str, str],
    secrets: set[str],
    lane: str,
    timeout: int | None = None,
) -> dict[str, Any]:
    result = run_command(
        command_id=command_id,
        argv=argv,
        cwd=cwd,
        logs_dir=logs_dir,
        environment=environment,
        secret_values=secrets,
        lane=lane,
        timeout=timeout,
    )
    commands.append(result)
    return result


def _archive_gate_specs() -> list[tuple[str, list[str]]]:
    commands = [
        ("fmt", "cargo fmt --all --check"), ("check", "cargo check --workspace --locked"),
        ("clippy", "cargo clippy --workspace --all-targets --locked -- -D warnings"),
        ("nextest", "cargo nextest run --workspace --locked --all-features --profile default"),
        ("doctest", "cargo test --workspace --locked --all-features --doc"),
        ("doc", "cargo doc --workspace --no-deps --locked"),
        ("deny-bans", "cargo deny check --exclude-dev bans"),
        ("deny-licenses-sources", "cargo deny check licenses sources"),
        ("audit", "cargo audit -d /private/tmp/selene-advisory-db"),
        ("file-size", "bash .github/scripts/check-file-size.sh"), ("no-secrets", "bash .github/scripts/check-no-secrets.sh"),
        ("thirdparty", "bash .github/scripts/check-thirdparty-current.sh"), ("rowid", "bash .github/scripts/check-no-rowid-arith.sh"),
        ("feature-errors", "bash .github/scripts/check-no-version-locked-feature-error.sh"),
        ("bench-invocation", "bash .github/scripts/check-bench-invocation.sh"),
        ("bench-doc", "bash .github/scripts/check-benchmarks-doc.sh ."),
        ("doc-constants", "bash .github/scripts/check-doc-constants.sh"),
        ("mimalloc", "bash .github/scripts/check-mimalloc-dev-dep.sh"),
    ]
    return [(f"archive-{name}", shlex.split(command)) for name, command in commands]


def _benchmark_specs() -> list[tuple[str, list[str]]]:
    commands = [
        ("smoke", "scripts/run-benches.sh --smoke"),
        ("single-graph-read", "scripts/run-benches.sh --profile full --bench single_graph --filter graph_(node_fetch|label_index_lookup|typed_index_point)"),
        ("write-lifecycle", "scripts/run-benches.sh --profile full --bench write_txn_lifecycle --filter write_txn_lifecycle/(graph_clone|create_only|delete_only)"),
        ("mixed-r60w40", "scripts/run-benches.sh --profile full --bench graph_mixed_workload --filter graph_mixed_workload/point_read_update_r60w40"),
    ]
    return [(f"benchmark-{name}", shlex.split(command)) for name, command in commands]


def _nextest_count(log_path: pathlib.Path) -> int | None:
    if not log_path.exists():
        return None
    text = log_path.read_text(errors="replace")
    matches = re.findall(r"(\d+)\s+tests?\s+run", text)
    return int(matches[-1]) if matches else None


def _example_target_count(root: pathlib.Path, paths: list[str]) -> int:
    source_targets = {
        path.split("/examples/", 1)[0] + "/examples/" + path.split("/examples/", 1)[1].split("/", 1)[0]
        for path in paths
        if "/examples/" in path and path.endswith(".rs")
    }
    declared = 0
    for _, _, relative in PUBLISHED_CRATES:
        manifest = tomllib.loads((root / relative / "Cargo.toml").read_text())
        declared += len(manifest.get("example", []))
    return len(source_targets) + declared


def capture(
    root: pathlib.Path,
    requested_output: pathlib.Path | None = None,
    _publish_hook: Callable[[str], None] | None = None,
) -> pathlib.Path:
    root = root.resolve()
    source_sha = resolve_archive_commit(root, ARCHIVE_SHA)
    head = git_output(root, "rev-parse", "HEAD")
    if subprocess.run(["git", "merge-base", "--is-ancestor", EXPECTED_BASE_SHA, head], cwd=root).returncode != 0:
        raise BaselineError(f"harness HEAD {head} does not contain required base {EXPECTED_BASE_SHA}")
    validate_capture_worktree(root)
    output = controlled_output_dir(root, requested_output)
    staged = _staged_sibling(output, "capture")
    logs_dir = staged / "logs"
    environment, secrets = _scrubbed_environment()
    commands: list[dict[str, Any]] = []
    benchmarks: list[dict[str, Any]] = []
    clone = pathlib.Path(tempfile.mkdtemp(prefix="selene-v2-baseline-archive-")) / "repository"
    validate_clone_location(root, clone)
    def run(command_id: str, argv: list[str], cwd: pathlib.Path = clone, lane: str = "archive",
            timeout: int | None = None) -> dict[str, Any]:
        return _run_required(commands, command_id=command_id, argv=argv, cwd=cwd, logs_dir=logs_dir,
                             environment=environment, secrets=secrets, lane=lane, timeout=timeout)

    previous_term = signal.getsignal(signal.SIGTERM)
    signal.signal(signal.SIGTERM, lambda _signum, _frame: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        clone_result = run("inventory-clone",
                           ["git", "clone", "--shared", "--no-checkout", "--local", str(root), str(clone)],
                           root, "inventory")
        if clone_result["disposition"] != "passed":
            raise BaselineError("isolated local archive clone failed")
        checkout_result = run("inventory-checkout", ["git", "checkout", "--detach", source_sha], lane="inventory")
        if checkout_result["disposition"] != "passed" or git_output(clone, "rev-parse", "HEAD") != source_sha:
            raise BaselineError("isolated checkout did not resolve to the exact archive source")

        paths = _tracked_paths(clone)
        packages, workspace = _package_facts(clone)
        rustdoc_crates: list[dict[str, Any]] = []
        rustdoc_format_version: int | None = None
        for package, crate_name, _relative in PUBLISHED_CRATES:
            result = run(f"inventory-rustdoc-{crate_name.removeprefix('selene_')}",
                         ["cargo", "+nightly-2026-08-15", "rustdoc", "-Z", "unstable-options", "--locked",
                          "-p", package, "--lib", "--output-format", "json"], lane="inventory")
            rustdoc_path = clone / "target" / "doc" / f"{crate_name}.json"
            if result["disposition"] != "passed" or not rustdoc_path.exists():
                raise BaselineError(f"complete public API extraction failed for {package}")
            rustdoc = json.loads(rustdoc_path.read_text())
            crate_inventory = inventory_rustdoc_crate(package, crate_name, rustdoc)
            rustdoc_crates.append(crate_inventory)
            observed_format = crate_inventory["rustdoc_format_version"]
            if rustdoc_format_version is None:
                rustdoc_format_version = observed_format
            elif rustdoc_format_version != observed_format:
                raise BaselineError("rustdoc JSON format changed during inventory collection")
        inventory = _inventory_document(rustdoc_crates)
        inventory_errors = api_inventory_errors(inventory)
        if inventory_errors:
            raise BaselineError("; ".join(inventory_errors))
        inventory_path = staged / "api-inventory.json"
        inventory_path.write_text(canonical_json(inventory))

        for command_id, argv in _archive_gate_specs():
            if command_id == "archive-audit" and not pathlib.Path("/private/tmp/selene-advisory-db").is_dir():
                commands.append(
                    unavailable_command(
                        command_id,
                        _command_text(argv),
                        clone,
                        "the required local advisory database is absent",
                    )
                )
                continue
            result = run(command_id, argv)
            if command_id == "archive-thirdparty" and result["disposition"] == "failed":
                result["reason"] = (
                    "The archive did not pin cargo-about; sanctioned cargo-about 0.9.2 found immutable "
                    "THIRDPARTY.md drift. M00-PR04 retains the failure without archive repair."
                )

        criterion_dir = clone / "target" / "criterion"
        benchmark_results: dict[str, list[dict[str, Any]]] = {}
        for command_id, argv in _benchmark_specs():
            before = _criterion_snapshot(criterion_dir)
            result = run(command_id, argv, lane="benchmark")
            try:
                criterion_results = collect_criterion_results(criterion_dir, before)
            except BaselineError as error:
                result["disposition"] = "failed"
                result["reason"] = str(error)
                if result["exit_code"] == 0:
                    result["exit_code"] = 1
                criterion_results = []
            if criterion_results:
                annotate_criterion_log(criterion_results, logs_dir / f"{command_id}.log")
                benchmark_results[command_id] = criterion_results
                benchmarks.append(
                    {
                        "command_id": command_id,
                        "allocator": "mimalloc",
                        "result_count": len(criterion_results),
                        "results": criterion_results,
                    }
                )

        write_results = benchmark_results.get("benchmark-write-lifecycle", [])
        if maximum_cv(write_results) > 0.25:
            write_argv = dict(_benchmark_specs())["benchmark-write-lifecycle"]
            command_id = "benchmark-write-lifecycle-repeat"
            argv = [*write_argv, "--sample-size", "100", "--measurement-time", "10"]
            before = _criterion_snapshot(criterion_dir)
            result = run(command_id, argv, lane="benchmark")
            try:
                criterion_results = collect_criterion_results(criterion_dir, before)
            except BaselineError as error:
                result.update(disposition="failed", reason=str(error), exit_code=result["exit_code"] or 1)
            else:
                annotate_criterion_log(criterion_results, logs_dir / f"{command_id}.log")
                benchmarks.append({"command_id": command_id, "allocator": "mimalloc", "result_count": len(criterion_results),
                                   "results": criterion_results})

        for target in _fuzz_facts(paths):
            fuzz_root = clone / "crates" / target["crate"] / "fuzz"
            short_crate = target["crate"].removeprefix("selene-")
            build = run(f"fuzz-{short_crate}-{target['name']}-build",
                        ["cargo", "+nightly-2026-08-15", "fuzz", "build", target["name"]], fuzz_root, "fuzz")
            if build["disposition"] == "passed":
                run(f"fuzz-{short_crate}-{target['name']}-run",
                    ["cargo", "+nightly-2026-08-15", "fuzz", "run", target["name"], "--", "-max_total_time=10"],
                    fuzz_root, "fuzz", 120)
            else:
                argv = ["cargo", "+nightly-2026-08-15", "fuzz", "run", target["name"], "--", "-max_total_time=10"]
                commands.append(unavailable_command(f"fuzz-{short_crate}-{target['name']}-run", _command_text(argv),
                                                    fuzz_root, "the target's required fuzz build failed", "fuzz", "skipped"))

        public_summaries = []
        for crate in inventory["crates"]:
            public_summaries.append(
                {
                    "package": crate["package"],
                    "crate": crate["crate"],
                    "disposition": crate["disposition"],
                    "owner": crate["owner"],
                    "path_count": len(crate["paths"]),
                    "declared_item_count": len(crate["declared_items"]),
                    "example_count": len(crate["examples"]),
                }
            )
        archive_tree = git_output(clone, "show", "-s", "--format=%T", source_sha)
        manifest = {
            "$schema": "./manifest.schema.json",
            "schema_version": SCHEMA_VERSION,
            "archive": {
                "repository": "jscott3201/selene-db",
                "source_sha": source_sha,
                "source_tree_sha": archive_tree,
                "source_commit_time": git_output(clone, "show", "-s", "--format=%cI", source_sha),
                "source_title": git_output(clone, "show", "-s", "--format=%s", source_sha),
                "workspace_version": workspace["version"],
                "edition": workspace["edition"],
                "rust_version": workspace["rust_version"],
                "archive_refs": "pending_owner_only",
            },
            "harness": {
                "initial_base_sha": EXPECTED_BASE_SHA,
                "initial_base_tree_sha": git_output(root, "show", "-s", "--format=%T", EXPECTED_BASE_SHA),
                "capture_head_sha": head,
                "capture_head_tree_sha": git_output(root, "show", "-s", "--format=%T", head),
                "script": _source_file(root, "scripts/v2-baseline.sh"),
                "helper": _source_file(root, "scripts/v2_baseline.py"),
                "rustdoc_toolchain": "nightly-2026-08-15",
                "rustdoc_json_format_version": rustdoc_format_version,
            },
            "deterministic": {
                "packages": packages,
                "public_api": {
                    "inventory_path": "docs/v2/baseline/api-inventory.json",
                    "inventory_sha256": sha256_file(inventory_path),
                    "inventory_scope": API_INVENTORY_SCOPE,
                    "published_crate_count": len(inventory["crates"]),
                    "path_count": sum(len(crate["paths"]) for crate in inventory["crates"]),
                    "declared_item_count": sum(
                        len(crate["declared_items"]) for crate in inventory["crates"]
                    ),
                    "example_count": sum(len(crate["examples"]) for crate in inventory["crates"]),
                    "cargo_example_target_count": _example_target_count(clone, paths),
                    "crates": public_summaries,
                },
                "persistence": _persistence_facts(clone),
                "procedures": {
                    "builtin": _procedure_group(
                        clone,
                        "crates/selene-gql/src/runtime/builtins/catalog/specs.rs",
                        "BUILTIN_SPECS",
                    ),
                    "algorithm": _procedure_group(
                        clone,
                        "crates/selene-gql/src/runtime/native_algorithms/mod.rs",
                        "ALGO_SPECS",
                    ),
                },
                "feature_register": _feature_facts(clone),
                "corpora": _corpus_facts(paths),
                "fuzz_targets": _fuzz_facts(paths),
                "known_tests": {
                    "ignored": [
                        {
                            "name": "helper_process",
                            "path": "crates/selene-persist/src/manifest_lock/tests.rs",
                            "reason": "ignored helper invoked by separate_process_contends_on_the_same_lock_file",
                        },
                        {
                            "name": "local_spec_corpus_snapshots_match",
                            "path": "crates/selene-gql/tests/plan_snapshot_corpus.rs",
                            "reason": "local-only specification mirror; run manually with --ignored",
                        },
                    ],
                    "slow": [
                        {
                            "name": "parsing_hostile_fold_then_dropping_is_safe",
                            "path": "crates/selene-gql/tests/parser_expr_depth.rs",
                            "reason": "nextest slow-timeout override: 48 seconds, terminate after five periods",
                        }
                    ],
                },
            },
            "observations": {
                "captured_at": utc_now(),
                "host": _host_facts(),
                "tools": {
                    "stable_rustc": _tool_version(["rustc", "--version", "--verbose"]),
                    "stable_cargo": _tool_version(["cargo", "--version", "--verbose"]),
                    "nightly_rustc": _tool_version(
                        ["rustc", "+nightly-2026-08-15", "--version", "--verbose"]
                    ),
                    "nightly_cargo": _tool_version(
                        ["cargo", "+nightly-2026-08-15", "--version", "--verbose"]
                    ),
                    "cargo_nextest": _tool_version(["cargo", "nextest", "--version"]),
                    "cargo_deny": _tool_version(["cargo", "deny", "--version"]),
                    "cargo_audit": _tool_version(["cargo", "audit", "--version"]),
                    "cargo_about": _tool_version(["cargo", "about", "--version"]),
                    "cargo_fuzz": _tool_version(["cargo", "fuzz", "--version"]),
                    "criterion": f"criterion {workspace['criterion']} (archive Cargo.toml)",
                    "allocator": f"mimalloc {workspace['mimalloc']} (archive benchmark default)",
                },
                "commands": commands,
                "benchmarks": benchmarks,
                "nextest_test_count": _nextest_count(logs_dir / "archive-nextest.log"),
                "notes": _observation_notes(commands),
            },
            "reports": [],
        }
        schema_path = root / BASELINE_RELATIVE / "manifest.schema.json"
        if sha256_file(schema_path) != EXPECTED_SCHEMA_SHA256:
            raise BaselineError("tracked manifest schema hash differs from the harness contract")
        schema = json.loads(schema_path.read_text())
        errors = closed_schema_errors(schema) + schema_errors(manifest, schema)
        if errors:
            raise BaselineError("manifest construction failed: " + "; ".join(errors))
        reports_dir = staged / "reports"
        render_reports(manifest, reports_dir)
        manifest["reports"] = [
            {
                "path": f"docs/v2/baseline/{path.name}",
                "sha256": sha256_file(reports_dir / path.name),
                "bytes": (reports_dir / path.name).stat().st_size,
            }
            for path in REPORT_PATHS
        ]
        final_errors = schema_errors(manifest, schema)
        if final_errors:
            raise BaselineError("final manifest failed schema: " + "; ".join(final_errors))
        (staged / "manifest.json").write_text(canonical_json(manifest))
        (staged / "manifest.schema.json").write_text(canonical_json(schema))
        candidate_errors = evidence_package_errors(root, staged)
        if candidate_errors:
            raise BaselineError("captured package is invalid: " + "; ".join(candidate_errors))
        _publish_directory(
            staged,
            output,
            lambda: evidence_package_errors(root, output),
            _publish_hook,
        )
        return output
    finally:
        signal.signal(signal.SIGTERM, previous_term)
        shutil.rmtree(clone.parent, ignore_errors=True)
        if staged.exists():
            shutil.rmtree(staged, ignore_errors=True)


def _package_errors(
    root: pathlib.Path, package_dir: pathlib.Path, reports_dir: pathlib.Path
) -> list[str]:
    errors: list[str] = []
    schema_path = package_dir / "manifest.schema.json"
    manifest_path = package_dir / "manifest.json"
    inventory_path = package_dir / "api-inventory.json"
    for path in (schema_path, manifest_path, inventory_path):
        if not path.is_file():
            errors.append(f"required package file is absent: {path}")
    if errors:
        return errors
    try:
        schema = json.loads(schema_path.read_text())
        manifest = json.loads(manifest_path.read_text())
        inventory = json.loads(inventory_path.read_text())
    except (json.JSONDecodeError, OSError) as error:
        return [f"baseline JSON could not be read: {error}"]

    if sha256_file(schema_path) != EXPECTED_SCHEMA_SHA256:
        errors.append("manifest schema differs from the helper's closed schema")
    errors.extend(closed_schema_errors(schema))
    errors.extend(schema_errors(manifest, schema))
    errors.extend(api_inventory_errors(inventory))
    if errors:
        return errors

    inventory_sha256 = sha256_file(inventory_path)
    errors.extend(cross_file_inventory_errors(manifest, inventory, inventory_sha256))
    for key in ("script", "helper"):
        identity = manifest["harness"][key]
        path = root / identity["path"]
        if not path.is_file():
            errors.append(f"harness {key} is absent: {identity['path']}")
        elif sha256_file(path) != identity["sha256"]:
            errors.append(f"harness {key} hash mismatch")

    expected_reports = {f"docs/v2/baseline/{path.name}" for path in REPORT_PATHS}
    observed_report_paths = [report["path"] for report in manifest["reports"]]
    if len(observed_report_paths) != len(expected_reports) or set(observed_report_paths) != expected_reports:
        errors.append("manifest report set differs from the five required reports")
    for report in manifest["reports"]:
        if report["path"] not in expected_reports:
            continue
        path = reports_dir / pathlib.Path(report["path"]).name
        if not path.is_file():
            errors.append(f"required report is absent: {report['path']}")
            continue
        if sha256_file(path) != report["sha256"] or path.stat().st_size != report["bytes"]:
            errors.append(f"report hash mismatch: {report['path']}")

    with tempfile.TemporaryDirectory(prefix="selene-baseline-validate-") as raw:
        rendered = pathlib.Path(raw)
        render_reports(manifest, rendered)
        for report in REPORT_PATHS:
            tracked = reports_dir / report.name
            candidate = rendered / report.name
            if tracked.is_file() and tracked.read_bytes() != candidate.read_bytes():
                errors.append(f"deterministic report render mismatch: {report.name}")
    return errors


def evidence_package_errors(root: pathlib.Path, evidence_dir: pathlib.Path) -> list[str]:
    return _package_errors(root.resolve(), evidence_dir, evidence_dir / "reports")


def baseline_package_errors(root: pathlib.Path, baseline_dir: pathlib.Path) -> list[str]:
    if not baseline_dir.is_dir():
        return [f"required baseline directory is absent: {baseline_dir}"]
    errors = _package_errors(root.resolve(), baseline_dir, baseline_dir)
    allowed = {
        "README.md",
        "gates.md",
        "public-api.md",
        "formats.md",
        "benchmarks.md",
        "manifest.json",
        "manifest.schema.json",
        "api-inventory.json",
    }
    extras = sorted(path.name for path in baseline_dir.iterdir() if path.is_file() and path.name not in allowed)
    if extras:
        errors.append(f"unexpected baseline files: {', '.join(extras)}")
    return errors


def validate_tracked_baseline(root: pathlib.Path) -> list[str]:
    root = root.resolve()
    return baseline_package_errors(root, root / BASELINE_RELATIVE)


def install(
    root: pathlib.Path,
    evidence: pathlib.Path | None = None,
    _publish_hook: Callable[[str], None] | None = None,
) -> None:
    root = root.resolve()
    evidence_dir = evidence or controlled_output_dir(root, None)
    errors = evidence_package_errors(root, evidence_dir)
    if errors:
        raise BaselineError("captured evidence is invalid: " + "; ".join(errors))
    manifest = json.loads((evidence_dir / "manifest.json").read_text())
    baseline_dir = root / BASELINE_RELATIVE
    staged = _staged_sibling(baseline_dir, "install")
    try:
        shutil.copyfile(evidence_dir / "manifest.schema.json", staged / "manifest.schema.json")
        shutil.copyfile(evidence_dir / "api-inventory.json", staged / "api-inventory.json")
        render_reports(manifest, staged)
        (staged / "manifest.json").write_text(canonical_json(manifest))
        candidate_errors = baseline_package_errors(root, staged)
        if candidate_errors:
            raise BaselineError("installed candidate is invalid: " + "; ".join(candidate_errors))
        _publish_directory(
            staged,
            baseline_dir,
            lambda: baseline_package_errors(root, baseline_dir),
            _publish_hook,
        )
    finally:
        if staged.exists():
            shutil.rmtree(staged, ignore_errors=True)


def copy_baseline_package(source_root: pathlib.Path, destination_root: pathlib.Path) -> None:
    for relative in ("scripts/v2-baseline.sh", "scripts/v2_baseline.py"):
        destination = destination_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_root / relative, destination)
    source = source_root / BASELINE_RELATIVE
    destination = destination_root / BASELINE_RELATIVE
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("capture", "install", "verify", "render"))
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--evidence", type=pathlib.Path)
    parser.add_argument("--manifest", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    return parser


def main() -> int:
    arguments = _parser().parse_args()
    try:
        if arguments.action == "capture":
            output = capture(arguments.root, arguments.evidence)
            print(f"baseline evidence captured at {output}")
        elif arguments.action == "install":
            install(arguments.root, arguments.evidence)
            print("baseline evidence installed")
        elif arguments.action == "verify":
            errors = validate_tracked_baseline(arguments.root)
            if errors:
                for error in errors:
                    print(f"baseline error: {error}", file=sys.stderr)
                return 1
            print("baseline manifest, inventory, and reports verified")
        else:
            if arguments.manifest is None or arguments.output is None:
                raise BaselineError("render requires --manifest and --output")
            render_reports(json.loads(arguments.manifest.read_text()), arguments.output)
    except (BaselineError, OSError, subprocess.CalledProcessError) as error:
        print(f"baseline error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
