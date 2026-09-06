#!/usr/bin/env python3
"""Validate the tracked Selene DB 2.0 program contract."""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import re
import subprocess
import sys
from collections.abc import Iterable
from typing import Any
from urllib.parse import unquote, urlsplit

PLAN = pathlib.Path("docs/v2/roadmap/plan.json")
SCHEMA = pathlib.Path("docs/v2/roadmap/plan.schema.json")
DECISIONS = pathlib.Path("docs/v2/decisions/finalized.md")
ISSUES = pathlib.Path("docs/v2/issue-ownership.md")
PULL_REQUEST_TEMPLATE = pathlib.Path(".github/pull_request_template.md")
VALIDATION_WORKFLOWS = (
    pathlib.Path(".github/workflows/ci.yml"),
    pathlib.Path(".github/workflows/release.yml"),
)
BASELINE_HELPER = pathlib.Path("scripts/v2_baseline.py")
EXACT_REVISION_EXPRESSION = "${{ github.event.pull_request.head.sha || github.sha }}"
PROVENANCE_COMMAND = 'run: test "$(git rev-parse HEAD)" = "$EXPECTED_REVISION"'
EXPECTED_ISSUES = {1088, 1092, 1093, 1094, 1097, 1128, 1137}
LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+[^)]*)?\)")
ANCHOR_RE = re.compile(r'<a\s+id=["\']([^"\']+)["\']\s*></a>', re.IGNORECASE)
LOCAL_DIRECTORY_RE = re.compile(r"(?<![A-Za-z0-9/])(_[A-Za-z0-9][A-Za-z0-9_-]*)/")

EXPECTED_LEGACY_WORK_ITEMS = frozenset(
    [
        *(f"M00-PR0{i}" for i in range(1, 5)),
        *(f"M01-PR0{i}" for i in range(1, 7)),
        *(f"M02-PR0{i}" for i in range(1, 6)),
        *(f"M03-PR0{i}" for i in range(1, 6)),
        *(f"M04-PR0{i}" for i in range(1, 6)),
        *(f"M05-PR0{i}" for i in range(1, 7)),
        *(f"M06-PR0{i}" for i in range(1, 8)),
        *(f"M07-PR0{i}" for i in range(1, 7)),
        *(f"M08-PR0{i}" for i in range(1, 7)),
        *(f"M09-PR0{i}" for i in range(1, 9)),
        *(f"M10-PR0{i}" for i in range(1, 8)),
    ]
)

RETAINED_COMPLETED_ITEMS = frozenset(
    [
        *(f"M00-PR0{i}" for i in range(1, 5)),
        *(f"M01-PR0{i}" for i in range(1, 7)),
        *(f"M02-PR0{i}" for i in range(1, 6)),
        *(f"M03-PR0{i}" for i in range(1, 6)),
        "M04-PR01",
    ]
)


class Check:
    def __init__(self, root: pathlib.Path) -> None:
        self.root = root.resolve()
        self.errors: list[str] = []

    def fail(self, message: str) -> None:
        self.errors.append(message)

    def read_text(self, relative: pathlib.Path) -> str:
        try:
            return (self.root / relative).read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            self.fail(f"{relative}: cannot read: {error}")
            return ""

    def read_json(self, relative: pathlib.Path) -> Any:
        text = self.read_text(relative)
        try:
            return json.loads(text)
        except json.JSONDecodeError as error:
            self.fail(f"{relative}:{error.lineno}:{error.colno}: invalid JSON: {error.msg}")
            return None


def json_type_matches(value: Any, expected: str | list[str]) -> bool:
    if isinstance(expected, list):
        return any(json_type_matches(value, item) for item in expected)
    return {
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "boolean": isinstance(value, bool),
        "null": value is None,
    }.get(expected, False)


def resolve_ref(schema_root: dict[str, Any], reference: str) -> dict[str, Any] | None:
    if not reference.startswith("#/"):
        return None
    value: Any = schema_root
    for part in reference[2:].split("/"):
        if not isinstance(value, dict) or part not in value:
            return None
        value = value[part]
    return value if isinstance(value, dict) else None


def validate_schema_value(
    check: Check,
    value: Any,
    rule: dict[str, Any],
    schema_root: dict[str, Any],
    location: str,
) -> None:
    if "$ref" in rule:
        resolved = resolve_ref(schema_root, rule["$ref"])
        if resolved is None:
            check.fail(f"{location}: unresolved schema reference {rule['$ref']!r}")
            return
        validate_schema_value(check, value, resolved, schema_root, location)
        return
    if "const" in rule and value != rule["const"]:
        check.fail(f"{location}: expected constant {rule['const']!r}")
    if "enum" in rule and value not in rule["enum"]:
        check.fail(f"{location}: value {value!r} is outside the closed enum")
    expected = rule.get("type")
    if expected and not json_type_matches(value, expected):
        check.fail(f"{location}: expected {expected}, got {type(value).__name__}")
        return
    if isinstance(value, dict):
        required = rule.get("required", [])
        for key in required:
            if key not in value:
                check.fail(f"{location}: missing required property {key!r}")
        properties = rule.get("properties", {})
        if rule.get("additionalProperties") is False:
            for key in value.keys() - properties.keys():
                check.fail(f"{location}: unexpected property {key!r}")
        for key, child in value.items():
            if key in properties:
                validate_schema_value(check, child, properties[key], schema_root, f"{location}.{key}")
    elif isinstance(value, list):
        if "minItems" in rule and len(value) < rule["minItems"]:
            check.fail(f"{location}: expected at least {rule['minItems']} items")
        if "maxItems" in rule and len(value) > rule["maxItems"]:
            check.fail(f"{location}: expected at most {rule['maxItems']} items")
        if rule.get("uniqueItems"):
            encoded = [json.dumps(item, sort_keys=True) for item in value]
            if len(encoded) != len(set(encoded)):
                check.fail(f"{location}: duplicate array item")
        item_rule = rule.get("items")
        if isinstance(item_rule, dict):
            for index, child in enumerate(value):
                validate_schema_value(check, child, item_rule, schema_root, f"{location}[{index}]")
    elif isinstance(value, str):
        if "minLength" in rule and len(value) < rule["minLength"]:
            check.fail(f"{location}: string is shorter than {rule['minLength']}")
        if "pattern" in rule and re.fullmatch(rule["pattern"], value) is None:
            check.fail(f"{location}: value {value!r} does not match {rule['pattern']!r}")
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in rule and value < rule["minimum"]:
            check.fail(f"{location}: value is below {rule['minimum']}")
        if "maximum" in rule and value > rule["maximum"]:
            check.fail(f"{location}: value is above {rule['maximum']}")


def check_closed_schema(check: Check, schema: Any) -> None:
    if not isinstance(schema, dict):
        check.fail(f"{SCHEMA}: schema root must be an object")
        return
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        check.fail(f"{SCHEMA}: expected JSON Schema draft 2020-12")

    def walk(rule: Any, location: str) -> None:
        if isinstance(rule, dict):
            if rule.get("type") == "object" and rule.get("additionalProperties") is not False:
                check.fail(f"{SCHEMA}:{location}: object schema is not closed")
            for key, child in rule.items():
                walk(child, f"{location}/{key}")
        elif isinstance(rule, list):
            for index, child in enumerate(rule):
                walk(child, f"{location}/{index}")

    walk(schema, "#")


def duplicates(values: Iterable[Any]) -> set[Any]:
    seen: set[Any] = set()
    repeated: set[Any] = set()
    for value in values:
        if value in seen:
            repeated.add(value)
        seen.add(value)
    return repeated


def check_dependency_cycles(check: Check, dependencies: dict[str, list[str]]) -> None:
    state = {identity: 0 for identity in dependencies}
    stack: list[str] = []

    def visit(identity: str) -> bool:
        state[identity] = 1
        stack.append(identity)
        for dependency in sorted(dependencies.get(identity, [])):
            if dependency not in dependencies:
                continue
            if state[dependency] == 1:
                cycle_start = stack.index(dependency)
                cycle = stack[cycle_start:] + [dependency]
                check.fail(f"dependency cycle: {' -> '.join(cycle)}")
                return True
            if state[dependency] == 0 and visit(dependency):
                return True
        stack.pop()
        state[identity] = 2
        return False

    for identity in sorted(dependencies):
        if state[identity] == 0 and visit(identity):
            return


def anchors(text: str) -> set[str]:
    return set(ANCHOR_RE.findall(text))


def anchor_section(text: str, identity: str) -> str | None:
    matches = list(ANCHOR_RE.finditer(text))
    matching_indexes = [index for index, match in enumerate(matches) if match.group(1) == identity]
    if len(matching_indexes) != 1:
        return None
    index = matching_indexes[0]
    start = matches[index].end()
    end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
    return text[start:end]


def check_plan_semantics(check: Check, plan: dict[str, Any]) -> None:
    milestones = plan.get("milestones", [])
    prs = plan.get("pull_requests", [])
    issues = plan.get("issues", [])
    legacy_items = plan.get("legacy_work_items", [])

    # Unique milestone IDs and unique PR IDs
    milestone_ids = [item["id"] for item in milestones]
    repeated_milestones = duplicates(milestone_ids)
    if repeated_milestones:
        check.fail(f"duplicate milestone IDs: {sorted(repeated_milestones)}")

    pr_ids = [item["id"] for item in prs]
    repeated_prs = duplicates(pr_ids)
    if repeated_prs:
        check.fail(f"duplicate work item IDs: {sorted(repeated_prs)}")

    milestones_by_id = {item["id"]: item for item in milestones}
    prs_by_id = {item["id"]: item for item in prs}

    for pr_id, pr in prs_by_id.items():
        m_id = pr.get("milestone")
        if m_id is not None and m_id not in milestones_by_id:
            check.fail(f"{pr_id}: references unknown milestone {m_id}")

    # Check depends_on: all referenced dependencies must exist in pull_requests
    for pr_id, pr in prs_by_id.items():
        for dependency in pr.get("depends_on", []):
            if dependency not in prs_by_id:
                check.fail(f"{pr_id}: unknown dependency {dependency}")

    # Topological cycle detection
    pr_dependencies = {pr_id: pr.get("depends_on", []) for pr_id, pr in prs_by_id.items()}
    check_dependency_cycles(check, pr_dependencies)

    # Check dependency status: if a PR has state "merged", all its dependencies must also be "merged"
    for pr_id, pr in prs_by_id.items():
        if pr.get("state") == "merged":
            unmerged = [
                dep
                for dep in pr.get("depends_on", [])
                if dep in prs_by_id and prs_by_id[dep].get("state") != "merged"
            ]
            if unmerged:
                check.fail(f"{pr_id}: merged work item has unmerged dependencies: {unmerged}")

    # Check integration_gates: unique IDs, non-empty requires, valid PR references
    gates = plan.get("integration_gates", [])
    gate_ids = [gate["id"] for gate in gates]
    repeated_gates = duplicates(gate_ids)
    if repeated_gates:
        check.fail(f"duplicate integration gate IDs: {sorted(repeated_gates)}")

    for gate in gates:
        g_id = gate.get("id", "unknown")
        requires = gate.get("requires", [])
        if not requires:
            check.fail(f"integration gate {g_id}: requires array must be non-empty")
        for req_pr in requires:
            if req_pr not in prs_by_id:
                check.fail(f"integration gate {g_id}: unknown required work item {req_pr}")

    # Check issues: all 7 issues in plan["issues"] must map to valid closure_owner PRs
    issue_numbers = [item["number"] for item in issues]
    repeated_issues = duplicates(issue_numbers)
    if repeated_issues:
        check.fail(f"duplicate issue numbers: {sorted(repeated_issues)}")
    if set(issue_numbers) != EXPECTED_ISSUES:
        check.fail(f"issue set differs: {sorted(set(issue_numbers) ^ EXPECTED_ISSUES)}")

    for issue in issues:
        number = issue["number"]
        owner = issue.get("closure_owner")
        if not owner:
            check.fail(f"issue #{number}: missing closure_owner")
        elif owner not in prs_by_id:
            check.fail(f"issue #{number}: unknown closure_owner {owner}")

    # Cross-check issues with docs/v2/issue-ownership.md
    issue_text = check.read_text(ISSUES)
    for issue in issues:
        identity = f"issue-{issue['number']}"
        section = anchor_section(issue_text, identity)
        if section is not None:
            expected = (f"#{issue['number']}", issue.get("closure_owner", ""))
            missing = [value for value in expected if value not in section]
            if missing:
                check.fail(f"{ISSUES}: {identity!r} section is missing {', '.join(repr(value) for value in missing)}")
        else:
            check.fail(f"{ISSUES}: missing anchor section for {identity!r}")

    for pr_id, pr in prs_by_id.items():
        for issue_num in pr.get("issues", []):
            if issue_num not in EXPECTED_ISSUES:
                check.fail(f"{pr_id}: references unknown issue #{issue_num}")

    # Check legacy mapping: all 65 legacy items mapped to completed or valid finish PR owners
    legacy_by_id = {item["id"]: item for item in legacy_items}
    repeated_legacy = duplicates(item["id"] for item in legacy_items)
    if repeated_legacy:
        check.fail(f"duplicate legacy work item IDs: {sorted(repeated_legacy)}")

    if set(legacy_by_id) != EXPECTED_LEGACY_WORK_ITEMS:
        check.fail(
            f"legacy work items differ from expected 65 items: "
            f"{sorted(set(legacy_by_id) ^ EXPECTED_LEGACY_WORK_ITEMS)}"
        )

    valid_legacy_states = {"merged", "unmerged", "partial"}
    for item in legacy_items:
        state = item.get("state")
        if state not in valid_legacy_states:
            check.fail(
                f"{item['id']}: legacy item state {state!r} must be one of {sorted(valid_legacy_states)}"
            )

    for item_id, item in legacy_by_id.items():
        if item_id in RETAINED_COMPLETED_ITEMS:
            if item.get("state") != "merged":
                check.fail(f"{item_id}: completed legacy item must have state 'merged', got {item.get('state')!r}")
        else:
            if item.get("state") == "merged":
                check.fail(
                    f"{item_id}: incomplete legacy item cannot have state 'merged' before finish PRs complete"
                )
            new_owners = item.get("new_owners", [])
            if not new_owners:
                check.fail(f"{item_id}: incomplete legacy item must have at least one new_owner")
            for owner in new_owners:
                if owner not in prs_by_id:
                    check.fail(f"{item_id}: new_owner {owner!r} not found in pull_requests")

    # Bidirectional reconciliation between legacy_work_items new_owners and PR replaces
    expected_replaces: dict[str, set[str]] = {pr_id: set() for pr_id in prs_by_id}
    for item in legacy_items:
        if item.get("state") != "merged":
            for owner in item.get("new_owners", []):
                if owner in expected_replaces:
                    expected_replaces[owner].add(item["id"])
                else:
                    check.fail(f"{item['id']}: new_owner {owner!r} not found in pull_requests")

    for pr_id, pr in prs_by_id.items():
        actual = set(pr.get("replaces", []))
        expected = expected_replaces[pr_id]
        if actual != expected:
            check.fail(
                f"{pr_id}: replaces {sorted(actual)} does not match legacy work item mapping {sorted(expected)}"
            )

    # Check decision anchors in finalized decisions
    decision_text = check.read_text(DECISIONS)
    decision_anchors = anchors(decision_text)
    for number in range(1, 23):
        anchor_id = f"d-{number:03d}"
        if anchor_id not in decision_anchors:
            check.fail(f"{DECISIONS}: missing decision anchor {anchor_id!r}")


def check_plan_targets(check: Check, plan: dict[str, Any]) -> None:
    records: list[tuple[str, str | None]] = []
    for milestone in plan.get("milestones", []):
        records.append((milestone["id"], milestone.get("file")))
    for pr in plan.get("pull_requests", []):
        records.append((pr["id"], pr.get("file")))

    for identity, reference in records:
        if not reference:
            check.fail(f"{identity}: missing file target reference")
            continue
        path_text, separator, fragment = reference.partition("#")
        path = pathlib.PurePosixPath(path_text)
        if path.is_absolute():
            check.fail(f"{identity}: file target must not be absolute: {reference}")
            continue

        if tuple(path.parts[:2]) == ("docs", "v2"):
            target = (check.root / pathlib.Path(*path.parts)).resolve()
            rel_path = path_text
        else:
            target = (check.root / "docs" / "v2" / "roadmap" / pathlib.Path(*path.parts)).resolve()
            rel_path = str(target.relative_to(check.root))

        if not target.is_relative_to(check.root):
            check.fail(f"{identity}: file target escapes repository: {reference}")
            continue
        if any(part.startswith("_") for part in target.relative_to(check.root).parts):
            check.fail(f"{identity}: unsafe file target in underscore directory: {reference}")
            continue
        if not target.is_file():
            check.fail(f"{identity}: missing file target: {reference}")
            continue
        if target.stat().st_size == 0:
            check.fail(f"{identity}: file target is empty: {reference}")
            continue

        ignored = subprocess.run(
            ["git", "check-ignore", "--quiet", "--", rel_path],
            cwd=check.root,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        if ignored == 0:
            check.fail(f"{identity}: file target is ignored rather than trackable: {reference}")
        if fragment and fragment not in anchors(target.read_text(encoding="utf-8")):
            check.fail(f"{identity}: missing target fragment: {reference}")

    for key in ("sources_file", "entrypoint"):
        fname = plan.get(key)
        if fname:
            target = check.root / "docs" / "v2" / "roadmap" / fname
            if not target.is_file():
                check.fail(f"plan {key} missing: {fname}")


def check_finish_documents(check: Check, plan: dict[str, Any]) -> None:
    roadmap_dir = check.root / "docs" / "v2" / "roadmap"
    for pr in plan.get("pull_requests", []):
        pr_id = pr["id"]
        fname = pr.get("file")
        if not fname:
            continue
        target = roadmap_dir / fname
        if target.is_file():
            text = target.read_text(encoding="utf-8")
            if pr_id not in text:
                check.fail(f"{fname}: missing PR ID {pr_id!r} in document body")

    for milestone in plan.get("milestones", []):
        m_id = milestone["id"]
        fname = milestone.get("file")
        if not fname:
            continue
        target = roadmap_dir / fname
        if target.is_file():
            text = target.read_text(encoding="utf-8")
            if m_id not in text:
                check.fail(f"{fname}: missing milestone ID {m_id!r} in document body")

    historical_files = (
        "milestones.md",
        "work-items-00-04.md",
        "work-items-05-10.md",
        "work-item-contract.md",
    )
    for fname in historical_files:
        fpath = roadmap_dir / fname
        if fpath.is_file():
            text = fpath.read_text(encoding="utf-8")
            if not text.startswith("> **Historical reference notice:"):
                check.fail(f"{fname}: missing historical reference notice banner")


def link_target(check: Check, source: pathlib.Path, raw: str) -> tuple[pathlib.Path, str] | None:
    value = unquote(raw.strip("<>"))
    parsed = urlsplit(value)
    if parsed.scheme in {"http", "https", "mailto"}:
        return None
    if parsed.scheme or parsed.netloc or value.startswith(("/", "~")) or re.match(r"^[A-Za-z]:[\\/]", value):
        check.fail(f"{source}: absolute or unsupported link target {raw!r}")
        return None
    path_part = parsed.path
    target = source if not path_part else source.parent / pathlib.PurePosixPath(path_part)
    resolved = (check.root / target).resolve()
    if not resolved.is_relative_to(check.root):
        check.fail(f"{source}: link escapes repository: {raw!r}")
        return None
    relative = resolved.relative_to(check.root)
    if any(part.startswith("_") for part in relative.parts):
        check.fail(f"{source}: link targets an underscore working directory: {raw!r}")
        return None
    return relative, parsed.fragment


def check_markdown_links(check: Check) -> None:
    docs = check.root / "docs" / "v2"
    for source_file in sorted(docs.rglob("*.md")):
        source = source_file.relative_to(check.root)
        text = source_file.read_text(encoding="utf-8")
        for raw in LINK_RE.findall(text):
            target_info = link_target(check, source, raw)
            if target_info is None:
                continue
            target, fragment = target_info
            target_path = check.root / target
            if not target_path.exists():
                check.fail(f"{source}: missing link target {raw!r}")
                continue
            if fragment:
                if not target_path.is_file():
                    check.fail(f"{source}: fragment target is not a file: {raw!r}")
                    continue
                target_anchors = anchors(target_path.read_text(encoding="utf-8"))
                if fragment not in target_anchors:
                    check.fail(f"{source}: missing explicit fragment {raw!r}")
    pdfs = sorted(
        path.relative_to(check.root)
        for path in docs.rglob("*")
        if path.is_file() and path.suffix.lower() == ".pdf"
    )
    if pdfs:
        check.fail(f"docs/v2: PDF files are forbidden: {pdfs}")


def check_repository_policy(check: Check) -> None:
    readme = check.read_text(pathlib.Path("README.md"))
    agents = check.read_text(pathlib.Path("AGENTS.md"))
    v2_readme = check.read_text(pathlib.Path("docs/v2/README.md"))
    eol = check.read_text(pathlib.Path("docs/v2/eol-and-version-policy.md"))
    for path, text in (("README.md", readme), ("AGENTS.md", agents)):
        if "docs/v2/README.md" not in text:
            check.fail(f"{path}: missing link to docs/v2/README.md")
    role_requirements = {
        "implementer edits repository files and runs tests only": "implementer edit/test-only boundary",
        "orchestrator owns commits, pushes, non-draft pr creation and updates": "orchestrator Git/GitHub ownership",
        "one independent read-only review is the default": "independent review default",
        "required exact-head checks are green": "exact-head required checks",
        "final review is blocker/major-clean": "Blocker/Major-clean review",
        "repository policy and branch protection permit the merge": "repository merge permission",
        "scope and worktree state are clean": "clean scope and worktree",
        "user has given explicit authorization": "explicit merge authorization",
        "a changed head voids pass": "changed-head invalidation",
        "does not authorize self-approval, auto-merge": "self-approval and auto-merge prohibition",
        "branch-protection changes": "branch-protection mutation prohibition",
    }
    agents_lower = re.sub(r"\s+", " ", agents.lower())
    for text, label in role_requirements.items():
        if text not in agents_lower:
            check.fail(f"AGENTS.md: missing corrected {label}")
    for obsolete in (
        "agents never merge 2.0 work",
        "repository owner alone merges",
        "implementation agent opens a non-draft pr",
    ):
        if obsolete in agents_lower:
            check.fail(f"AGENTS.md: superseded role policy remains: {obsolete!r}")
    handoff_fields = [
        "Plan ID:",
        "PR URL:",
        "Base SHA / Head SHA / commits:",
        "Outcome delivered:",
        "Files/subsystems changed:",
        "Public API and persisted/profile changes:",
        "Commands and results:",
        "Benchmarks/fuzz/crash evidence:",
        "Decisions and deviations:",
        "Temporary bridges and deletion owner:",
        "Known risks/follow-ups:",
        "Reviewer questions:",
    ]
    for field in handoff_fields:
        if field not in agents:
            check.fail(f"AGENTS.md: missing required handoff field {field!r}")
    template = check.read_text(PULL_REQUEST_TEMPLATE)
    template_fields = [
        "Plan ID",
        "Base / head / commits",
        "Objective",
        "Scope",
        "Non-goals",
        "Deviations / replan",
        "Public API / persisted / profile changes",
        "Commands and results",
        "Skipped validation",
        "Benchmark / fuzz / crash disposition",
        "Temporary bridges / deletion owner",
        "Risks / follow-ups",
        "Reviewer questions",
        "Role and merge-eligibility confirmations",
    ]
    for field in template_fields:
        if field not in template:
            check.fail(f"{PULL_REQUEST_TEMPLATE}: missing required field {field!r}")
    pending = "pending owner-only"
    if pending not in v2_readme.lower() or pending not in eol.lower():
        check.fail("archive state must be described as pending owner-only in v2 README and EOL policy")
    local_working_dirs = set(re.findall(r"(?m)^- `(_[A-Za-z0-9_-]+)/`", agents))
    if not local_working_dirs:
        check.fail("AGENTS.md: no top-level local-only underscore directories declared")
    for path in sorted((check.root / "docs" / "v2").rglob("*")):
        text = path.read_text(encoding="utf-8") if path.is_file() and path.suffix in {".md", ".json"} else ""
        referenced_dirs = set(LOCAL_DIRECTORY_RE.findall(text))
        referenced_dirs.update(
            forbidden
            for forbidden in local_working_dirs
            if re.search(rf"(?<![A-Za-z0-9/]){re.escape(forbidden)}(?:/)?\b", text)
        )
        for forbidden in sorted(referenced_dirs):
            check.fail(f"{path.relative_to(check.root)}: local-only source name {forbidden!r} is forbidden")


def workflow_job(text: str, job_id: str) -> str | None:
    lines = text.splitlines()
    marker = f"  {job_id}:"
    try:
        start = lines.index(marker)
    except ValueError:
        return None
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if re.fullmatch(r"  [A-Za-z0-9_-]+:", lines[index]):
            end = index
            break
    return "\n".join(lines[start:end])


def check_validation_workflows(check: Check) -> None:
    required_provenance = {
        pathlib.Path(".github/workflows/ci.yml"): ("rust-compile-and-test", "v2-plan-contract"),
        pathlib.Path(".github/workflows/release.yml"): ("v2-plan-contract",),
    }
    for path in VALIDATION_WORKFLOWS:
        text = check.read_text(path)
        lines = text.splitlines()
        expected_env = f"  EXPECTED_REVISION: {EXACT_REVISION_EXPRESSION}"
        if lines.count(expected_env) != 1:
            check.fail(f"{path}: EXPECTED_REVISION must select the pull-request head with github.sha fallback")
        checkout_count = 0
        for index, line in enumerate(lines):
            if line.strip() != "- uses: actions/checkout@v7":
                continue
            checkout_count += 1
            indent = line[: len(line) - len(line.lstrip())]
            expected = [f"{indent}  with:", f"{indent}    ref: {EXACT_REVISION_EXPRESSION}"]
            if lines[index + 1:index + 3] != expected:
                check.fail(f"{path}:{index + 1}: checkout must select the exact event revision")
        if checkout_count == 0:
            check.fail(f"{path}: no actions/checkout@v7 steps found")
        for job_id in required_provenance[path]:
            job = workflow_job(text, job_id)
            if job is None:
                check.fail(f"{path}: required job {job_id!r} is missing")
            elif job.count(PROVENANCE_COMMAND) != 1:
                check.fail(f"{path}: job {job_id!r} must assert checkout provenance")


def check_executable_baseline(check: Check) -> None:
    helper_path = check.root / BASELINE_HELPER
    if not helper_path.is_file():
        check.fail(f"{BASELINE_HELPER}: executable baseline validator is missing")
        return
    spec = importlib.util.spec_from_file_location("selene_v2_baseline_validator", helper_path)
    if spec is None or spec.loader is None:
        check.fail(f"{BASELINE_HELPER}: executable baseline validator cannot be loaded")
        return
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
        errors = module.validate_tracked_baseline(check.root)
    except Exception as error:  # noqa: BLE001 - validator failures belong in one report.
        check.fail(f"{BASELINE_HELPER}: executable baseline validator failed: {error}")
        return
    for error in errors:
        check.fail(f"executable baseline: {error}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, required=True, help="repository root")
    parser.add_argument(
        "--write-projections",
        action="store_true",
        help="retained for CLI compatibility; finish plan uses individual PR documents",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    check = Check(args.root)
    plan = check.read_json(PLAN)
    schema = check.read_json(SCHEMA)
    check_closed_schema(check, schema)
    if isinstance(plan, dict) and isinstance(schema, dict):
        validate_schema_value(check, plan, schema, schema, "plan")
        if not check.errors:
            check_plan_semantics(check, plan)
            check_plan_targets(check, plan)
            check_finish_documents(check, plan)
    check_markdown_links(check)
    check_repository_policy(check)
    check_validation_workflows(check)
    check_executable_baseline(check)
    if check.errors:
        print("v2 plan validation failed:", file=sys.stderr)
        for error in check.errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    milestone_count = len(plan.get("milestones", [])) if isinstance(plan, dict) else 0
    pr_count = len(plan.get("pull_requests", [])) if isinstance(plan, dict) else 0
    issue_count = len(plan.get("issues", [])) if isinstance(plan, dict) else 0
    print(f"v2 plan validation passed: {milestone_count} milestones, {pr_count} work items, {issue_count} issues")
    return 0


if __name__ == "__main__":
    sys.exit(main())
