# Release Notes

Top-level release notes live here as `releases/<version>.md`.

Each release note aggregates per-crate changelog entries and adds the cross-crate
context needed by users upgrading a full selene-db workspace.

## Pending Pre-Release Notes

- BRIEF-03: declared transaction model is single-writer + MVCC; ID allocation produces permanent holes on abort; WAL carries opaque caller principal slot.
