#!/usr/bin/env python3
"""Validate the tracked Selene DB 2.0 program contract."""

from __future__ import annotations

import argparse
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
MILESTONES = pathlib.Path("docs/v2/roadmap/milestones.md")
WORK_ITEMS_LOW = pathlib.Path("docs/v2/roadmap/work-items-00-04.md")
WORK_ITEMS_HIGH = pathlib.Path("docs/v2/roadmap/work-items-05-10.md")
DECISIONS = pathlib.Path("docs/v2/decisions/finalized.md")
ISSUES = pathlib.Path("docs/v2/issue-ownership.md")
PULL_REQUEST_TEMPLATE = pathlib.Path(".github/pull_request_template.md")
VALIDATION_WORKFLOWS = (
    pathlib.Path(".github/workflows/ci.yml"),
    pathlib.Path(".github/workflows/release.yml"),
)
EXACT_REVISION_EXPRESSION = "${{ github.event.pull_request.head.sha || github.sha }}"
PROVENANCE_COMMAND = 'run: test "$(git rev-parse HEAD)" = "$EXPECTED_REVISION"'
EXPECTED_ISSUES = {1088, 1092, 1093, 1094, 1097, 1128, 1137}
LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+[^)]*)?\)")
ANCHOR_RE = re.compile(r'<a\s+id=["\']([^"\']+)["\']\s*></a>', re.IGNORECASE)
LOCAL_DIRECTORY_RE = re.compile(r"(?<![A-Za-z0-9/])(_[A-Za-z0-9][A-Za-z0-9_-]*)/")


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


def json_type_matches(value: Any, expected: str) -> bool:
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
        for dependency in sorted(dependencies[identity]):
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


def check_plan_semantics(check: Check, plan: dict[str, Any]) -> None:
    meta = plan["meta"]
    collections = {
        "milestone": plan["milestones"],
        "work item": plan["pull_requests"],
        "issue": plan["issues"],
        "decision": plan["decisions"],
    }
    actual = {name: len(items) for name, items in collections.items()}
    declared = {
        "milestone": meta["milestone_count"],
        "work item": meta["pr_count"],
        "issue": meta["issue_count"],
        "decision": meta["decision_count"],
    }
    expected = {"milestone": 11, "work item": 64, "issue": 7, "decision": 22}
    for name in expected:
        if actual[name] != expected[name] or declared[name] != expected[name]:
            check.fail(f"plan counts: {name} declared={declared[name]} actual={actual[name]} expected={expected[name]}")

    milestones = {item["id"]: item for item in plan["milestones"]}
    prs = {item["id"]: item for item in plan["pull_requests"]}
    issues = {item["number"]: item for item in plan["issues"]}
    decisions = {item["id"]: item for item in plan["decisions"]}
    for label, source, mapped in (
        ("milestone", plan["milestones"], milestones),
        ("work item", plan["pull_requests"], prs),
        ("issue", plan["issues"], issues),
        ("decision", plan["decisions"], decisions),
    ):
        key = "number" if label == "issue" else "id"
        repeated = duplicates(item[key] for item in source)
        if repeated:
            check.fail(f"duplicate {label} IDs: {sorted(repeated)}")
        if len(mapped) != len(source):
            check.fail(f"{label} IDs are not unique")

    expected_milestones = {f"M{number:02d}" for number in range(11)}
    if set(milestones) != expected_milestones:
        check.fail(f"milestone set differs: {sorted(set(milestones) ^ expected_milestones)}")
    if set(issues) != EXPECTED_ISSUES:
        check.fail(f"issue set differs: {sorted(set(issues) ^ EXPECTED_ISSUES)}")
    expected_decisions = {f"D-{number:03d}" for number in range(1, 23)}
    if set(decisions) != expected_decisions:
        check.fail(f"decision set differs: {sorted(set(decisions) ^ expected_decisions)}")

    memberships: list[str] = []
    for milestone_id, milestone in milestones.items():
        if milestone["number"] != int(milestone_id[1:]):
            check.fail(f"{milestone_id}: number does not match ID")
        for dependency in milestone["dependencies"]:
            if dependency not in milestones and dependency not in prs:
                check.fail(f"{milestone_id}: unknown dependency {dependency}")
        memberships.extend(milestone["pr_ids"])
        for pr_id in milestone["pr_ids"]:
            if pr_id not in prs:
                check.fail(f"{milestone_id}: unknown member {pr_id}")
            elif prs[pr_id]["milestone"] != milestone_id:
                check.fail(f"{pr_id}: milestone membership disagrees with {milestone_id}")
    repeated_members = duplicates(memberships)
    if repeated_members:
        check.fail(f"work items appear in multiple milestones: {sorted(repeated_members)}")
    if set(memberships) != set(prs):
        check.fail(f"milestone membership differs: {sorted(set(memberships) ^ set(prs))}")

    for pr_id, pr in prs.items():
        if pr_id[:3] != pr["milestone"] or int(pr_id[-2:]) != pr["number"]:
            check.fail(f"{pr_id}: owner milestone or number does not match ID")
        for dependency in pr["dependencies"]:
            if dependency not in prs:
                check.fail(f"{pr_id}: unknown dependency {dependency}")
        for issue in pr["issues"]:
            if issue not in issues:
                check.fail(f"{pr_id}: references unknown issue #{issue}")

    dependency_graph = {identity: record["dependencies"] for identity, record in milestones.items()}
    dependency_graph.update(
        {
            identity: [*record["dependencies"], record["milestone"]]
            for identity, record in prs.items()
        }
    )
    check_dependency_cycles(check, dependency_graph)

    issue_references: dict[int, list[str]] = {number: [] for number in issues}
    for pr_id, pr in prs.items():
        for number in pr["issues"]:
            issue_references.setdefault(number, []).append(pr_id)
    for number, issue in issues.items():
        owners = issue_references.get(number, [])
        if owners != [issue["owner"]]:
            check.fail(f"issue #{number}: owner={issue['owner']} work-item references={owners}")

    for pr_id, pr in prs.items():
        if pr["status"] == "Merged":
            unmerged = [
                dependency
                for dependency in pr["dependencies"]
                if dependency in prs and prs[dependency]["status"] != "Merged"
            ]
            if unmerged:
                check.fail(f"{pr_id}: merged work item has unmerged dependencies: {unmerged}")
        for field in ("scope", "non_goals", "acceptance", "tests", "review_focus", "stop_conditions", "bridge"):
            if not pr[field]:
                check.fail(f"{pr_id}: {field} contract is empty")


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


def check_plan_targets(check: Check, plan: dict[str, Any]) -> None:
    records = plan["milestones"] + plan["pull_requests"] + plan["issues"] + plan["decisions"]
    for record in records:
        reference = record["file"]
        path_text, separator, fragment = reference.partition("#")
        identity = record.get("id", f"issue-{record.get('number')}")
        if not separator or not fragment:
            check.fail(f"{identity}: file reference needs an explicit fragment: {reference}")
            continue
        path = pathlib.PurePosixPath(path_text)
        if path.is_absolute() or tuple(path.parts[:2]) != ("docs", "v2"):
            check.fail(f"{identity}: file target must be beneath docs/v2: {reference}")
            continue
        target = (check.root / pathlib.Path(*path.parts)).resolve()
        if not target.is_relative_to(check.root) or any(part.startswith("_") for part in path.parts):
            check.fail(f"{identity}: unsafe file target: {reference}")
            continue
        if not target.is_file():
            check.fail(f"{identity}: missing file target: {reference}")
            continue
        ignored = subprocess.run(
            ["git", "check-ignore", "--quiet", "--", path_text],
            cwd=check.root,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        if ignored == 0:
            check.fail(f"{identity}: file target is ignored rather than trackable: {reference}")
        if fragment not in anchors(target.read_text(encoding="utf-8")):
            check.fail(f"{identity}: missing target fragment: {reference}")


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
    pdfs = sorted(path.relative_to(check.root) for path in docs.rglob("*") if path.is_file() and path.suffix.lower() == ".pdf")
    if pdfs:
        check.fail(f"docs/v2: PDF files are forbidden: {pdfs}")


def projection_path(pr_id: str) -> str:
    group = "00-04" if int(pr_id[1:3]) <= 4 else "05-10"
    return f"work-items-{group}.md#{pr_id.lower()}"


def bullet(lines: list[str]) -> str:
    return "\n".join(f"- {line}" for line in lines)


def render_work_items(plan: dict[str, Any], low: int, high: int) -> str:
    lines = [f"# Selene DB 2.0 work items M{low:02d}–M{high:02d}", "", "<!-- Generated from plan.json; do not edit by hand. -->", "",
             "The machine plan carries additional design, path, documentation, and benchmark metadata for each contract.", ""]
    for pr in plan["pull_requests"]:
        number = int(pr["milestone"][1:])
        if not low <= number <= high:
            continue
        lines += [f'<a id="{pr["id"].lower()}"></a>', f'## {pr["id"]} — {pr["title"]}', "",
                  f'- **Owner:** {pr["milestone"]}', f'- **State:** {pr["status"]}',
                  f'- **Risk / size:** {pr["risk"]} / {pr["size"]}',
                  f'- **Dependencies:** {", ".join(pr["dependencies"]) or "None"}',
                  f'- **Issues:** {", ".join("#" + str(issue) for issue in pr["issues"]) or "None"}',
                  f'- **Commit scope:** `{pr["commit_scope"]}`', "", pr["outcome"], "", "### Scope", "", bullet(pr["scope"]), "",
                  "### Non-goals", "", bullet(pr["non_goals"]), "", "### Acceptance evidence", "", bullet(pr["acceptance"]), "",
                  "### Tests and gates", "", bullet(pr["tests"]), "", "### Review focus", "", bullet(pr["review_focus"]), "",
                  "### Stop conditions", "", bullet(pr["stop_conditions"]), "", "### Bridge and deletion", "", bullet(pr["bridge"]), ""]
    return "\n".join(lines).rstrip() + "\n"


def render_milestones(plan: dict[str, Any]) -> str:
    lines = ["# Selene DB 2.0 milestones", "", "<!-- Generated from plan.json; do not edit by hand. -->", "",
             "The dependency fields in the machine plan are authoritative. Work may overlap only after every listed dependency is satisfied.", "",
             "| ID | Milestone | Depends on | Work items |", "|---|---|---|---|"]
    for milestone in plan["milestones"]:
        deps = ", ".join(milestone["dependencies"]) or "None"
        prs = ", ".join(f"[{pr_id}]({projection_path(pr_id)})" for pr_id in milestone["pr_ids"])
        lines.append(f'| {milestone["id"]} | {milestone["title"]} | {deps} | {prs} |')
    lines += ["", "## Critical path", "", "`M00 → M01/M02 → M03 → M04 → M05 → M06 → M07 → M08 → M09 → M10`", ""]
    for milestone in plan["milestones"]:
        lines += [f'<a id="{milestone["id"].lower()}"></a>', f'## {milestone["id"]} — {milestone["title"]}', "", milestone["objective"], "",
                  f'**Dependencies:** {", ".join(milestone["dependencies"]) or "None"}', "", "**Entry:**", "", bullet(milestone["entry"]), "",
                  "**Exit:**", "", bullet(milestone["exit"]), ""]
    return "\n".join(lines).rstrip() + "\n"


def check_projections(check: Check, plan: dict[str, Any], write: bool) -> None:
    expected = {
        MILESTONES: render_milestones(plan),
        WORK_ITEMS_LOW: render_work_items(plan, 0, 4),
        WORK_ITEMS_HIGH: render_work_items(plan, 5, 10),
    }
    for path, content in expected.items():
        target = check.root / path
        if write:
            target.write_text(content, encoding="utf-8")
        elif check.read_text(path) != content:
            check.fail(f"{path}: projection is stale; run checker with --write-projections")

    designated = [
        (MILESTONES, [item["id"] for item in plan["milestones"]]),
        (WORK_ITEMS_LOW, [item["id"] for item in plan["pull_requests"] if int(item["milestone"][1:]) <= 4]),
        (WORK_ITEMS_HIGH, [item["id"] for item in plan["pull_requests"] if int(item["milestone"][1:]) >= 5]),
        (DECISIONS, [item["id"] for item in plan["decisions"]]),
        (ISSUES, [f'issue-{item["number"]}' for item in plan["issues"]]),
    ]
    for path, identities in designated:
        text = check.read_text(path)
        found = ANCHOR_RE.findall(text)
        for identity in identities:
            expected_anchor = identity.lower()
            if found.count(expected_anchor) != 1:
                check.fail(f"{path}: expected exactly one {expected_anchor!r} anchor")

    decision_text = check.read_text(DECISIONS)
    for decision in plan["decisions"]:
        identity = decision["id"].lower()
        section = anchor_section(decision_text, identity)
        if section is not None and decision["decision"] not in section:
            check.fail(f"{DECISIONS}: {identity!r} section does not contain its decision body")
    issue_text = check.read_text(ISSUES)
    for issue in plan["issues"]:
        identity = f"issue-{issue['number']}"
        section = anchor_section(issue_text, identity)
        if section is not None:
            expected = (f"#{issue['number']}", issue["owner"])
            missing = [value for value in expected if value not in section]
            if missing:
                check.fail(f"{ISSUES}: {identity!r} section is missing {', '.join(repr(value) for value in missing)}")


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
        "independent read-only reviewer pair": "independent reviewer pair",
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
        "Plan ID:", "PR URL:", "Base SHA / Head SHA / commits:", "Outcome delivered:",
        "Files/subsystems changed:", "Public API and persisted/profile changes:",
        "Commands and results:", "Benchmarks/fuzz/crash evidence:", "Decisions and deviations:",
        "Temporary bridges and deletion owner:", "Known risks/follow-ups:", "Reviewer questions:",
    ]
    for field in handoff_fields:
        if field not in agents:
            check.fail(f"AGENTS.md: missing required handoff field {field!r}")
    template = check.read_text(PULL_REQUEST_TEMPLATE)
    template_fields = [
        "Plan ID", "Base / head / commits", "Objective", "Scope", "Non-goals",
        "Deviations / replan", "Public API / persisted / profile changes",
        "Commands and results", "Skipped validation", "Benchmark / fuzz / crash disposition",
        "Temporary bridges / deletion owner", "Risks / follow-ups", "Reviewer questions",
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, required=True, help="repository root")
    parser.add_argument("--write-projections", action="store_true", help="rewrite deterministic Markdown projections")
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
            check_projections(check, plan, args.write_projections)
            check_plan_targets(check, plan)
    check_markdown_links(check)
    check_repository_policy(check)
    check_validation_workflows(check)
    if check.errors:
        print("v2 plan validation failed:", file=sys.stderr)
        for error in check.errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("v2 plan validation passed: 11 milestones, 64 work items, 7 issues, 22 decisions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
