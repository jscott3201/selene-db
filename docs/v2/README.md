# Selene DB 2.0 program

This directory is the tracked program-of-record for Selene DB 2.0. It contains
11 milestones, 64 PR-sized work items, 22 finalized decisions, and one owner for
each of the seven open issues recorded at installation.

## Read in this order

1. [Finalized decisions](decisions/finalized.md)
2. [Milestones and critical path](roadmap/milestones.md)
3. [Work items M00–M04](roadmap/work-items-00-04.md) or
   [work items M05–M10](roadmap/work-items-05-10.md)
4. [Work-item contract](roadmap/work-item-contract.md)
5. [Review protocol](review-protocol.md)
6. [Operating guide](operating-guide.md)

The [machine plan](roadmap/plan.json) is a portable mirror of the reviewed
package. Its [closed schema](roadmap/plan.schema.json) and the positive
validator pin counts, dependencies, issue ownership, projections, and links.
The machine plan is governance data, not an engine or build input. Markdown is
the human review surface. A work-item `status` records merged state only; it is
not an estimate of readiness or progress.

## Authority and current state

The architecture review used source snapshot
`b8782bec34ff0b815b62711ac7e33cac09d8ea71`. This package was installed from
base `c5c0a9855f5c043ecc927d561e4ad8ba001346d9`. Keep those coordinates
distinct when citing evidence.

The c5 baseline has six workspace crates, no `selene-db` facade or catalog
crate, a row-oriented executor, and the current persistence formats. Those are
current-source facts, not the target architecture. The roadmap introduces the
profile authority in M01, the facade and catalog in M02, batch execution in
M06, and format 2 in M09. Do not describe target components as implemented
before their owning work items merge.

The generated `selene-profile` capability records are implementation inventory.
They cannot support a formal 2.0 conformance claim by themselves; use the
[evidence-gated conformance policy](conformance-policy.md).

The archive branch and tag named by D-002 are **pending owner-only** actions.
They were absent when this package was installed. Verify both refs at the
reviewed snapshot before relying on them.

Untracked local material may support research, but future work must be
executable from tracked source, this package, current tests, and read-only
repository or issue evidence.

## Policy and evidence documents

- [2.0 line and 1.x end-of-life policy](eol-and-version-policy.md)
- [Conformance and claim policy](conformance-policy.md)
- [Source snapshot and assumptions](source-snapshot-and-assumptions.md)
- [Executable final-1.x baseline](baseline/README.md)
- [Risk register](risk-register.md)
- [Issue ownership](issue-ownership.md)
- [Review protocol](review-protocol.md)
- [Operating guide](operating-guide.md)
- [Post-GA backlog](post-ga-backlog.md)

M00-PR03 added CI wiring, compile-lane changes, validator failure fixtures, the
PR handoff template, corrected role policy, and deterministic tool pins.

From the repository root:

```bash
python3 -B .github/scripts/check-v2-plan.py --root .
```
