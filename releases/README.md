# Release Notes

Top-level release notes live here as `releases/<version>.md`.

Each release note aggregates per-crate changelog entries and adds the cross-crate
context needed by users upgrading a full selene-db workspace.

## Pending Pre-Release Notes

- BRIEF-03: declared transaction model is single-writer + MVCC; ID allocation produces permanent holes on abort; WAL carries opaque caller principal slot.
- BRIEF-04a: snapshot section tags are 8 bytes (provider+sub-tag); imbl-shaped state archives via rkyv sorted-vec intermediate; recovery is two-step (snapshot apply + WAL replay) with the canonical IndexProvider trait in spec 06.
- BRIEF-04b: extension API surface hardened — single ProcedureRegistry trait in selene-gql; per-tier concrete Context structs with dyn-compatible Procedure traits; pack-lifecycle audit consolidates through WAL only (AuditEntry/EMITTED_AUDIT removed).
- BRIEF-05: selene-core foundation bootstrapped with primitive IDs, IStr interning, Value variants, extension type IDs, and ValueTypeAdapter registry.
- BRIEF-05.1: IStr cap admission is now atomic under concurrency (double-checked locking with a static admission Mutex); fixes PR #3 Copilot P1 finding.
