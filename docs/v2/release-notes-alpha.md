# 2.0.0-alpha.1 — release-notes draft

**Draft, not a release announcement. Publication, tagging and release remain
pending separate owner authorization.** This version identifies the qualified
source candidate, not a verified registry publication or a GA compatibility
promise. See [qualification evidence](release-qualification.md).

## Alpha boundary

Selene DB is an embedded Rust graph database. Applications use the `selene-db`
facade for named graphs, owned sessions/requests/results, schema construction and
durable lifecycle. GQL remains the only query/mutation language. The native engine
joins batch execution, mixed directed/undirected edges, bounded path execution,
constraints, scalar JSON expression indexes and graph/vector/text/JSON retrieval.
Native values, indexes and `selene.*` / `algo.*` procedures are disclosed Selene
facilities, not extra ISO grammar or a loadable extension ABI.

The permitted statement is **“ISO-aligned with disclosed conformance gaps; not a
complete selected-profile claim.”** Neither ISO minimum conformance nor complete
selected-profile conformance is claimed. The formal `selected_profile` claim is
**denied-by-design for alpha**. The September 13 owner decision narrows the alpha
delivery boundary in prose only; it does not change canonical feature selection,
generated claims, evidence dispositions or the Flagger.

Canonical-target gaps remain GC03, GE04, GE05, GG02, GG20, GG21, GP16, GQ01, GV66,
GV67 and implied GV60/GV61/GV65. The rule inventory remains `seeded_incomplete`,
and applicable Annex B choices remain pending. Their completion is deferred as
specified in the [tracked backlog](post-ga-backlog.md). The exact boundary and
embedding rules are in [release readiness](release-readiness.md).

## Durability and compatibility

- Managed filesystem mode reads/writes **format 2 only**. Format-1 headers reject
  without payload decoding; there is no 1.x decoder, migration or maintenance
  support. Rebuild from application-owned source data into a fresh store. No
  cross-alpha persisted-format compatibility is promised.
- The infallible builder is memory-only. Fallible create/open/checkpoint use
  retained directory handles, one writer ownership domain and eager required-index
  reconstruction. Open does not silently repair or return background readiness.
- Durable cancellation, uncertainty and synchronized-but-unacknowledged completion
  are distinct typed outcomes. An indeterminate result is not an instruction to
  retry a non-idempotent write. Reconcile authoritative state first.
- Checkpoint/rotation and explicit prune preserve selected artifacts and reader
  leases. Verification describes a captured on-disk view, not acknowledgment,
  write permission, physical durability or subsequent freshness. Process-kill
  tests are not filesystem/device power-loss certification.
- Stable element/catalog IDs and process-local reference provenance are distinct.
  Reissue graph/node/edge handles after reopen. Lower engine construction and
  physical-row types are not stable facade exports.

## Platforms and limits

Rust 1.97.1 / edition 2024 is the declared floor. Managed filesystem mode supports
native Linux and macOS on compatible filesystems/storage stacks; other platforms
return typed unsupported-platform errors. This candidate was exercised on native
macOS arm64. **Linux qualification is unavailable in this local run**, not passed;
neither emulation nor cross-compilation was used. Other CPU/filesystem combinations
were not qualified here.

Resources remain bounded: no unlimited intermediate state, path search or disk
spill is promised. F06-QUAL-04 adds a pre-pest `5GQL1` program limit for excessive
active bare nested-query wrappers, preventing the reproduced brace-backtracking
timeout. Eight consecutive bare query levels remain admitted; further active
brace-to-brace wrappers reject before descent. Record and `EXISTS` nesting retain
their existing limits. Grammar, normal result semantics and fuzz timeouts are
unchanged. The bounded fuzz campaigns are evidence, not exhaustive DoS proof.

## Artifacts and remaining work

Eight public Rust crates package and build in dependency order, with MIT/Apache-2.0
license texts, NOTICE and third-party attribution. An external consumer passed
default/all-feature tests using only extracted candidate packages for local engine
dependencies; normal version requirements remain in their normalized manifests.
There are no new server binaries, binding releases or wheels.

This work does not introduce audit logging, backup/export/migration tools,
replication or CDC. The existing read-only `Database::verify` API remains part of
the facade. Full native Linux/exact-head release gates and independent review
remain delivery-owner work. No package upload, tag, GitHub release or issue closure
is authorized by this draft.
