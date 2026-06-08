# Changelog

All notable changes to selene-db are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Current-datetime keyword forms (ISO §20.27).** The parser now accepts bare
  `CURRENT_DATE`, `CURRENT_TIME`, `CURRENT_TIMESTAMP`, `LOCAL_TIMESTAMP`, and
  `LOCAL_TIME` value functions, plus the ISO optional `LOCAL_TIME()` spelling.
  These forms lower into the same niladic temporal evaluators as the existing
  call spellings, preserving request-stable timestamps and session time-zone
  semantics. The constructor call forms `DATE([string])`,
  `ZONED_TIME([string])`, `ZONED_DATETIME([string])`, `LOCAL_TIME([string])`,
  and `LOCAL_DATETIME([string])` also execute for niladic/current values and
  string-parameter temporal parsing. They now also accept record-constructor
  parameters, validate ISO field-name sets with `22G05`, and report invalid
  datetime field values with `22G06`.
- **Duration value function records (ISO §20.29).** `DURATION(...)` now accepts
  string parameters and open record-constructor parameters. Year/month records
  build year-month durations, day/time records build day-time durations, invalid
  duration field sets report `22G07`, and invalid generated duration text
  reports `22G0H`. `ABS(<duration>)` now implements the ISO duration absolute
  value function, returning a non-negative duration while preserving `NULL`
  propagation.
- **List TRIM value function (ISO §20.16).** `TRIM(<list>, <count>)` now
  returns the list with `<count>` tail elements removed, evaluates the count
  before the list per the ISO null short-circuit rule, and reports list-element
  errors as `22G0C`.
- **Path ELEMENTS value function (ISO §20.16).** `ELEMENTS(<path>)` now returns
  the ordered alternating node/edge reference list for a path value, propagates
  `NULL`, and records both `GF04` and `GV50` conformance features.
- **GP03 — explicit variable-scope `CALL` subqueries (ISO §15.2).** `CALL (x, y) { ... }`
  now binds and executes: the subquery body sees **only** the named imported
  variables, and an empty `CALL () { ... }` is fully isolated. An outer variable
  not in the import list is out of scope inside the body (`42N03` undefined
  reference), and duplicate import names are rejected (`42N10`). Implemented in
  the analyzer via a boundary subquery scope seeded with the named outer
  bindings' ids (so body references flow through `outer_binding_refs` unchanged —
  no lowering/runtime change); the former analyzer + planner `NotImplemented`
  rejects are removed and `GP03` moves from `NOT_SUPPORTED_RATIONALE` to
  `SUPPORTED_FEATURES`. Plain `CALL { ... }` (GP02, implicit import-all) is
  unchanged. `IN TRANSACTIONS` and write-ops-inside-`CALL{}` remain unsupported.
- **Caller-configurable implementation-defined caps (CAPS-IL).** Embedders can
  now set the `ImplDefinedCaps` limit surface (ISO IL013/IL015/IL018 — variable-
  length quantifier upper bound, set-op / `GROUP BY` distinct-key caps,
  optimizer-iteration and path-length bounds) via
  **`Session::with_impl_defined_caps(caps)`**, mirroring `with_deadline` /
  `with_row_cap`. Previously the caps existed and were honored by the
  runtime/optimizer but no public path could set non-default values, and the
  plan-time quantifier gate read `ImplDefinedCaps::default()` directly (so a
  caller value could neither loosen nor tighten it). The session caps are now
  threaded into planning via the new **`plan_with_caps(analyzed, registry, caps)`**
  entry point (`plan(..)` is retained as a default-caps wrapper) and consulted by
  both the plan-time quantifier gate (including quantifiers *inside* subquery
  bodies) and the runtime/optimizer cap checks. Adds `ImplDefinedCaps::DEFAULT`
  (a `const` so `Session::new` can stay `const fn`) and
  `ImplDefinedCaps::with_max_quantifier`.
- **CAST specification completeness (GA05, ISO §20.8).** The `<cast specification>`
  conversion matrix is now complete across the numeric family and `DECIMAL`:
  `DECIMAL` in both directions, numeric-family widening guarded by `i64::try_from`
  range-checks (no silent narrow-through), and strict-ISO boolean conversions —
  `BOOLEAN`↔numeric (including `BOOLEAN`↔`DECIMAL`, raising `22G03` for an
  out-of-domain value), `BOOLEAN`→`STRING` (uppercase `TRUE`/`FALSE`), and
  case-insensitive `STRING`→`BOOLEAN`. Builds on the `GA05` claim (see **Changed**
  / CONFORMANCE-00 below). (§20.8 is the `<cast specification>` clause; §22.10 is
  the distinct store-assignment clause.)
- **Value type predicates (GA06, ISO §19.6).** `IS [NOT] TYPED <value type>` is
  now advertised as the implemented `GA06` optional feature and the flagger stamps
  GA06 before any target value-type features. This is conformance accounting for
  the already-implemented typed predicate surface; runtime behavior is unchanged.
- **Counted shortest paths — `SHORTEST n PATH(S)` / `SHORTEST n GROUP(S)`
  (G019/G020, ISO §16.6).** The path-pattern grammar accepts the ISO counted
  forms, and the planner/executor return the `n` lowest-cost paths (PATHS) or the
  paths in the `n` lowest-cost length groups (GROUPS), per pattern.
- **Match modes — `DIFFERENT EDGES` / `REPEATABLE ELEMENTS` (G002/G003, ISO
  §16.4).** A `MATCH` may carry an explicit element-binding match mode.
  `DIFFERENT EDGES` imposes edge-uniqueness across the whole comma-separated
  pattern (NOTE 222 — "imparts `TRAIL` to each path pattern"); `REPEATABLE
  ELEMENTS` is the user-chosen default (ID086) and imposes no constraint. A
  multiply-declared edge variable yields the empty result (NOTE 225/227), not an
  error. Violations filter (no GQLSTATUS).
- **`GG21` "Explicit element type key label sets" — honest singleton (ISO
  §18.2/18.3).** The type-DDL grammar now parses the explicit
  `[ <label set phrase> ] <implies>` form, accepting **both** ISO `<implies>`
  spellings — the symbolic `=>` and the `IMPLIES` keyword (matched only in this
  position, not added to the global reserved-word set, so `implies` stays usable
  as an identifier). The flagger stamps `GG21` only for the explicit form (bare
  `:Name` stays `GG20`-only). Key-label-set cardinality is fixed at the singleton
  `IL003` bound (min=max=1): a cardinality-0 set is rejected with `42012` (node) /
  `42014` (edge) and a cardinality->1 set (e.g. `:A & :B =>`) with `42013` /
  `42015` — the spec's own GQLSTATUS. The accepted singleton is observationally
  identical to the implied `:Name` form (same `GraphTypeDef`, same WAL
  `SchemaChange`, exact-equality element-type identification — and it survives a
  snapshot+WAL recovery round-trip). The separate implied-label form
  (`:Person => :Employee`, which needs containment identification) is the only
  shape that returns `42N01` `FEATURE_NOT_SUPPORTED`; it and full multi-label key
  label sets are deferred to v1.3. `GG21` re-enters `SUPPORTED_FEATURES`; it
  implies `GG02` (§24.7), which stays claimed.

### Changed

- **ISO §20.27 current-datetime request timestamp.** `current_timestamp()`,
  `current_time()`, `current_date()`, `localtimestamp()`, and `localtime()` now
  share one request timestamp captured on the statement `TxContext` instead of
  re-reading the wall clock on each scalar-function call. Temporal casts that
  need the current session date use the same captured request timestamp.
- **Conformance-honesty: feature-ID corrections (CONFORMANCE-00).** Three
  fixes so the advertised optional-feature set matches the ISO taxonomy and the
  implemented surface:
  - **`CAST` re-stamped with its real ISO feature `GA05` "Cast specification"**,
    and **`GE08` reclaimed for its real ISO meaning — "Reference parameters"**
    (ISO/IEC 39075:2024 §17.7 / Annex D Table D.1 row 77). `CAST` was mislabeled
    "CAST operator" and stamped `GE08` on every `ValueExpr::Cast`. `CAST` is
    `<cast specification>` (§20.8), whose optional feature is **`GA05` "Cast
    specification"** (Annex D row 53), not `GE08`. Per ISO Annex A item 52, a
    conforming implementation may not contain a `<cast specification>` without
    `GA05` — `CAST` is **not** baseline — so `CAST` now records `GA05`, and
    because selene-db implements the cast construct, **`GA05` is claimed in
    `SUPPORTED_FEATURES`**. `GE08` is moved out of `SUPPORTED_FEATURES`
    (reference parameters are unimplemented; it is referenced-but-not-claimed
    without a rationale, having no parser surface). `CALL selene.feature_status()`
    now reports `GA05` (supported), not `GE08`, for `CAST`. The `GE08-cast*.gql`
    corpus files were renamed to `cast-*.gql` and re-declared `feature: GA05`.
  - **`GG21` "Explicit element type key label sets" de-stamped** (ISO §18.2/18.3).
    `GG21` requires a `<node/edge type key label set>` — `[ <label set phrase> ]
    <implies>` — but the type-DDL grammar has **no `<implies>` token**: `CREATE
    NODE TYPE :Person (...)` uses an explicit `<node type name>` (that is `GG20`,
    which stays claimed) and the key label set is *implied* from `:Person` per
    §18.2 Syntax Rule 3c, not explicitly specified. With no syntax to express an
    explicit key label set, `GG21` was an over-claim and is removed from
    `SUPPORTED_FEATURES` and the type-DDL flagger. `GG02` (closed graph type) and
    `GG20` (explicit element type names) are unaffected. (Later in this cycle the
    explicit `<implies>` syntax landed and `GG21` was re-claimed as an honest
    singleton — see the `GG21` entry under **Added** above.)
  - **`ExecutorError::FeatureNotInV1_1` renamed to `FeatureNotSupportedYet`**
    and its (and sibling parser/analyzer) user-facing messages degraded to
    version-agnostic phrasing (e.g. "feature not yet supported") so they do not
    go stale across releases. The GQLSTATUS mapping is unchanged (still `42N01`
    `FEATURE_NOT_SUPPORTED`). A CI/pre-commit gate
    (`.github/scripts/check-no-version-locked-feature-error.sh`) forbids the old
    variant name and version-locked "not supported in vX" message strings from
    returning.

### Removed

- **Non-ISO `round(...)` scalar function.** The closed scalar-function runtime
  no longer accepts `round(...)`; ISO §20.22 numeric value functions cover
  `ABS`, `MOD`, trigonometric/logarithmic functions, `EXP`, `POWER`, `SQRT`,
  `FLOOR`, `CEIL`, and `CEILING`, but not `ROUND`.

## [1.1.0] — 2026-05-31

### Removed

- **Vector index extension externalized.** The `selene-vector` and
  `selene-vector-pack` crates — the HNSW and IVF `IndexProvider`
  implementations (with SQ8 / PQ / OPQ quantization) and their `vector.*`
  procedure-pack adapters — were removed from the workspace and moved to a
  separate dedicated project. **BREAKING** change to the public crate
  surface: embedders depending on either crate must repoint to the
  externalized project. The core `selene-graph` query and mutation surfaces
  are unchanged, and the `IndexProvider` boundary is unaffected. (The
  procedure-pack apparatus referenced here was subsequently removed entirely
  — see "Procedure-pack system removed" below.)
- **Planned full-text / BM25 extension dropped.** The previously planned
  `selene-text` / `selene-text-pack` (Tantivy-backed) full-text extension is
  dropped entirely and is no longer a roadmap item. The reserved
  `SELENE_FULLTEXT` extension type id is removed.
- **Procedure-pack system removed — `selene-pack` and `selene-algorithms-pack`
  deleted.** **BREAKING.** selene-db is now a single native graph engine with
  no extension/procedure-pack model. The entire loadable-pack apparatus —
  manifest validation (JSON Schema 2020-12), the typestate-sealed activation
  lifecycle, `ProcedurePackRegistry`, the `ExternalGraphProcedure` /
  `ExternalMutationProcedure` adapter traits, `content_hash`, and the
  `algo.*` pack adapters — was removed. `CALL algo.*` and `CALL selene.*` are
  unchanged at the GQL surface (ISO IW010 external procedures; the grammar is
  not touched) — only the implementation behind them changed (see below).
- **Pack-lifecycle audit removed (greenfield `postcard` break).** **BREAKING.**
  `selene_core::pack_lifecycle::PackLifecycleEvent`,
  `SchemaChange::ProcedurePackLifecycle`, the three legacy
  `SchemaChange::ProcedurePack{Activated,Deprecated,Disabled}` variants, the
  `GraphCommitSink` pack→funnel audit adapter, and the `pack_history` built-in
  are deleted. This changes `Change`/`SchemaChange` postcard discriminants;
  acceptable because there are no shipped consumers (no compatibility shim).
  The dedicated `selene-persist` `audit.log` (`AuditLog` / `AuditRecord` /
  `AuditRetentionPolicy`, D24) and its `with_audit_log` wiring are **retained**
  as the durable substrate for future first-party engine events.
- **Consumerless storage abstractions removed.** **BREAKING** (to the
  `selene-graph` public surface). The `ChangeSubscriber` trait (with
  `SharedGraph::with_change_subscriber`, the runtime `notify_subscribers`
  fan-out, and the recovery subscriber fan-out + subscriber-tag validation)
  and the `StorageCompactor` trait (never wired; zero impls) are deleted —
  both became fully consumerless after the vector and pack removals. The
  in-use CORE-internal densify compaction (`LiveIdSet` / `CompactionReport` /
  `compact_core`) and the `IndexProvider` / `DurableProvider` /
  `RecoveryProvider` plumbing are kept untouched.
- **Workspace dependencies dropped:** `jsonschema`, `schemars`, and `papaya`
  were used only by the deleted pack crates / registry and are removed from
  `Cargo.toml`. `THIRDPARTY.md` regen is deferred to the version bump.

#### Pre-1.1.0 deep-review residue teardown (greenfield; no shipped consumers)

- **`selene-core`:** the dead `codec` module (`Codec` / `CodecError`, ~223 LOC),
  the inert `value_adapter` registry and its `ExtensionTypeId{Conflict,
  Unregistered}` errors (`Value::Extended` + `ExtensionTypeId` kept), the unused
  `ChangeKindSet`, and the `PropertyMap::with_capacity` DoS shape are removed.
  (CORE-02/03/04/10)
- **`Change::IndexExtensionEvent` + `Mutator::extension_event` deleted.**
  `ChangeKind::IndexExtensionEvent` (= 7) is removed and trailing discriminants
  renumber (greenfield `postcard` re-tag; no production WAL/snapshot). The
  durable first-party engine-event channel is the separate `audit.log` (D24).
  (CORE-05/19)
- **Dead `Origin` write-funnel plumbing removed** — the always-`Local` `origin`
  parameter and `_origin` field drop off `Mutator`; `Origin::Replicated` is
  retained (reworded as reserved-for-replication, no format break). (CORE-17/
  GRAPH-27)
- **Dead public/internal surfaces demoted or deleted** across `selene-graph`
  (`as_any`, withdrawn-D23 `LiveIdSet` residue, unconstructed `ProviderError`
  variants, `IdAllocator::from_meta`, `EdgeEndpointDef::node_type_indices`),
  `selene-algorithms` (10 `*_checked` runner helpers collapsed onto
  `*_with_checker(disabled())`), and `selene-gql` (`format_mutate_statement`
  stub, `GqlType::Binary`/`VarBinary`, dead `label_expression` walker, and
  several planner dead-code paths). (GRAPH-24..30, ALGO-17/18, PARSE-02/03/04,
  PLAN-*)
- **Crash-unsafe legacy `WalWriter::rotate` removed** (the pre-MANIFEST rotate
  with a Seam-F crash window; zero callers). The SCMA snapshot section collapses
  its dual V1/V2 decoder to a single hard-reject version, mirroring the GTYP
  clean break. (PERSIST-02, GRAPH-23)

### Chore

- Local test invocation aligned with CI (nextest + line-tables-only debug +
  `.config/nextest.toml`). See CLAUDE.md Build & test.
- **Pre-1.1.0 deep-review test hardening (+~150 tests, 2503 → 2655 workspace).**
  New coverage spans WAL/snapshot/recovery codec round-trips and crash-safety,
  deep-graph algorithm exercises (100K-node line-graph SCC/articulation/bridges
  in a bounded worker stack, concurrent `ensure_fresh` rebuild races),
  mid-execution cancellation past the 1024-row stride, the conformance corpus
  exact-feature-multiset (catching §24.6 over-stamping the `⊆` check missed),
  write-side `FormatError` variant pinning, and per-numeric-variant negate /
  RECORD-equality / set-op conformance. Findings cataloged in the review ledger;
  the deferred large features and bench-gated perf are tracked as grounded
  briefs for v1.2+.

### Changed

#### Pre-1.1.0 deep-review ISO error-class + surface pass

- **Set-operation arms must be column-name-equal.** `UNION` / `INTERSECT` /
  `EXCEPT` arms with mismatched output names are now rejected at lowering with
  `PlannerError::SetOpArmsNotCombinable` (`42001`) per ISO §14.2 SR v, instead
  of silently relabelling to the left arm's names. Both-arms-unnamed stays
  legal. **Behavior-breaking** for queries that previously relied on the
  silent relabel. (GQLRT-31)
- **Removed-construct error classes are now honest.** Dead/donor grammar that
  is no longer offered (`CAST(x AS VECTOR)`, the full-text/time-series DDL
  constraints, hex/octal/binary numeric literals, unsigned-integer suffixes)
  now produces a clean `42001` syntax error rather than a silent `42N01` /
  `5GQL0`; `vector` is again a usable bare identifier. `UNIQUE` constraints
  surface an honest `42N01` deferral (was a silent `5GQL0`). (PARSE-05/06/07/08)
- **`5GQL0` is now reserved for internal-invariant breaks.** ISO-legal but
  unimplemented mutation constructs map to `42N01`; a property access on a
  non-node/edge value maps to `22G03` (span threaded); `5 IS TRUE` (non-boolean
  operand) maps to `22G03`. (GQLRT-13/20/21)
- **`GraphError::IdOverflow` → `RowSpaceExhausted { rows, max_rows }`** — the
  error now names the exhausted row space rather than blaming the id. (GRAPH-14)
- **SHOW rendering fidelity.** Closed RECORD types now render recursively as
  `RECORD { name :: TYPE, ... }`; `render_default_value` hard-errors on an
  unrenderable default instead of emitting `<unsupported-default>`.
  (GQLRT-23/17)
- **`ProcedureRegistry` seam narrowed** to `{Graph, Mutation} × {Read,
  SchemaWrite}` — the pack-era `Persist` / `GraphWrite` / `Admin` / `Capability`
  tiers are deleted. The D16 `&dyn` injection seam and the frozen
  `registry_version() == 0` are untouched. **Breaking** to the `selene-gql`
  registry surface (greenfield; no shipped consumers). (GQLRT-32)

- `selene-gql` planner: `MATCH (n:A|B|C) WHERE ...` flat-disjunctive-label
  patterns now expand to per-label sub-plans wrapped in a new
  `JoinTree::DisjunctiveScan` IR variant when at least one branch has an
  applicable per-label typed/composite/in-list index. Closes the ergonomic
  gap where downstream consumers had to manually construct UNION ALL
  across label families to get index-accelerated lookups. EXPLAIN renders
  the expanded plan transparently (one ScanSnapshot per branch). The
  executor dedups branch outputs by `NodeId` at the `JoinTree::DisjunctiveScan`
  arm, so a node carrying labels A AND B appears exactly once in the
  unioned binding table — matching the unexpanded
  `LabelExpr::Disjunction(any(...))` semantics and preserving the
  catalog-present vs catalog-absent invariant across COUNT / LIMIT /
  aggregates. Edge label disjunction stays Linear (no edge-index rules at
  HEAD; tracked in post-1.0 backlog). The `disjunctive_label_expansion`
  rule lands at slot 5 in `DEFAULT_RULES`. (BRIEF-155 / ariadne #2)
- `selene-gql::plan::ir::JoinTree`: new `DisjunctiveScan { branches:
  Vec<NodeOrEdgeScan>, scan_anchor: NodeOrEdgeScan }` variant. Internal
  IR — only the planner constructs it. `JoinTree` is `#[non_exhaustive]`,
  so in-crate exhaustive matches compile-break; downstream wildcards
  should add an explicit arm (`branches.first_mut()` is a reasonable
  default for "first scan" helpers — selene-testing's `first_scan_mut`
  models this). (BRIEF-155)
- `docs/gql-reference.md`: clarified `LIMIT N` in a composite query
  (`A UNION ALL B`) attaches statically to the syntactic arm it sits
  within — `... B LIMIT N` limits arm B only; arm A runs unlimited. Use
  `CALL { ... } RETURN ... LIMIT N` to limit the union total.
  (BRIEF-155 / ariadne #3)
- `selene-gql::ScanAccess::TypedIndexRange` / `BitmapUnion` /
  `CompositeLookup` now carry `IndexKey { Literal, Parameter }` in their key
  slots (was bare `Literal`). Parameterized equality + range + InList
  predicates now select the typed-index access path at plan time; runtime
  resolves parameter slots at probe time via `resolve_index_key`. Unblocks
  indexed-lookup acceleration for parameterized queries that previously
  fell back to linear scan. `ScanAccess::BitmapUnion` gains a `kind:
  IndexKind` field; `ScanAccess::CompositeLookup.properties` widens to
  `Vec<(IStr, IndexKind)>` (mirrors
  `CompositeIndexHandle.properties`) — both feed the runtime parameter
  resolver's `expected_kind`. NULL-bound parameters return empty rows
  (3VL parity with inline `WHERE n.x = NULL`); ExternalString-bound
  parameters against STRING indexes are coerced via `selene_core::lookup`
  with the BRIEF-153 unpoolable carve-out; wrong-kind and unbound
  parameters error loud (`ExecutorError::InvalidParameterType` /
  `UnboundParameter`, GQLSTATUS `22G03` per ISO §23.1 Table 8). EXPLAIN
  summaries gain a new `[bounds=…]` detail surface across all three
  indexed-scan access paths (literals render with kind tag + value;
  parameters render as `$name`). Additive `ScanSnapshot.bounds_detail:
  Option<String>` field carries the formatted detail without breaking
  existing `#[non_exhaustive]` callers. (BRIEF-154)
- `selene-graph::CompositeIndexValueError` is now `#[non_exhaustive]` and no
  longer derives `Clone/Copy/Eq/PartialEq`. New `ComponentAdmissionFailed
  { index, expected_kind, reason: selene_core::CoreError }` variant carries
  the IStr-pool cap-exceeded source from STRING-component admission
  failures during composite index commit/build. Downstream `match` arms
  must add a wildcard fallback. The single-key
  `selene-graph::TypedIndexValueError` (crate-private) underwent an
  equivalent shape change. (BRIEF-153)
- `selene-graph::GraphError`: new `IndexAdmissionExhausted { label,
  property, source: CoreError }` variant (diagnostic code `SLENE_G_018`)
  for IStr-pool admission failures at the property-index commit boundary.
  Maps to GQLSTATUS `5GQL1`. The enum is already `#[non_exhaustive]`, so
  this addition is semver-safe. (BRIEF-153)
- `selene-graph::CompositeTypedIndex::key_from_values` is replaced by
  `key_from_values_admit` (write/maintenance path; admits new
  `Value::ExternalString` content into the global `IStr` pool when the
  component kind is `STRING`) and `key_from_values_lookup` (read path;
  returns `Ok(None)` instead of admitting). The GQL `composite_lookup_rows`
  and the `selene.verify` builtin now use the lookup variant to close a
  read-path admission DoS. (BRIEF-153)
- `Value::ExternalString` may now be admitted to the global `IStr` pool
  **only at the property-index commit boundary** when the column is
  declared `INDEXED` (single or composite, `STRING` kind). The stored
  property value remains the `ExternalString` variant; only the secondary
  index key space sees the admitted handle. DDL `INDEXED` is the user's
  consent for this carve-out from variant-strict storage; cap exhaustion
  now surfaces as a hard `IndexAdmissionExhausted` error instead of a
  silent skip. (BRIEF-153)
- ISO GQL error-code conformance: remapped `GqlStatus` constants and
  `miette` diagnostic tags to GQLSTATUS values per ISO/IEC 39075:2024
  section 23.1 Table 8. Public-facing `as_str()` output changes for 9
  status constants (for example, `42601 -> 42001`, `42883 -> 22G03`,
  `0A000 -> 42N01`). Added `GqlStatus::IN_FAILED_TRANSACTION` (`25N02`)
  and `GqlStatus::class()`. No source changes are required for downstream
  consumers using `GqlStatus::SYNTAX_ERROR` and related constants by name;
  consumers comparing raw 5-character codes must update strings.
- Program-limit errors from `IStrCapExceeded`, `PayloadTooLarge`,
  `TooManySections`, and `SectionTooLarge` now report GQLSTATUS `5GQL1`
  instead of the SQLSTATE-shaped `54000`.
- Removed phantom Pattern alpha support claims for path selectors
  (`G015`-`G018`) and graph management (`GC04`/`GC05`); these surfaces now
  reject with GQLSTATUS `42N01` before execution instead of reaching planner
  or runtime implementation-defined failures.
- Collapsed residual implementation-defined `XX500`/`XX501`/`XX502` status
  emissions to single GQLSTATUS `5GQL0`, with diagnostic-detail tags for graph
  mutation, durability flush, and generic implementation-defined failures.
  Residual `22023` emissions in core, graph, and persist now report `22G03`.
- GQL record equality now propagates NULL and NaN through nested record fields
  in the runtime `=` comparator while preserving `Value::PartialEq` and
  runtime row-key structural equality for deduplication.
- Runtime data exceptions now carry a `DataExceptionSubclass` selected at the
  emission site. Arithmetic overflow reports `22003`, division by zero
  reports `22012`, invalid power arguments report `2201F`, invalid value-type
  paths report `22G03`, incomparable ordering reports `22G04`, and record /
  graph-property subclasses use `22G0X`, `22G0M`, `22G0S`, and `22G0T` where
  live paths exist. GQLSTATUS is read from `gqlstatus()`; the stable miette
  diagnostic tag remains broad for the parent data-exception variant.
- Transaction and graph-type surfaces now emit live ISO Table 8 classes:
  nested `START TRANSACTION` returns `25G01`, write attempts in read-only
  transaction contexts return `25G03`, invalid transaction termination returns
  `2D000`, bare node delete with incident edges returns `G1001`, and closed
  graph schema/type violations return `G2000`.

### Added

- **Single per-graph committer thread + `WriteTxn::seal()` (v1.2 multi-writer,
  BRIEF 1).** Commit now splits into a session-thread `seal()` (generation/meta
  bump + GG02 validation under the write lock, then **lock release**) and a
  durable+publish tail that runs on one dedicated committer thread per graph —
  the **sole writer** of the published-snapshot `ArcSwap` cell. The public
  `commit()` contract is unchanged ("returns ⇒ durable + visible"); only the
  internal threading model differs. Every snapshot publisher (autocommit,
  explicit-txn terminal COMMIT, index DDL, and compaction) routes through the
  committer. Publish order is kept equal to seal order — and thus to
  lock-acquisition order — by stamping each publishable unit with a
  strictly-monotonic `seal_seq` under the write lock and publishing strictly in
  `seal_seq` order via a reorder buffer; this is a new, load-bearing,
  **not** type-enforced D10 invariant (a second committer or second `ArcSwap`
  writer would silently break strict-serializability). Compaction builds its
  dense graph on the caller thread under the lock (seal-and-handover) so the
  committer never holds the write lock. A post-seal durable failure or a
  committer-body panic **poisons** the engine (reopen-required; the durable WAL
  never received the failed entry, so recovery heals it) rather than leaving the
  live in-memory graph diverged from the published snapshot. The BRIEF-117
  cancellation cut-line is sampled inside `seal()` under the lock, so a cancel
  rolls back via `Drop` exactly like an aborted transaction (no trace, no
  poison). Durability-neutral: the WAL stays in `SyncPolicy::EveryN(1)` (WAL
  group commit lands in BRIEF 2).
- **WAL group commit — R1 fsync-before-publish barrier + `CommitBatching`
  (v1.2 multi-writer, BRIEF 2).** The committer's per-commit durable+publish
  tail is split into Stage 1 **append** (`write_commit` with fsync deferred),
  Stage 2 **flush** (one `flush_durables` per drained run — the single group
  fsync), and Stage 3+4 **publish** (`publish_appended`, now **infallible** —
  returns `CommitOutcome`, not `Result`, structurally foreclosing the
  "returns-`Err`-but-already-published" inversion). The committer forms a
  contiguous-`seal_seq` run of appended commits, fsyncs the whole run **once**,
  then publishes + acks each member in `seal_seq` order, so neither the
  published snapshot nor the acked `durable_at` is observable before fsync
  (durable-before-visible holds for the whole batch). New embedder knob
  `SharedGraphBuilder::with_commit_batching(CommitBatching)`:
  `CommitBatching::Off` (the default) caps each run at one commit — one append +
  one fsync + one publish + one ack, behaviorally **identical** to BRIEF 1's
  `EveryN(1)`; `CommitBatching::On { max_commits, max_bytes }` (and the
  conventional `CommitBatching::DEFAULT_ON` = 64 commits / 8 MiB) coalesces a
  contiguous run into one group fsync for higher write throughput and lower tail
  latency under concurrent fan-in. A `CALL`-issued compaction stays a hard flush
  boundary (never co-batched — its dense snapshot already embeds every
  lower-`seal_seq` commit's mutation, so the pending run is flushed + published
  before the dense store), and a flush failure / partial-append failure /
  publish panic poisons the engine and error-acks every in-flight run member
  (no silently-dropped reply channel, no `recv()` hang). **Behavioral note:**
  because the committer is now the sole fsync caller for the committer-managed
  WAL, `SharedGraphBuilder::with_wal` / `SharedGraph::from_graph_with_wal` /
  recovery **force the WAL into `SyncPolicy::OnFlushOnly`**, discarding any
  caller `WalConfig::sync_policy`; fsync cadence is set by `CommitBatching`
  instead. A `WalWriter` opened directly (outside `selene-graph`) still honors
  the caller's policy. D10 strict-serializability is preserved (single committer,
  sole `ArcSwap` writer, `seal_seq`-ordered publishing).
- **`selene-algorithms` is now a mandatory first-class crate with a native
  Rust API.** **BREAKING** (promotes a previously opt-in crate; `selene-gql`
  now build-depends on it). Every algorithm is callable directly from Rust
  with no registry/`CALL` machinery: free functions (e.g. `pagerank_on(...)`,
  with `*_with_checker` variants for cancellation) plus a `GraphAlgorithms`
  Rust extension trait for an ergonomic methods-on-graph surface. The
  1024-thread `Parallelism` cap moved into `selene-algorithms` (was
  adapter-side). No algorithm bodies changed.
- **Native `BuiltinProcedureRegistry` in `selene-gql`.** The sole frozen
  production `ProcedureRegistry` impl: it registers the 19 `algo.*` procedures
  (binding `CALL algo.*` directly over the native algorithms API) and the
  5 platform built-ins relocated native from the deleted pack crate —
  `selene.health`, `selene.feature_status`, `selene.verify`,
  `selene.create_index`, `selene.drop_index` (the index DDL still routes
  through the `Mutator` funnel). Argument coercion (Int→f64, NULL→default,
  arity-from-trailing-nullable) is ported verbatim. `registry_version()` is a
  constant `0` (frozen at construction) so the CALL plan cache key stays
  stable. The `ProcedureRegistry` trait is kept as the plan/execute seam.
- **Dedicated `audit.log` for engine-owned events**, with retention independent
  of the WAL + snapshot lineage (BRIEF-Item-7, deletion+reclamation audit Item 7
  / Seam D, D24). An append-only `audit.log` (`SLAU`) that retention prunes
  separately from the WAL/snapshot lineage, so an engine event survives even when
  an embedder prunes WAL archives ([D26](#)). New selene-persist API: `AuditLog`
  (`open` / `append` / `read_all` / `prune`, with a torn-tail-truncating
  scan-on-open mirroring the WAL's crash recovery), `AuditRecord`,
  `AuditRetentionPolicy { keep_n_events, max_age }` (conjunctive; default
  unbounded — events are sparse; prune is an atomic read-filter-rewrite).
  Records are generic `kind`-tagged opaque payloads with a caller-supplied
  wall-clock stamp, so `selene-persist` stays below lifecycle semantics and the
  system clock. Wire it up via `SharedGraphBuilder::with_audit_log` (requires
  `with_wal`); engine events are mirrored **WAL-first, audit-after**
  (the WAL append gates the commit; the audit write is best-effort and the event
  also stays in the WAL, so a failed mirror degrades to WAL-only rather than
  losing data — "audit lag is recoverable, fiction is not"). Recovery reattaches
  the audit log when the file is present, so post-recovery commits keep
  mirroring. **Scoped surgically:** the D12 per-commit principal stays in the WAL
  entry header (no WAL-format break, no hot-path write-amplification); only
  engine-owned events move to `audit.log`. The substrate is retained as the
  durable channel reserved for future first-party engine events.
  `selene_persist::prune` (D26) never touches `audit.log`.
- **Snapshot + WAL-archive retention** via a typed `RetentionPolicy` and a
  MANIFEST-atomic `prune` (BRIEF-Item-5, deletion+reclamation audit Item 5 /
  D26). `selene_persist::prune(dir, &policy)` (and the `WalWriter::prune`
  wrapper) reclaim superseded `snapshot.{seq}.snap` + `wal.{seq}.archive` files
  that a v1.0 directory accumulated without bound. `RetentionPolicy { keep_n_snapshots,
  keep_n_wal_archives, max_total_size_bytes: Option<u64>, time_based: Option<Duration> }`
  is **embedder configuration** (deliberately *not* persisted in the MANIFEST —
  a stored policy would be a divergent second source of truth); the four
  constraints are conjunctive; defaults keep 2 snapshots / 4 archives with no
  size or time cap. The load-bearing safety floor: the live snapshot
  (`live_snapshot_seq`) and the active WAL are **never** deletable regardless of
  policy — even `keep_n_snapshots = 0` retains the live snapshot. Prune is
  crash-safe by the MANIFEST commit-point invariant — it rewrites the MANIFEST
  (shrinking `archived_wal_seqs`, the linearization point) *before* deleting any
  file, so a crash mid-prune leaves orphans that recovery already ignores and
  the next prune reclaims; the committed MANIFEST is never observed referencing
  a deleted archive. Archive selection is MANIFEST-authoritative (the rewritten
  list only ever shrinks, never adopting a crash-orphan), while superseded
  orphan archives (`seq < live`) and orphan snapshots (`seq > live`) are
  reclaimed. New public API: `RetentionPolicy`, `PruneOutcome`, `prune`,
  `WalWriter::prune`, `parse_wal_archive_filename`, `WAL_ARCHIVE_PREFIX` /
  `WAL_ARCHIVE_SUFFIX`, `DEFAULT_KEEP_SNAPSHOTS` / `DEFAULT_KEEP_WAL_ARCHIVES`.
  No format change (the MANIFEST `retention_present` byte stays reserved).
- `DROP GRAPH` is now executable as a **factory-reset** of the single (D1)
  session graph, replacing the v1.0 `ImplementationDefined` stub (BRIEF-152,
  deletion+reclamation audit Item 10). It wipes **every** node and edge —
  including untyped / arbitrary-label rows (enumerated from the live-row
  bitmaps via `SeleneGraph::live_nodes`/`live_edges`, so a per-type truncate
  cannot miss them) — and resets the schema to open (`bound_type → None`),
  turning a previously closed (GG02) graph back into an open (GG01) one.
  Recorded as exactly **one** declarative `Change::GraphReset` regardless of
  instance count (O(1) WAL), while per-row `NodeDeleted`/`EdgeDeleted`
  tombstones are still produced on both the runtime and the recovery-replay
  paths so index providers stay consistent. Recovery re-derives the wiped rows from the recovered
  store and forces the graph open — a `recover_closed(bound_type)` after a
  reset reconstructs the identical empty+open state. The MANIFEST epoch and WAL
  archive lineage are untouched (this is one committed WAL entry, not a
  file-level wipe); `DROP GRAPH` is idempotent. Flagged as the `IM_DROP_GRAPH`
  vendor extension; the parsed graph name is informational under D1 and
  `IF EXISTS` is trivially satisfied. `CREATE GRAPH` remains rejected (D1
  cannot create a second graph). New `Change::GraphReset` variant +
  `ChangeKind` discriminant (postcard tags appended; pre-existing tags stable).
- `DROP NODE TYPE` / `DROP EDGE TYPE` now take an optional `RESTRICT | CASCADE`
  behavior tail (BRIEF-151, deletion+reclamation audit Item 3, Seam B).
  `RESTRICT` is the default when no keyword is written and is the Seam-B fix:
  dropping a type whose instances still exist (or, for a node type, whose
  instances are still referenced by an edge type's endpoint) is now rejected
  EARLY at the drop op with `G2000` `GRAPH_TYPE_VIOLATION` and drops NOTHING —
  no orphan instances, no dangling edges, the bound graph type left fully
  intact. Previously the drop emitted `SchemaChange::NodeTypeDropped` without
  scanning instances and only failed late at commit with a mislabelled
  `UnknownNodeLabel`. `CASCADE` is the `IM_DROP_CASCADE` vendor extension: it
  truncates the type's instances first (reusing the BRIEF-150
  `Mutator::truncate_node_type` / `truncate_edge_type` funnel, so incident edges
  are removed and there is one O(1) declarative truncate `Change`), then drops
  the type — both the truncate `Change` and the `SchemaChange` land in the same
  committed transaction, so commit and WAL replay are atomic (a failure rolls
  back both). Node-CASCADE of a type still index-referenced by a surviving edge
  type is rejected (recursive type cascade is out of scope). `CASCADE` flags
  `IM_DROP_CASCADE` at parse time (GQL Flagger clause 24.6); `RESTRICT` and the
  default carry only the existing `GG02`/`GG20`/`GG21` type-DDL flags.
  `selene-graph::DropBehavior` parameterizes the single write funnel so no I/O
  surface can bypass the policy; `selene-gql::DropBehavior` is its AST mirror.
- `IM_TRUNCATE` vendor extension: `TRUNCATE NODE TYPE :L` and
  `TRUNCATE EDGE TYPE :L` declarative bulk delete (BRIEF-150, deletion+
  reclamation audit Item 11). A truncate is observationally identical to
  `MATCH (n:L) DETACH DELETE n` — same final graph state, incident edges of
  every type cascaded, no dangling edges, and the same per-row tombstones so
  index providers stay consistent. The crucial difference is the WAL: exactly ONE
  declarative `Change::NodesOfTypeTruncated { label }` /
  `Change::EdgesOfTypeTruncated { label }` is written regardless of the number
  of instances removed (O(1) WAL). Recovery re-derives the affected rows by
  walking the recovered store and expands them to the same per-row tombstones,
  so tombstoning is byte-identical on the runtime and recovery
  paths. New `Mutator::truncate_node_type` / `truncate_edge_type` route through
  the single write funnel (reusable by future `DROP NODE TYPE CASCADE` /
  `DROP GRAPH`). Two new `ChangeKind` discriminants (`NodesOfTypeTruncated`=11,
  `EdgesOfTypeTruncated`=12). The GQL
  Flagger stamps `FeatureId::IM_TRUNCATE` on every truncate statement
  (clause 24.6); `IM_TRUNCATE` is registered in `SUPPORTED_FEATURES`. TRUNCATE
  is valid on both open (GG01) and closed (GG02) graphs — it removes instances
  and keeps the bound type. An absent label is a clean no-op; double-truncate
  is idempotent. (Pre-v1.x postcard tag append — audit line 329 sanctions the
  format break; no snapshot archive bump.)
- `selene-persist` MANIFEST epoch descriptor + crash-safe multi-phase rotate +
  three-step recovery (BRIEF-148, deletion+reclamation audit Item 2 / Seam F).
  A small fixed-layout `MANIFEST` file (`SLMF` magic, format_version 1, LE
  fields, trailing blake3-low-128 body hash; `Manifest::{encode,decode,
  write_atomic,read}` + `sync_dir`) names the single live persistence epoch and
  is rewritten atomically each rotation. `WalWriter::rotate_with_manifest`
  replaces the embedder's two-call snapshot-finalize-then-`rotate` sequence with
  a four-phase rotation whose MANIFEST rename is the single linearization /
  commit point: Phase 1 publishes the snapshot, Phase 2 archives the WAL (both
  non-destructive, with parent-directory fsync after each publish), Phase 3
  commits the MANIFEST, and Phase 4 resets the active WAL — strictly after the
  commit, so a crash never produces the "MANIFEST names N-1 but wal.log already
  reset to N" data-loss state. `recover()` now reads the MANIFEST first
  (`RecoveryOutcome::manifest_present`): when present it is authoritative
  (snapshot opened by `live_snapshot_seq` directly, orphan snapshots ignored,
  the Seam-F `WalSnapshotMismatch` cross-check relaxed), and when absent it
  falls back to the legacy `find_latest_snapshot` path with the cross-check
  intact so pre-MANIFEST directories recover identically and migrate forward on
  the next rotate. The exact crash-race that previously hard-failed
  (`WalSnapshotMismatch`) now auto-reconciles. Parent-directory fsync (D6
  durability ordering, previously deferred) is folded in across all three
  publish points. D15 (two-step recovery) becomes three-step.
- ISO/IEC 39075:2024 §7 session management (the D1-meaningful subset) in
  `selene-gql`: `SESSION SET VALUE [IF NOT EXISTS] $p = <value-spec>` (GS03),
  `SESSION SET TIME ZONE '<zone>'` (GS15), `SESSION RESET [ALL]
  [CHARACTERISTICS|PARAMETERS]` / `RESET PARAMETER $p` / `RESET TIME ZONE`
  (GS04/GS07/GS08/GS16), and `SESSION CLOSE` (§7.3). `SET TIME ZONE` threads
  the zone into a new §20.27 current-datetime family
  (`current_timestamp`/`localtimestamp`/`current_date`/`current_time`/
  `localtime`); the default is UTC (ID048) and `RESET` restores it. `SESSION
  CLOSE` sets a termination flag (rolling back any active transaction) that is
  enforced at the single statement funnel — both `Session::execute_source`
  and the cached-plan `execute_statement` entry reject post-close requests
  with GQLSTATUS `2DN01`. Invalid time zones raise `22009`. Default session
  parameters are the empty set (ID049). Catalog-dependent forms (SET
  GRAPH/SCHEMA/BINDING TABLE — GS01/GS02 — and GS10–GS14) are deferred under
  D1 (single-graph embeddable): they are absent from `SUPPORTED_FEATURES`,
  parse-fail cleanly, and each carries a `NOT_SUPPORTED_RATIONALE` entry. The
  Flagger stamps the GS feature family, co-stamping GS08+GS16 for `RESET
  PARAMETER <name>` per §7.2 CR6/CR7. (BRIEF-136)
- Dedicated `Change::NodePropertyRemoved`, `Change::EdgePropertyRemoved`, and
  `Change::NodeLabelRemoved` variants. GQL `REMOVE` now emits these variants
  instead of drop-only `NodeUpdated`/`EdgeUpdated` diffs, while direct mutator
  update APIs retain their existing diff contract. This preserves postcard tag
  stability by appending the variants.
- Vendor `IM_TYPED_PARAMS` inline typed parameter declarations in
  `selene-gql`: `$id :: TYPE` is now parsed at expression and LIMIT/OFFSET
  parameter sites, typed by the analyzer, validated against bound session
  values at runtime, formatted into CALL cache canonicalization, and attributed
  by the Flagger. This closes the BRIEF-115 declared-parameter scope cut as a
  selene-db extension; ISO §22.1 external typed bindings remain the standard
  contract for future strict-ISO work. Public AST/IR note:
  `LimitValue::Parameter` changed from tuple to struct shape and
  `#[non_exhaustive]` was added to both `LimitValue` and `LimitAmount`.
- Composite-property node indexes across `selene-graph`, `selene-core`,
  `selene-persist`, and `selene-gql`: `CREATE INDEX <name> ON
  :Label(a, b, c)` now registers durable tuple indexes, maintains them across
  node create/update/delete commits, recovers them from WAL plus CORE/CPIX
  snapshots, and wires optimizer composite lookups to storage-backed execution.
  Query-planner composite lookup substrate
  was already present; edge-property and SHOW INDEX aggregation remain split
  into BRIEF-140c/140d.
- Implementation-defined named index DDL in `selene-gql`: `CREATE INDEX <name>
  ON :Label(property)` and `DROP INDEX <name>` now execute for single-property
  node indexes, infer the storage index kind from the declared property type,
  resolve drops by catalog name, enforce DDL-level name uniqueness, and record
  the vendor `IM_INDEX_DDL` feature ID. Composite-property indexes, edge-property
  indexes, and `SHOW INDEXES` provider aggregation remain split into BRIEF-140b
  through BRIEF-140d.
- Implementation-defined `EXTENDS` property composition for `CREATE NODE TYPE`
  and `CREATE EDGE TYPE` in `selene-gql`. Parent properties are flattened at
  CREATE time, same-kind parents are required, exact-match redeclarations
  succeed without duplication, mismatches raise `GraphTypeViolation`, and
  feature attribution records the vendor `IM_EXTENDS` feature ID.
- ISO/IEC 39075:2024 cluster-R string and aggregate conformance in
  `selene-gql`: §19.7 `IS [NOT] NORMALIZED`, §20.24 `NORMALIZE`, `LEFT`,
  `RIGHT`, multi-character TRIM family (GF05), explicit TRIM syntax (GF06),
  GF10 attribution for ISO `STDDEV_POP` / `STDDEV_SAMP` / `COLLECT_LIST`, and
  GF11 `PERCENTILE_CONT` / `PERCENTILE_DISC` binary aggregates. New substring
  and trim data exceptions emit ISO Table 8 `22011` and `22027`; string
  results preserve the `ExternalString` DoS invariant. SQL-only drift
  (`REPLACE`, `LPAD`, `RPAD`, `POSITION`, `SIGN`, `RAND`) remains out of
  scope.
- Implementation-defined first-class UUID support in `selene-gql`: `UUID`
  type names and literals, `uuid_v4()`, `uuid_v7()`, `uuid('<string>')`,
  `CAST(... AS UUID)`, UUID typed-index routing, and `CAST(UUID AS STRING)`
  rendering through `ExternalString`. Feature attribution records the vendor
  `IM_UUID` feature ID.
- ISO/IEC 39075:2024 §20.10 `ELEMENT_ID` (G100), §20.22 `CARDINALITY`
  (GF12), and minimum-conformance `CHAR_LENGTH` / `CHARACTER_LENGTH` scalar
  functions in `selene-gql`. `ELEMENT_ID` returns `ExternalString` node/edge
  IDs; `CARDINALITY` covers binding-table references, paths, lists, and
  records; `CHAR_LENGTH` aliases the existing Unicode scalar-counting
  `LENGTH` implementation. Session parameters now support table bindings
  through `Session::bind_table_parameter(...)` for binding-table-reference
  tests and procedure output handoff.
- ISO/IEC 39075:2024 §20.22 numeric value function clusters in `selene-gql`:
  GF01 enhanced numeric feature registration for `ABS`, `MOD`, `FLOOR`,
  `CEIL`/`CEILING`, and `SQRT`; GF02 trigonometric functions (`SIN`, `COS`,
  `TAN`, `COT`, `SINH`, `COSH`, `TANH`, `ASIN`, `ACOS`, `ATAN`, `DEGREES`,
  `RADIANS`); and GF03 logarithmic functions (`LN`, `LOG`, `LOG10`, `EXP`,
  plus `POWER` feature claiming). `LN(<=0)` now emits GQLSTATUS `2201E`
  invalid-argument-for-natural-logarithm.
- Native ISO RECORD types end-to-end (JSON/L1c + L1c-d), per ISO/IEC
  39075:2024 §18.9 `<record type>` / §18.10 `<field type>`. Closed/typed
  `RECORD { a :: INT, b :: STRING }`, open/bare `RECORD` / `ANY RECORD`, and
  nested record types parse, flag, and persist; features `GV45` (record
  types), `GV46` (closed), `GV47` (open), and `GV48` (nested) are registered
  supported and fire through the §24.6 Flagger on every record type position.
  The catalog persists record field-type structure on **both** type-models
  (D14 / D19): the rkyv `RecordFieldTypes` descriptor in `CORE/GTYP`
  (authoritative; `CORE/GTYP` collapsed to a single `GTYP_VERSION = 1`), and
  the serde `RecordFieldStructure` on the WAL `PropertyDef.record_fields`
  (recovery reconstructs the closed-record catalog faithfully). Commit-time
  closed-graph (GG02) record-property validation enforces ISO §4.15.4
  field-name-set equality (no extra, no missing-required) and raises `G2000`
  on violation. In expression context: `x IS TYPED RECORD{…}` performs
  name-keyed §4.15.4 conformance (two-valued — `NULL IS TYPED <T>` is `false`,
  not unknown), and `CAST(x AS RECORD{…})` performs per-field recursive
  coercion with §20.8 SR12 subset projection (extra source fields dropped),
  raising the new GQLSTATUS `22G0U` (record-fields-do-not-match) when a target
  field is absent and `22G03` (datatype-mismatch) for a non-record source or a
  record→scalar cast. Record field names are charged against the per-parse
  interner budget. **Known limitation (deliberately deferred):** name-keyed
  `IS TYPED` / `CAST` for a *catalog-bound* `Value::RecordTyped` operand is
  fail-closed (`IS TYPED` → `false`; `CAST` → `42N01`). `RecordTyped` carries a
  `type_id` plus positional slots with no inline field names, so ISO-mandated
  name-keyed conformance requires resolving `type_id` through a named-record-
  type catalog (`GraphTypeDef.record_types`) that is not yet built — and no
  production read path materializes `Value::RecordTyped` today (records surface
  as `Record::Open`, handled name-keyed above), so the fail-closed arms are
  unreachable by any user query. Positional matching is intentionally **not**
  done (it would be silent non-conformance). Name-keyed `RecordTyped` is
  deferred to a future named-record-type catalog + `RecordTyped` read-path
  producer brief.
- Explicit `CAST(<expr> AS <type>)` expressions in `selene-gql` per ISO/IEC
  39075:2024 §22. New `ValueExpr::Cast { value, target_type, span }` AST
  variant; feature `GE08` (CAST operator) is registered as supported. The
  runtime dispatch matrix covers numeric ↔ numeric (Integer/Float), string
  ↔ numeric (strict parse), boolean ↔ string (lowercase only), boolean ↔
  integer (0/1), and `LIST<T>` element-wise casts. NULL → ANY returns NULL
  per the §22 universal rule. Failure modes emit `22018`
  (invalid-character-value-for-cast — new in the Table 8 map), `22003`
  (numeric overflow), `22000` (boolean domain), or `42N01` for
  intentionally unsupported source/target combinations (NODE / EDGE / PATH
  sources, and catalog-bound `RecordTyped` record sources; `NULL` /
  `NOTHING` targets — open-record `CAST(… AS RECORD{…})` is supported, see
  the native-record-types entry). The analyzer's
  `bind_value_expr` recursive walker now grows the stack via
  `stacker::maybe_grow(64K, 1MB)` so future `ValueExpr` variant additions
  cannot reach into the 2 MB default macOS pthread stack at the depth-256
  contract enforced by `check_expr_depth`.
- `EdgeEndpointDef::OneOf` for polymorphic-enumerated edge endpoints in
  `selene-graph` (storage `Vec<u32>`) and `selene-core` (WAL
  `SmallVec<[NodeTypeRef; 4]>`). Catalog DDL of the form
  `CREATE EDGE TYPE :E (FROM :A, :B TO :C)` now resolves the multi-label
  endpoint to `OneOf([idx_A, idx_B])` when each label is a distinct single-
  label node type. The `EdgeEndpointDef::one_of` constructor sorts, dedupes,
  and collapses singletons to `NodeType` on both type models;
  `GraphTypeDef::validate_ref` rejects malformed OneOf payloads (singleton,
  unsorted, duplicated, out-of-range) as defense in depth for rkyv/serde
  decode paths. SHOW EDGE TYPES renders OneOf endpoints as comma-joined
  member labels (round-trippable through `parse()` + re-execute). GTYP V3
  and WAL `EdgeTypeAddedV2` extend in place (both dev-line, born post
  v1.0.0); legacy V1 / V2 GTYP and `EdgeTypeDefV1` WAL payloads remain
  OneOf-blind by design and decode unchanged. Closes Mnemosyne M0-T04
  blocker (U12).
- Non-leading `MATCH` clauses now lower as sequential binding-table extensions
  in `selene-gql`, covering cross-product and correlated continuation shapes.
- Scalar `VALUE { ... }` subqueries in `selene-gql`, including correlated
  read-query bodies, static ISO §20.6 shape checks, and empty-result `NULL`.
- Inline `CALL { ... }` table subqueries in `selene-gql`, including implicit
  variable-scope correlation, `YIELD`/`YIELD ... AS`, and per-row Cartesian
  result composition.
- Bounded variable-length edge patterns in `selene-gql` for `WALK` matches,
  including `JoinTree::Repeat`, `LIST<EdgeRef>` group-variable binding, zero-hop
  results, per-hop edge predicates, and cancellation checks.
- `MATCH` path selectors in `selene-gql` for `ALL`, `ANY`, `ALL SHORTEST`,
  and `ANY SHORTEST` over fixed and bounded variable-length path patterns.
- Restrictive `MATCH` path modes in `selene-gql` for explicit `WALK`, `TRAIL`,
  `SIMPLE`, and `ACYCLIC`, including ordered path contributors for fixed and
  bounded variable-length path validation.
- Wave 3 variable-length relationships are complete: unbounded edge
  quantifiers (`*`, `+`, `{m,}`), questioned edges (`?`), and the ISO §16.4
  legality matrix now execute in `selene-gql` with hard `max_quantifier`
  backstops.
- Shared `selene-gql` `CallPlanCache` for repeated top-level procedure
  `CALL` statements across short-lived sessions, with graph-id and
  schema-version keyed invalidation plus a cache-hit metric.
- Feature-gated `metrics` facade with query, commit, persistence, recovery,
  cancellation, algorithm, and graph-size metrics.
- `EXCEPT`, `EXCEPT ALL`, `INTERSECT`, `INTERSECT ALL`, and `OTHERWISE`
  set-operation runtime support per ISO GQL §14, including `RuntimeEqKey`
  grouping semantics and a configurable implementation-defined set-op key cap.
- `StatementOutput::Written` write metadata for committed GQL catalog and
  data mutations, including optional rows for write statements with `RETURN`.
- `Session::flush()` and `DurableProvider::flush()` for explicit durability
  barriers over commit-critical providers.
- Live `CoreProvider` WAL writes through `SharedGraphBuilder::with_wal(...)`
  and `SharedGraph::from_graph_with_wal(...)`.
- `parse_many(...)` for semicolon-separated multi-statement GQL scripts.
- `Session::start_transaction`, `Session::commit_transaction`, and
  `Session::rollback_transaction` for explicit transaction control without
  parser round-trips. Returns `TransactionOutcome` / `RollbackOutcome` with
  changes, durable_at, statement_count, and duration metadata.
- `write_e2e` benches for explicit-transaction Rust API commit and rollback
  paths.
- `selene-graph` runtime schema-version epoch for committed schema changes,
  exposed through `SharedGraph::schema_version()`.
- `selene-gql` opt-in Session plan caching through
  `Session::with_plan_cache(...)` and `Session::execute_source(...)`. The
  cache invalidates on schema-version bumps and skips CALL plans until
  procedure registries expose an epoch.
- `write_e2e/gql_insert_single_node_cached` and
  `write_e2e/gql_insert_single_node_cached_with_schema_churn` benches for the
  Session plan-cache hot path.
- **Parameter binding** through `Session::bind_parameter(name, value)`,
  `Session::clear_parameter(name)`, and `Session::clear_parameters()`.
  `$name` references are resolved from the session parameter map end-to-end;
  unreferenced parameters are ignored, runtime type mismatches are strict
  `ExecutorError::InvalidParameterType` errors with GQLSTATUS 22G03, and typed
  declarations such as `$id :: INTEGER` remain a v1.2 follow-up.
- **Evaluator completeness (16/18)**: `runtime/evaluator/` now executes `LIKE`,
  `BETWEEN`, `IS` checks (7 sub-kinds), `CASE`, 0-indexed list access, record
  literals, `PROPERTY_EXISTS`, `ALL_DIFFERENT`, `SAME`, a closed
  case-insensitive scalar function list of 15 functions, and binary `POWER`,
  `XOR`, concat, `CONTAINS`, `STARTS WITH`, and `ENDS WITH`. `IS NORMALIZED`
  returns `ExecutorError::FeatureNotInV1_1` with GQLSTATUS 42N01 pending v1.2
  Unicode normalization work; `Exists` and `CountSubquery` are deferred to
  BRIEF-116b.
- Evaluator now executes `EXISTS { MATCH ... }` and `COUNT { MATCH ... }`
  subqueries (ISO §19.4 EXISTS predicate; COUNT subquery is a selene-db
  dialect extension). 18/18 v1.0 evaluator stubs now closed (16 by BRIEF-116;
  2 by BRIEF-116b). Outer-binding references in subquery patterns are resolved
  at lowering and seeded at evaluation.
- `ExecutorError::{UnknownFunction, FunctionArityMismatch,
  InvalidFunctionModifier, FeatureNotInV1_1}` for scalar dispatch and scoped
  v1.1 evaluator feature errors.
- `runtime/evaluator/` submodule layout split into `mod.rs`, `binary_ops.rs`,
  `predicates.rs`, `scalar_fns.rs`, `case.rs`, and `collections.rs`.
- Cooperative cancellation and per-statement resource limits:
  `CancellationToken`, `Session::with_cancellation_token(...)`,
  `Session::with_deadline(Instant)`, and `Session::with_row_cap(usize)`.
  Cancellation is cooperative across executor pipeline checkpoints and
  algorithm hot loops. Row caps apply only to outermost statement output rows.
  New `ExecutorError::{Cancelled, Timeout, RowCapExceeded}` map to
  implementation-defined GQLSTATUS codes `5GQL2`, `5GQL3`, and reused
  `5GQL1`, respectively.
- GQL surface completeness: `SHOW INDEXES` now lists built-in property indexes,
  `SHOW PROCEDURES` lists registered procedures, `EXPLAIN <statement>` returns
  an indented plan dump without executing the inner statement, and
  `selene.feature_status` is a registered platform built-in. The analyzer now
  bounds value-expression recursion at depth 256 with
  `AnalysisError::RecursionLimitExceeded` (GQLSTATUS `5GQL1`). Named
  `CREATE INDEX` / `DROP INDEX` DDL lowering remains deferred pending
  storage-layer named-index support.
- Catalog type declarations now persist and enforce `DEFAULT`, `IMMUTABLE`,
  and `STRICT` / `WARN` validation modes. CORE/GTYP snapshots and schema WAL
  changes use additive v2 encodings while preserving legacy recovery; relaxed
  writes emit implementation-defined warning GQLSTATUS `01N01`.
- `IStrAdmissionPolicy` and `Session::with_istr_admission_policy(...)` let
  embedders opt into graceful interner-cap fallback at runtime string-admission
  boundaries. The default remains `Reject`; `FallbackToExternal` carries
  eligible over-cap text as `Value::ExternalString`.
- Procedure metadata now includes procedure-scope descriptions, signature
  `since_version`, parameter descriptions and `default_doc`, and output-column
  descriptions. `SHOW PROCEDURES` now returns seven columns:
  `name`, `tier`, `mutability`, `signature`, `description`, `since_version`,
  and `capability_required`.
- Conformance-integrity coverage for v1.1: Pattern beta ISO feature IDs
  `GQ08`, `GQ12`, `GQ13`, `GQ20`, `GE04`, `GE05`, `GE07`, `GF13`, and
  `GH02` are now registered and covered by corpus cases, and
  `STDDEV_POP` / `STDDEV_SAMP` execute with an O(1) Welford accumulator.
- `ExecutorWarning`, `WarningSink`, and
  `Session::with_warning_sink(...)` for opt-in runtime warning collection.
  Aggregate NULL elimination now emits ISO GQLSTATUS `01G11` once per
  aggregate expression per statement; sessions without a sink silently discard
  warnings.

### Performance

#### Pre-1.1.0 deep-review hot-path pass (no behavior change)

- **Commit path (`selene-graph`):** label-index inserts in place
  (`entry().or_default().insert()`, no whole-`RoaringBitmap` clone per label per
  node); typed-index union via bulk `|= &bitmap` (no per-element `insert_all`);
  closed-graph validation borrows `LabelSet`/`PropertyMap` (clones only on
  error); `candidate_keys` empty-index fast path. The provider encodes its rkyv
  section lock-free (the mutex is held only for `load_full`) and no longer
  re-allocates the principal slice per commit. (GRAPH-01/02/03/04/07/08)
- **Core (`selene-core`):** `PropertyMap::iter`/`keys`/`values` return concrete
  iterators instead of a per-call `Box<dyn Iterator>` on the commit/validate hot
  path (sorted present-only order preserved). (CORE-07)
- **Persistence (`selene-persist`):** `decode_changes` reads from the borrow (no
  `to_vec`); `SnapshotReader.sections` is an `Arc<[SectionEntry]>` (no double Vec
  clone). (PERSIST-03/07)
- **Query runtime (`selene-gql`):** GROUP BY keys into an insertion-ordered Vec
  (O(n) vs O(n_rows × n_groups)) with a `group_by_key_cap`; UNION consumes the
  owned right-hand side (no per-row clone); `IS NORMALIZED` is allocation-free;
  `NULLIF` borrows for the equality test. The parser borrows bare identifiers
  via `Cow`, builds DELETE ops single-pass, and canonicalizes keywords
  allocation-free; the planner precomputes selectivity outside the comparator
  and drains instead of `remove(0)`-looping. (GQLRT-02/03/04/06, PARSE-22/23/24/
  25, PLAN-10/11/12)
- **Algorithms (`selene-algorithms`):** release builds skip WCC's redundant
  in-neighbor union (debug retains the transpose cross-check); `triangle_count`
  and the API runners collapse their duplicated checked/unchecked
  implementations onto a single checker-threaded path. (ALGO-11/17/18)
- **Robustness:** the analyzer's `ExprId` fingerprint memo is now scoped to the
  single `for_expr` recursive walk (where all nodes are provably live) rather
  than persisted on the table with a subquery-boundary `clear()`, eliminating a
  pointer-reuse → stale-fingerprint → false-dedup hazard by construction.
  (ANALYZE-06)

### Fixed

- Align `POWER` overflow GQLSTATUS handling with ISO/IEC 39075:2024 §20.22 GR11:
  overflow now maps to `22003` numeric-value-out-of-range, while `2201F`
  invalid-argument-for-power-function is reserved for the zero-base negative
  exponent and negative-base non-integral exponent cases.

#### Pre-1.1.0 deep-review correctness pass

- **`RETURN UNKNOWN` is now accepted.** Per ISO §21.2 `<boolean literal> ::=
  TRUE | FALSE | UNKNOWN` and BOOLEAN is mandatory; `UNKNOWN` lowers to the Null
  truth value (was rejected `42N01`). (PARSE-01)
- **Three-valued-logic analyzer parity.** The analyzer no longer statically
  rejects `NULL < 5`, `NULL + 1`, `-NULL`, `NULL AND TRUE`, or `NOT NULL` — the
  runtime already evaluated these to NULL/UNKNOWN correctly, so the static
  rejection was a false negative (ISO §19.3 / §20.21 / §6.4). (ANALYZE-01)
- **Unary negate covers every numeric type.** `-x` now handles `Uint` /
  `Int128` / `Uint128` / `Float32` / `Decimal` (was `Int`/`Float` only);
  unsigned operands promote to the signed width and report `22003` when the
  magnitude cannot fit. (GQLRT-01)
- **Cross-type numeric comparison and equality are now exact and complete.**
  `numeric_equal` / `numeric_compare` / the runtime hash key cover
  `Int128` / `Uint128` / `Decimal` cross-type by exact representability —
  e.g. `$i128 = 1` is now `TRUE` (previously `FALSE`, even though `<= 1 AND
  >= 1` was `TRUE`). `SUM`/`AVG` widen `Int → Int128 → Decimal → Float`
  (SUM past `i64` → `i128`; overflow → `22003`), and `eval_arithmetic` gains
  the Decimal / `Uint128 + Uint128` / `Int128 ↔ Uint128` arms. The GV13/GV14/
  GV17 128-bit/Decimal feature claims are now honest end-to-end (no flagger
  change). The parity invariant holds throughout: equality, the hash/group
  key, and ordering all route through one canonical form, so DISTINCT /
  GROUP BY / set-ops agree with `=`. (GQLRT-26/27/30)
- **RECORD equality is field-name-keyed** (ISO §4.15). `{a: 1, b: 2} =
  {b: 2, a: 1}` is now `TRUE` (was a positional-zip `FALSE`); permuted records
  collapse correctly under DISTINCT / GROUP BY, with NULL/NaN → Unknown 3VL
  preserved. Only the reachable `Value::Record(Open)` path is affected; the
  deferred `Value::RecordTyped` arms (node-791) and Rust's derived
  `PartialEq`/`Hash` (HashMap keys / §16 identity) are untouched. (GQLRT-14)
- **The optimizer binding-ref collector walks `IS SOURCE/DESTINATION OF`
  operands.** `n IS SOURCE OF e` now contributes `{n, e}` (was `{n}`), so
  `expand_filter_pushdown` no longer pushes a predicate onto `n`'s scan before
  `e` is bound. (PLAN-01)
- **Recovery no longer double-rebuilds and double-validates indexes.**
  `into_graph` is registration-only; recovered closed-graph violations now
  surface as `TypeViolation`, matching the documented `recover_closed`
  contract. (GRAPH-06)

## [1.0.0] — 2026-05-16

First stable release. selene-db is now usable as a Rust dependency for
embedding a property graph engine that targets ISO/IEC 39075:2024 (GQL)
conformance. The public API surface across `selene-core`,
`selene-graph`, `selene-persist`, `selene-gql`, and `selene-pack` is
considered stable: subsequent 1.x releases will maintain
backwards-compatible additions and reserve breaking changes for major
version bumps.

### Highlights

- **Strict ISO/IEC 39075:2024 GQL** parser, semantic analyzer, planner,
  optimizer, and executor. The GQL Flagger (Clause 24.6) rejects
  non-standard or unclaimed constructs at parse time. No Cypher, no
  SQL, no SPARQL grammar leaks into the engine.
- **In-memory property graph** with copy-on-write isolation built on
  `ArcSwap` + `parking_lot::RwLock` + `imbl` persistent collections +
  `RoaringBitmap` label indexes + typed secondary indexes. Single graph
  write-lock with lock-free reads; strict-serializable transaction
  isolation.
- **Write-ahead log** (`SLDB` magic) and **rkyv-archived snapshots**
  (`SLSN` magic) with two-step recovery; the persistence crate is
  graph-blind (operates on `&[Change]`) so graph types can evolve
  independently.
- **Procedure-pack registry** with JSON Schema 2020-12 manifest
  validation, typestate-sealed activation, and a single mutation funnel
  shared between graph writes and pack lifecycle audit (atomic via the
  WAL).
- **Graph algorithm library** with 15 algorithms across four families
  (structural, pathfinding, centrality, community), exposed through 19
  `algo.*` procedures via `selene-algorithms-pack`. Rayon-parallel
  implementations for APSP, betweenness, PageRank, and triangle count.
- **Vector index extension** with HNSW and IVF providers, SQ8/PQ/OPQ
  quantization, and 9 `vector.*` procedures via `selene-vector-pack`.
- **Snapshot-protected** runtime surfaces: planner, executor,
  procedure-pack, and algorithm outputs are pinned by golden snapshots
  for drift detection (D21 snapshot harness pattern).
- **Engineering posture**: `#![forbid(unsafe_code)]` workspace-wide,
  `missing_docs = "deny"`, 700 LOC per-file cap, rustls-only TLS in
  transitive dependencies, no hand-rolled crypto / TLS / async / serde
  primitives.

### Crates

- `selene-core` — foundation types: `Value`, `IStr` interner,
  `PropertyMap`, `LabelSet`, schema types, `Codec`, `Origin`,
  `Changeset`, `feature_register` (ISO feature claims).
- `selene-graph` — in-memory property graph: storage primitives,
  `Mutator` write funnel, label/typed/composite indexes,
  `IndexProvider` extension trait, `GraphTypeDef` runtime binding.
- `selene-persist` — WAL format (`SLDB`), snapshot format with
  TLV-tagged sections (`SLSN`), recovery pipeline. Graph-blind.
- `selene-gql` — pest GQL grammar, AST, semantic analyzer, planner,
  rule-based optimizer (13 rules), row-at-a-time executor,
  `ProcedureRegistry` trait, GQL Flagger.
- `selene-pack` — procedure-pack registry, manifest validator,
  typestate activation state machine, atomic mutation-funnel audit,
  blake3 content hashing, platform built-ins (`selene.health`,
  `selene.create_index`, `selene.drop_index`, `selene.pack.history`,
  `selene.feature_status`).
- `selene-algorithms` — `GraphProjection` + `ProjectionCatalog`
  foundation, four algorithm families. Independent of `selene-gql`.
- `selene-algorithms-pack` — procedure-pack adapters that expose
  `selene-algorithms` through GQL `CALL`.
- `selene-vector` — opt-in HNSW and IVF vector indexes with search,
  mutation replay, snapshots, quantization, `IndexProvider`
  registration.
- `selene-vector-pack` — procedure-pack adapters for vector search,
  mutation, bulk mutation, IVF search, and IVF stats through `CALL`.
- `selene-testing` — shared fixtures, synthetic graph generators, and
  pure-mirror snapshot-harness DSLs. Dev-only.

### ISO/IEC 39075:2024 conformance

selene-db targets **minimum conformance** plus a curated subset of
optional features:

- Mandatory data types: `STRING`, `BOOLEAN`, `INT`, `FLOAT`.
- Optional types behind ISO feature gates: date/time, decimal, list,
  record, path, references.
- Both `GG01` (open graph) and `GG02` (closed graph) are supported;
  per-graph choice.
- Default transaction isolation is **serializable** (Clause 4.6).
- Implementation-defined hooks claimed: `IW010` (external procedures
  via `CALL`), `IV011` (dynamic property value type), `ID001` /
  `IW002` / `ID003` (principals, authorization, and privileges as
  embedder responsibilities), `IE002` / `IE004` (transaction
  isolation).
- No wire format is in scope (Clause 4.2.3 is explicit). Embedders pick
  their own transport.

The `feature_register` in `selene-core` is the authoritative list of
claimed Implication-table features. The Flagger rejects unclaimed
constructs at parse time so embedders cannot accidentally rely on
non-standard syntax.

### Snapshot and WAL format

- WAL magic: `SLDB`. Append-only log of `Change` records with
  configurable `SyncPolicy` (synchronous, batched, or unsynced for
  testing).
- Snapshot magic: `SLSN`. rkyv-archived TLV sections; producer-tagged
  for graph core (`GRPH`), vectors (`VECS`), quantization (`QUNT`,
  `CQNT`, `IPQB`), IVF (`IVF1`), OPQ rotation (`ROTN`), and PQ
  training (`PQTC`). Extensions register their own section tags.
- Two-step recovery: load snapshot, replay WAL from snapshot's last
  seqno. Selene preserves byte-parity for prior snapshot section
  versions on load while writing the current version.

### Performance posture

Benchmarks are local-only via `scripts/run-benches.sh` (criterion and
iai-callgrind, sequential, with `mimalloc` as the global allocator).
[`BENCHMARKS.md`](BENCHMARKS.md) is the committed, dated measurement
record. Selected v1.0.0 headlines (Apple M5, 100k-scale full profile):

| Surface | Measurement |
|---|---|
| `graph_node_fetch` | ~2.10 ns (columnar storage, lock-free read) |
| `graph_typed_index_point` | ~4.53 ns (flat-curve `Cow` tri-state) |
| `gql_analyze_corpus/m5c` | ~5.32 µs (full analyzer pipeline) |
| `algo_betweenness 100k parallel speedup` | ~2.40× over sequential |
| `vector_ivf_search` | ~2.88 µs per query |

See [`docs/performance.md`](docs/performance.md) for the full surface,
tuning knobs, and methodology.

### Documentation

This release introduces a full user-facing documentation set under
[`docs/`](docs/):

- [Getting started](docs/getting-started.md)
- [Embedding selene-db](docs/embedding-guide.md)
- [GQL reference](docs/gql-reference.md)
- [Architecture](docs/architecture.md)
- [Extension guide](docs/extension-guide.md)
- [Vector search](docs/vector-search.md)
- [Graph algorithms](docs/graph-algorithms.md)
- [Persistence and recovery](docs/persistence-and-recovery.md)
- [Performance](docs/performance.md)
- [Contributing](docs/contributing.md)

The README is now focused on evaluation and orientation; depth lives in
the documentation pages above.

### Stability guarantees

The following surfaces are stable starting with 1.0.0:

- Public types and traits in `selene-core` (`Value`, `IStr`,
  `PropertyMap`, `LabelSet`, `Change`, `Codec`).
- Public types and methods in `selene-graph` (`SharedGraph`,
  `Mutator`, `IndexProvider`, write-transaction commit flow).
- Public types and methods in `selene-persist` (WAL format, snapshot
  format, recovery API).
- Public types and methods in `selene-gql` (`parse`, `analyze`,
  `plan`, `execute_statement`, `Session`, `ProcedureRegistry`).
- Public types and methods in `selene-pack` (manifest schema,
  `ExternalProcedurePack`, lifecycle events).
- Wire formats for WAL and snapshot sections (read-side compatibility
  preserved across 1.x).
- The 13 optimizer rules and their static effects on plans.
- The 19 `algo.*` and 9 `vector.*` procedure surfaces.

### Platform support

| Platform | Status |
|---|---|
| Linux (x86_64, aarch64) | Primary deployment target |
| macOS (Apple Silicon, Intel) | Primary development target |
| Windows | Out of scope |

### Known deferrals (post-1.0.0)

The following items are intentionally deferred and tracked for future
1.x releases:

- Louvain parallelization (currently sequential).
- Edge-index planner support (typed/composite indexes for edges
  currently return `Linear` selectivity).
- Analyzer recursion-depth bound (parser is bounded at 64; analyzer
  binding is not yet contractually bounded).
- Mutation/`MATCH` planner threading of `BindingId` (currently uses
  per-statement context).
- OPQ rotation inner-allocation tightening.
- Fresh extension crates beyond `selene-vector` and `selene-algorithms`.

[Unreleased]: https://github.com/jscott3201/selene-db/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/jscott3201/selene-db/releases/tag/v1.1.0
[1.0.0]: https://github.com/jscott3201/selene-db/releases/tag/v1.0.0
