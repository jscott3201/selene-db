# Source snapshot and assumptions

## Coordinates

| Item | Value |
|---|---|
| Repository | `jscott3201/selene-db` |
| Integration branch | `development` |
| Reviewed architecture/archive snapshot | `b8782bec34ff0b815b62711ac7e33cac09d8ea71` |
| Program installation base | `c5c0a9855f5c043ecc927d561e4ad8ba001346d9` |
| Workspace coordinate at installation | `2.0.0-alpha.1` |
| Toolchain / edition | Rust 1.97.1 / edition 2024 |
| Current crates | `selene-core`, `selene-graph`, `selene-persist`, `selene-algorithms`, `selene-gql`, `selene-testing` |
| Future crates owned by roadmap | `selene-profile`, `selene-catalog`, `selene-db` |

The architecture review inspected source, documentation, workflows, issues,
existing evidence, and licensed reference material. It did not independently
execute the complete Rust workspace, fuzz targets, crash matrix, mutation
suite, or benchmarks. M00-PR04 owns that executable baseline.

## Current and target boundaries

Current architecture and persistence documentation describes the c5 source:
six crates, direct graph/session construction, row-oriented execution, and the
current WAL/snapshot/audit formats. The target adds a generated profile in M01,
database facade and catalog in M02, explicit contexts in M03, private typed
identity in M04, semantic/logical/physical layers in M05–M06, and format 2 in
M09. A target description is not implementation evidence.

The named archive branch and tag are **pending owner-only** actions. They were
absent at installation. The owner must create and protect them at the reviewed
snapshot, then verify the refs and release non-trigger before any work relies on
their presence.

## Assumptions to verify

- The reviewed snapshot remains the intended archive cut.
- Archive-tag classification still prevents publication.
- Full gates remain reproducible on the pinned toolchain and supported systems.
- The single-writer/immutable-reader model has no hidden contract requiring
  MVCC.
- Derived vector, text, JSON, index, and algorithm state can remain rebuildable.
- No user requires 2.0 to open or migrate a 1.x store.
- Unicode and collation choices can be pinned in profile and store metadata.

Current source and executable evidence win over a plan detail. A contradiction
returns REPLAN; it does not authorize changing the EOL, product, or claim
boundary in place.
