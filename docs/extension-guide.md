# Extension Guide

This guide is for engineers writing an extension crate against `selene-db`: a new index type, a new procedure pack, or both. It assumes you have read [`docs/architecture.md`](architecture.md) (especially the D1-D21 decisions) and [`docs/embedding-guide.md`](embedding-guide.md) (the embedder workflow, transaction model, persistence wiring).

The guide covers:

1. The two extension points the engine actually exposes.
2. What a procedure pack is and how it is loaded.
3. The procedure-pack manifest format and gate taxonomy.
4. The three procedure tiers and their context structs.
5. A full worked example: `hello.world` from trait impl through registration and tests.
6. How procedure-pack lifecycle events flow through the mutation funnel.
7. Reading `selene-algorithms-pack` as the canonical example.
8. The `IndexProvider` trait and its consistency / recovery / snapshot contract.
9. Snapshot section encoding (`SLSN` TLV, rkyv archives).
10. Testing your extension via the pure-mirror snapshot harness.
11. Versioning your extension and the wire-format invariants you owe consumers.

---

## 1. Two extension points

selene-db has exactly two extension points. Both are stable APIs. Everything else in the engine is internal and not a supported integration surface.

| Extension point | What it adds | Trait / surface | Lives in |
|:---|:---|:---|:---|
| `IndexProvider` | Derived state attached to a `SharedGraph`: a secondary index, a time-series materialization, an external mirror. | `selene_graph::IndexProvider` (trait, `Send + Sync + 'static`). | `selene-graph` |
| Procedure pack | Named procedures invokable as `CALL <pack>.<procedure>(...)` from GQL. | `selene_pack::ExternalProcedurePack` plus `ExternalGraphProcedure` / `ExternalMutationProcedure`. | `selene-pack` |

This is what D5 (architecture decision 5: non-graph capabilities in extension crates) means in practice. The graph crate ships pure graph storage plus the `IndexProvider` hook. Anything else - graph algorithms, time series, RDF - is its own crate and plugs in through one or both of these points.

The two extension points are independent. A pack can be pure read-only GQL surface with no `IndexProvider`. A provider can be a silent WAL sink with no procedures. Most non-trivial extensions ship a procedure pack, and a non-trivial extension that owns persistent derived state ships an `IndexProvider` alongside it; `selene-algorithms-pack` is the canonical procedure-pack example.

---

## 2. Procedure pack overview

A procedure pack is the bundle that a crate ships in order to expose `CALL <pack>.<procedure>(...)` from GQL.

A pack ships three things:

1. **A manifest.** JSON Schema 2020-12 validated. Declares pack name, version, content hash, and per-procedure metadata (name, tier, mutability, input schema, output schema, capability). The manifest is the immutable contract; deviations are rejected at construction time, not at runtime.
2. **Procedure implementations.** One or more types implementing `ExternalGraphProcedure` (read tier) or `ExternalMutationProcedure` (write tier). Each procedure carries its own static signature, output columns, and `execute(...)` body.
3. **Optional snapshot harness participation.** A pack that owns persistent derived state (rather than only computing over the live graph) typically also ships an `IndexProvider` that owns snapshot sections and a WAL event payload. The pack and provider are wired together at registry build time.

### Three lifecycle phases

Pack loading happens in three phases. They are codified by the typestate state machine in `selene_pack::activation` (`Uploaded -> Validating -> Staged -> Active`, plus `Deprecated` and `Disabled` terminals). The phases are:

1. **Register.** The embedder calls `ProcedurePackRegistry::builder().with_external_pack(MyPack::new().external_pack()).build()`. The builder validates pack-name shape, checks for duplicate pack names, allocates `ProcedureHandle`s, and freezes the result into a `ProcedurePackRegistry`. From this point the registry is read-only (D16: frozen registry).
2. **Validate.** When a manifest is admitted (typically at the same construction step, or separately through the `Uploaded::validate` typestate), it is parsed through `parse_manifest(&[u8]) -> Result<ProcedurePackManifest, ManifestError>`. Twenty-two manifest gates run in a defined order: syntax, typed shape, schema version support, pack name lexical, procedure count bound, name uniqueness, name lexical, namespace reservation, persist-tier rejection, tier/mutability consistency, name length, inline-schema size, inline-schema validity, path-schema safety, input-schema compile, output-schema compile, capability format, content-hash canonical, content-hash consistency, and the activation-seal pair.
3. **Activate.** When a `Staged` pack is committed to the live registry, an `Activated` `LifecycleEvent` is recorded through the `LifecycleSink`. The default sink for embedder use is `GraphCommitSink`, which routes the event through `Mutator::schema_change` so audit and graph commit are atomic (D18: lifecycle audit through the mutation funnel).

Construction-time registration is the only registration path in v1.0. The `ProcedurePackRegistry` has no `add_pack` or `remove_pack` method. Restart, rebuild, replace.

---

## 3. Procedure-pack manifest format

The manifest is a JSON document validated against the JSON Schema 2020-12 document returned by `selene_pack::manifest_json_schema()` (D15). The typed shape is `selene_pack::ProcedurePackManifest`.

### Required top-level fields

| Field | Type | Notes |
|:---|:---|:---|
| `schema_version` | `u32` | Must equal `SCHEMA_VERSION_SUPPORTED` (currently `1`). |
| `pack_name` | `string` | Canonical single-segment ASCII name. Cannot start with the reserved `selene` prefix. |
| `pack_version` | `string` | Semver `MAJOR.MINOR.PATCH`. |
| `content_hash` | `string` | Format `blake3:<64 lowercase hex chars>`. Must match the canonical blake3 hash of the manifest with `content_hash` zeroed (D19). |
| `procedures` | `array` | At most `MAX_PROCEDURES_PER_PACK` (256) entries. |

### Required per-procedure fields

| Field | Type | Notes |
|:---|:---|:---|
| `name` | `string` | Dot-joined canonical name, must start with `<pack_name>.`, byte length at most `MAX_PROCEDURE_NAME_LENGTH` (255). |
| `tier` | `"graph"` \| `"mutation"` \| `"persist"` | The `persist` tier is reserved and rejected in v1.0 manifests. |
| `mutability` | `"read"` \| `"graph_write"` \| `"schema_write"` \| `"admin"` | Must be consistent with `tier`: `graph` + `read`, `mutation` + `graph_write` or `schema_write`. |
| `input_schema` | `{ "inline": <JSON Schema> }` or `{ "path": { "relative_to": "<path>" } }` | Inline schemas are bounded by `MAX_INLINE_SCHEMA_SIZE_BYTES` (64 KiB); path references must be safe relative paths under the manifest directory. |
| `output_schema` | same shape as `input_schema` | Same bounds and rules. |
| `capability_required` | `string` or `null` | Optional opaque capability token. |

### Concrete manifest

A minimal manifest with no procedures declared looks like:

```json
{
  "schema_version": 1,
  "pack_name": "demo_pack",
  "pack_version": "0.1.0",
  "content_hash": "blake3:88f46c373df17f993f3d9765f6eab3ec4cfe420c6b42c9f4996dc23265e4d60b",
  "procedures": []
}
```

A typical procedure entry, taken from a `demo_pack.echo` test fixture, is:

```json
{
  "name": "demo_pack.echo",
  "tier": "graph",
  "mutability": "read",
  "input_schema": { "inline": { "type": "object" } },
  "output_schema": { "inline": { "type": "object" } },
  "capability_required": null
}
```

### Manifest gate taxonomy

Gates are exposed as the `Gate` enum (`selene_pack::Gate`) and grouped into evaluation-order slices: `MANIFEST_LEVEL_GATES` (post-typed-deserialization), `PROCEDURE_LEVEL_GATES` (per procedure), `FINAL_VALIDATION_COVERAGE` (content-hash canonical and consistency), `ACTIVATION_SEAL_COVERAGE` (registry conflict detection), `WAL_AUDIT_COVERAGE` (lifecycle atomicity), and `MANIFEST_VALIDATION_COVERAGE` (the complete ordered taxonomy). Pack authors rarely call these directly; they exist so the engine can self-test gate coverage and so embedders can introspect the validation contract.

### Computing the content hash

`content_hash` must be the blake3 hash of the canonicalized manifest with the `content_hash` field replaced by the literal sentinel `blake3:0000000000000000000000000000000000000000000000000000000000000000`. The helper for this is `selene_pack::ContentHash::from_validated_manifest(&manifest)`. Pack authors that ship the manifest as a static resource compute the hash once (typically in a build script or in a hand-run helper) and commit the result.

---

## 4. Tier procedures

Procedures are partitioned by tier (D17). Each tier has its own context struct and its own trait. Tier compatibility is enforced at plan time by the GQL analyzer against the surrounding statement category.

| Tier | Manifest tier | External trait | Context struct | Statement category required |
|:---|:---|:---|:---|:---|
| Read | `graph` | `ExternalGraphProcedure` | `GraphContext<'a>` | `ReadOnly` (or auto-committable container) |
| Write | `mutation` | `ExternalMutationProcedure` | `MutationContext<'a, 'g>` | `DataModifying` |
| Admin | (n/a as a separate context type) | (built-in only) | `ProcedureContext::Mutation`-wrapped admin built-ins | `CatalogModifying` |

The `ProcedureContext<'a, 'g>` enum is `#[non_exhaustive]` with two variants today: `Graph(GraphContext<'a>)` and `Mutation(MutationContext<'a, 'g>)`. A `persist` tier exists in the type-level enums (`ProcedureTier::Persist`, `ManifestTier::Persist`) but is rejected by both the manifest validator and the registry builder in v1.0.

### `GraphContext` (read tier)

`GraphContext` lets a read-tier procedure:

- borrow the published `&SeleneGraph` snapshot it is executing against (`ctx.snapshot()`),
- look up registered `IndexProvider`s by their 4-byte `ProviderTag` (`ctx.index_provider_by_tag(tag)`),
- read implementation-defined executor caps (`ctx.impl_defined_caps()`).

`GraphContext` does NOT expose a `Mutator`. Read-tier procedures cannot write the graph; the engine rejects an attempt to install a `read`/`graph` procedure that calls `begin_write` through any back channel (cross-thread re-entry into `begin_write` is documented misuse; see the `IndexProvider` rustdoc).

### `MutationContext` (write tier)

`MutationContext` lets a write-tier procedure:

- borrow the transaction-local working graph as a `&SeleneGraph` (`ctx.snapshot()`),
- borrow the active `Mutator<'a, 'g>` (`ctx.mutator()`) and emit `Change`s through it,
- look up `IndexProvider`s by tag (`ctx.index_provider_by_tag(tag)`).

The mutator hands a procedure access to the same write funnel the embedder uses directly. Every mutation the procedure performs flows through `Mutator::*`, accumulates in the transaction's pending change list, and commits atomically with the procedure's outer `WriteTxn`.

### Procedure metadata

Each procedure also declares static metadata through `ExternalProcedureMetadata`:

```rust
pub trait ExternalProcedureMetadata: Send + Sync + 'static {
    fn name(&self) -> &'static [&'static str];           // ["pack", "proc"]
    fn signature(&self) -> Vec<ExternalParameter>;       // positional inputs
    fn output_columns(&self) -> Vec<ExternalOutputColumn>; // YIELD columns
}
```

`ExternalParameter` carries `name: &'static str`, `ty: GqlType`, `nullable: bool`. `ExternalOutputColumn` carries `name: &'static str`, `ty: GqlType`. `GqlType` is the type AST enum from `selene-gql` (`String`, `Boolean`, `Integer`, `Float`, `NodeRef`, `EdgeRef`, plus the temporal and decimal types).

---

## 5. Writing a procedure: `hello.world`

The walk-through ships a one-procedure read-tier pack: `CALL hello.world(name) YIELD greeting`.

### 5.1 Crate skeleton

```toml
# my-hello-pack/Cargo.toml
[package]
name = "my-hello-pack"
version = "0.1.0"
edition = "2024"

[dependencies]
selene-core  = { path = "path/to/selene-db/crates/selene-core" }
selene-gql   = { path = "path/to/selene-db/crates/selene-gql" }
selene-graph = { path = "path/to/selene-db/crates/selene-graph" }
selene-pack  = { path = "path/to/selene-db/crates/selene-pack" }
```

The pack crate depends on `selene-pack` (for the registration surface), `selene-gql` (for `GqlType` and the procedure context), and `selene-core` (for `Value` and the interner). It does NOT depend on `selene-persist` unless it ships an `IndexProvider` that owns snapshot sections.

### 5.2 The procedure

```rust
// my-hello-pack/src/world.rs
use std::sync::Arc;

use selene_core::{Value, intern, resolve};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter,
    ExternalProcedureMetadata,
};

static HELLO_WORLD_NAME: [&str; 2] = ["hello", "world"];

/// `CALL hello.world(name) YIELD greeting`
pub(crate) struct HelloWorld;

impl ExternalProcedureMetadata for HelloWorld {
    fn name(&self) -> &'static [&'static str] {
        &HELLO_WORLD_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![ExternalParameter {
            name: "name",
            ty: GqlType::String,
            nullable: false,
        }]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![ExternalOutputColumn {
            name: "greeting",
            ty: GqlType::String,
        }]
    }
}

impl ExternalGraphProcedure for HelloWorld {
    fn execute(
        &self,
        _ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        if args.len() != 1 {
            return Err(ProcedureError::InvalidArgument {
                detail: format!("hello.world expects 1 argument, got {}", args.len()),
            });
        }
        let name_istr = match &args[0] {
            Value::String(istr) => *istr,
            other => {
                return Err(ProcedureError::InvalidArgument {
                    detail: format!("hello.world: arg 0 must be STRING, got {other:?}"),
                });
            }
        };
        let greeting = format!("hello, {}", resolve(name_istr));
        let greeting_istr =
            intern(&greeting).map_err(|err| ProcedureError::Internal {
                detail: format!("intern: {err}"),
            })?;
        Ok(ProcedureResult {
            rows: vec![vec![Value::String(greeting_istr)]],
        })
    }
}

pub(crate) fn procedure() -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(HelloWorld)
}
```

### 5.3 The pack handle

```rust
// my-hello-pack/src/lib.rs
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Procedure-pack adapter for the `hello.*` namespace.

mod world;

use std::sync::Arc;

use selene_pack::{
    ExternalGraphProcedure, ExternalProcedurePack, ProcedurePackRegistry, RegistryError,
};

/// Static external pack name registered with `selene-pack`.
pub const HELLO_PACK_NAME: &str = "hello";

/// Construct-time handle for the hello procedure pack.
#[derive(Clone, Debug, Default)]
pub struct HelloPack;

impl HelloPack {
    /// Construct a fresh hello pack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the external-pack description for registry-builder admission.
    #[must_use]
    pub fn external_pack(&self) -> ExternalProcedurePack {
        ExternalProcedurePack::new(HELLO_PACK_NAME, self.procedures())
    }

    /// Construct a registry containing platform built-ins and this pack.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if registration metadata is malformed or
    /// conflicts with another procedure.
    pub fn registry_with_builtins(&self) -> Result<ProcedurePackRegistry, RegistryError> {
        ProcedurePackRegistry::builder()
            .with_builtins()
            .with_external_pack(self.external_pack())
            .build()
    }

    fn procedures(&self) -> Vec<Arc<dyn ExternalGraphProcedure>> {
        vec![world::procedure()]
    }
}
```

### 5.4 The manifest

```json
{
  "schema_version": 1,
  "pack_name": "hello",
  "pack_version": "0.1.0",
  "content_hash": "blake3:<computed-hash>",
  "procedures": [
    {
      "name": "hello.world",
      "tier": "graph",
      "mutability": "read",
      "input_schema": {
        "inline": {
          "type": "object",
          "properties": {
            "name": { "type": "string" }
          },
          "required": ["name"]
        }
      },
      "output_schema": {
        "inline": {
          "type": "object",
          "properties": {
            "greeting": { "type": "string" }
          },
          "required": ["greeting"]
        }
      },
      "capability_required": null
    }
  ]
}
```

Replace `<computed-hash>` with the lowercase blake3 of the manifest canonicalized with the sentinel hash. The helper is `selene_pack::ContentHash::from_validated_manifest`. A test asserts the computed hash matches the committed value so drift is caught in CI.

### 5.5 Registering and using

```rust
use my_hello_pack::HelloPack;
use selene_core::{GraphId, Value, intern};
use selene_graph::SharedGraph;
use selene_gql::{Session, StatementOutput, analyze, execute_statement, parse, plan};

let registry = HelloPack::new().registry_with_builtins()?;
let graph = SharedGraph::new(GraphId::new(1));

let stmt = parse("CALL hello.world('Ada') YIELD greeting RETURN greeting")?;
let analyzed = analyze(stmt, &registry, None)?;
let planned = plan(&analyzed, &registry)?;
let mut session = Session::new(&graph);
let output = execute_statement(&planned, &mut session, &registry)?;

let StatementOutput::Rows(table) = output else {
    panic!("hello.world returns rows");
};
assert_eq!(table.row_count(), 1);
```

### 5.6 Test harness

`GraphContext::new` is `pub(crate)` inside `selene-gql`; tests drive procedures end-to-end through the planner/executor rather than constructing a context by hand. For more elaborate testing - golden snapshot output, lifecycle coverage, error-path matrices - use the pure-mirror DSLs in `selene-testing` (see §10).

---

## 6. The mutation-funnel audit

Procedure-pack lifecycle events (`LifecycleEvent::ValidationFailed`, `Staged`, `Activated`, `Deprecated`, `Disabled`) are not stored in a side ledger. They flow through the same `Mutator` funnel that graph writes use (D18).

The wiring is:

1. The activation typestate (`Staged::activate`, `Active::deprecate`, `Deprecated::disable`, ...) takes a `&dyn LifecycleSink` argument.
2. `selene-pack` ships two `LifecycleSink` implementations:
   - `NoopSink` - drops every event. Use for tests and for embedders that do not need durable audit.
   - `GraphCommitSink` - constructs a fresh `WriteTxn` on the held `SharedGraph`, calls `Mutator::schema_change(graph_anchor, SchemaChange::ProcedurePackLifecycle { event })`, and commits with the embedder-supplied principal bytes.
3. Because `schema_change` is just another mutation funnel event, it lands in the WAL as a `Change::SchemaChanged` entry alongside any graph writes committed in the same transaction.
4. A registered `IndexProvider` whose `on_change` matches the lifecycle event payload can mirror lifecycle state into derived form (e.g. a "current activated pack version" lookup).

This is what makes audit and graph state atomic. Either both commit or neither does; recovery replays them in the same order they happened. There is no parallel ledger and no audit-vs-graph split-brain scenario.

The `GraphCommitSink` constructor asserts the supplied `graph_anchor: GraphId` matches the live `SharedGraph::read().graph_id()` to keep per-graph audit attribution sound:

```rust
use std::sync::Arc;
use selene_core::GraphId;
use selene_pack::GraphCommitSink;

let sink = GraphCommitSink::new(Arc::clone(&graph), GraphId::new(1))
    .with_principal_bytes(principal_bytes);
```

The sink's `record` then drives `txn.commit_with_principal(self.principal_bytes.clone())`. The principal bytes are opaque audit material that flows into the WAL header for later replay attribution.

---

## 7. Reading the worked external pack

One external pack ships in-tree as a reference implementation. Read it when in doubt about a pattern.

### `selene-algorithms-pack`

Path: `crates/selene-algorithms-pack/`.

This pack exposes the nineteen graph algorithms in `selene-algorithms` through nineteen `algo.*` procedures. The crate is intentionally a thin adapter: `src/registry.rs` carries the `AlgorithmsPack` handle and the `external_pack()` builder; `src/<family>.rs` modules (pagerank, betweenness, community, structural, pathfinding, projection) each export a procedure constructor returning `Arc<dyn ExternalGraphProcedure>`; `src/args.rs` ships shared argument-parsing helpers (`expect_arity`, `required_string`, `nullable_f64`, ...); `src/state.rs` carries per-graph projection-catalog state shared across procedures via `Arc<AlgorithmsPackState>`. `selene-algorithms-pack` registers exclusively as read-tier; all nineteen algorithms read the published snapshot and compute over a `GraphProjection`.

The pack registers against a `SharedGraph` and its built-in registry:

```rust
use selene_core::GraphId;
use selene_graph::SharedGraph;
use selene_algorithms_pack::AlgorithmsPack;

let graph = SharedGraph::new(GraphId::new(1));
let pack = AlgorithmsPack::new();
let registry = pack.registry_with_builtins()?;
```

A pack that owns persistent derived state rather than only computing over the live graph additionally ships an `IndexProvider` registered through `SharedGraph::builder(...).with_provider(...)`; the provider owns the derived state and snapshot sections while the pack exposes operations over it. See §8 and §9 for that contract.

---

## 8. `IndexProvider` extension

`IndexProvider` is the trait the engine uses to admit derived state. It is defined in `selene_graph::index_provider` and the shape is:

```rust
pub trait IndexProvider: Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
    fn provider_tag(&self) -> ProviderTag;
    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError>;
    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError>;
    fn on_change(&self, change: &Change) -> Result<(), ProviderError>;
    fn declared_sub_tags(&self) -> &[SubTag];
}
```

`ProviderTag` is a `#[derive(Copy)]` 4-byte uppercase ASCII identifier (`ProviderTag(*b"GRPR")`). First-party allocations include `CORE` (engine-owned, do not collide), `TIMS`, `GRPR`. Pack authors pick a new tag from the remaining ASCII uppercase space and document it in their crate.

`SubTag` is also a 4-byte ASCII identifier, namespaced inside one provider's tag space (e.g. a provider might declare `HEAD`, `BODY`, `META` under its own tag).

### What selene-graph promises

- **Construction-time registration.** Providers are admitted to `SharedGraph` via `SharedGraph::builder(...).with_provider(Arc::new(provider))`. After `build()`, the provider set is fixed.
- **Event stream of `Change`s.** Every committed change is delivered to every registered provider via `on_change(&Change)`, in registration order, while the write lock is still held. The full `Change` variant list is in `selene_core::Change` (`NodeCreated`, `NodeUpdated`, `NodeDeleted`, `EdgeCreated`, `EdgeUpdated`, `EdgeDeleted`, `SchemaChanged`, `IndexExtensionEvent { provider, payload }`).
- **Snapshot recovery dispatch.** At recovery, the `RecoveryProvider` registry routes every snapshot section keyed by the section's `provider` tag to its `read_section(sub_tag, bytes)` callback. WAL replay then drives `on_change` for every post-snapshot change.
- **Serialized calls per graph.** The engine never calls `on_change` from two threads at once on the same provider for the same graph; you do not need a coarse lock around `on_change`.

### What the provider owes

A correct `IndexProvider` must: (1) **use interior mutability** for owned state (`parking_lot::RwLock`/`Mutex`, `arc_swap::ArcSwap`, or a lock-free map crate); (2) **honor the re-entrancy contract** - `on_change` MUST NOT initiate `begin_write` on the same graph directly or indirectly, and cross-thread re-entry that blocks the callback on a worker calling `begin_write` is documented misuse that deadlocks the engine; (3) **be consistent across replay** - snapshot decode plus WAL replay must produce byte-equivalent derived state, so non-deterministic numerics (SIMD reduction order varying by CPU) need a pinned scalar fallback for golden tests; (4) **validate every payload** - treat extension-event bytes as untrusted, bound sizes, verify magics, check version fields, refuse malformed input with `ProviderError::InvalidPayload`; (5) **filter foreign events** - `on_change` is delivered every `Change` variant, not just the provider's own `IndexExtensionEvent`, so match the variant and ignore unrelated cases.

A robust provider that owns multi-section derived state typically combines:

- an `ArcSwap<T>` publishing immutable derived snapshots so readers never block,
- a `Mutex<SectionStaging>` typestate machine guarding the section read/write protocol (a header section must precede its body; later sections validate against the committed state; emitting a partial snapshot is refused),
- explicit refusal to apply mutation events while a header section has been staged without a matching body commit,
- `Change` variant filtering: `NodeCreated`, `EdgeCreated`, etc. are noops; only `IndexExtensionEvent` whose `provider` field matches the provider's own interned name is decoded.

### `ProviderError`

`IndexProvider` methods return `ProviderError`, a `#[non_exhaustive]` enum with variants `InvalidPayload { reason }` (decode/validation failure), `SectionMissing { sub_tag }`, `SerializationFailed { reason }`, `UnknownProvider { tag, sub_tag }` (recovery routing miss), and `Inconsistent { reason }`. All variants map to GQLSTATUS `XX500` and are wrapped in `GraphError::Provider` on commit and recovery paths.

---

## 9. Snapshot encoding for an extension

Snapshots are atomic envelopes (`SLSN` magic) that contain zero or more **sections** keyed by `(provider, sub_tag)` pairs. The engine owns the `CORE` provider; extension providers own their own provider tag and their own subsections.

### TLV section structure

Per Spec 04, each section in the `SLSN` envelope is framed by:

- 4-byte `ProviderTag` (uppercase ASCII).
- 4-byte `SubTag` (provider-local).
- 4-byte big-endian length.
- length-bytes payload.
- 32-byte blake3 digest of the payload (D19).

A provider participates in the snapshot by declaring its sub-tags through `declared_sub_tags()`. At snapshot write time, the engine calls `write_section(sub_tag)` for each declared sub-tag in declared order and writes the returned bytes into the envelope. At recovery, the engine calls `read_section(sub_tag, bytes)` for each section whose `provider` tag matches.

### rkyv as the canonical payload format

selene-db uses rkyv 0.8 with `pointer_width_64` and `unaligned` features for snapshot section payloads (D14). The benefit is zero-copy decode from a `Vec<u8>` buffer with no `unsafe`. Pack authors that ship snapshot sections almost always use rkyv too; the engine does not mandate this, but the test corpus and the persistence path are tuned for archived bodies.

A minimal section body looks like:

```rust
use rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct MyHeaderV1 {
    pub(crate) dim: u16,
    pub(crate) count: u32,
    pub(crate) magic: [u8; 4],   // version pin
}

fn encode(header: &MyHeaderV1) -> Result<Vec<u8>, ProviderError> {
    rkyv::to_bytes::<rkyv::rancor::Error>(header)
        .map(|aligned| aligned.into_vec())
        .map_err(|err| ProviderError::SerializationFailed { reason: err.to_string() })
}

fn decode(bytes: &[u8]) -> Result<MyHeaderV1, ProviderError> {
    rkyv::from_bytes::<MyHeaderV1, rkyv::rancor::Error>(bytes)
        .map_err(|err| ProviderError::InvalidPayload { reason: err.to_string() })
}
```

### A multi-section layout (illustrative example)

A provider that owns several related sections declares them in a fixed
order. As a worked shape, a provider with tag `DEMO` might declare three
subsections:

| `SubTag` | Body type | What it carries |
|:---|:---|:---|
| `HEAD` | `HeadBody` (header + per-row topology + optional liveness map) | Structural topology and a live-slot bitmap. A version magic in the header lets new layouts add a magic without breaking old decoders. |
| `BODY` | `BodyV1` (header + flat payload) | The bulk payload. Cross-validated against `HEAD` for matching counts. |
| `META` | `MetaBody` or empty | An optional overlay. May be empty when the feature it backs is disabled. |

The write protocol is ordered: `HEAD` must be emitted before `BODY`, and `META` is optional and emitted last. The provider implements a typestate machine inside a `staging: Mutex<SectionStaging>` field to enforce this. A failed encode resets staging so a subsequent retry does not pick up stale captured state.

The read protocol is the mirror: `HEAD` admits the topology, `BODY` admits the bulk payload and the provider then assembles its in-memory state, and `META` (if present and non-empty) attaches a post-commit overlay via an `ArcSwap` `rcu`.

### Extension events on the WAL

A provider that mutates derived state in response to procedure calls emits a `Change::IndexExtensionEvent { provider, payload }` through the mutation funnel:

```rust
use std::sync::Arc;
use selene_core::intern;

let payload: Arc<[u8]> = my_encoded_event.into();
ctx.mutator()
    .extension_event(intern("my-provider")?, payload);
```

The mutator records the event in the transaction's change list; on commit, the WAL writer frames it as a `Change::IndexExtensionEvent` and fans it out to every registered provider via `on_change`. Each provider filters by the `provider` field (a `selene_core::IStr`) and ignores events not addressed to it; the value is the provider's own interned crate name.

The extension-event payload format is provider-owned. Spec 04 only specifies the framing; the bytes inside are yours. A common pattern is a magic-prefixed postcard-serialized enum so the decoder can dispatch by event kind and reject malformed input.

---

## 10. Testing your extension

selene-db pins runtime surfaces with a snapshot harness (D21). The pattern is the same whether you are pinning a planner output, an executor result, or an extension procedure's row shape.

### The pure-mirror DSL

`selene-testing` ships pure-mirror DSLs that mirror the public surface of each pinned producer. The mirror does not depend on the target crate (per the no-target-dep invariant); it expresses the output shape as a set of serializable structs.

For a procedure-pack author, the relevant entry points are `selene_testing::pack_corpus::PackCorpusFixture` (with `PackManifestFixture` and `PackLifecycleStep`), `selene_testing::mock_procedure_registry` (a stub `ProcedureRegistry` for analyzer/planner tests), and `selene_testing::pack_corpus::coverage::GATE_COVERAGE` (the validation-gate slice for gate-coverage cross-checks).

### The three-piece pattern

For each pinned surface:

1. **A pure-mirror struct in `selene-testing`** that names every field a snapshot should pin.
2. **A renderer in the target crate** (your pack crate) that constructs the mirror from a real producer.
3. **An integration test** in the target crate's `tests/` directory that:
   - fans out over a corpus of inputs,
   - invokes the renderer,
   - asserts each rendered mirror against a committed `.snap` file via `insta` (`insta::assert_yaml_snapshot!(rendered);`).

For a hello-world pack with one procedure, a typical integration test looks like:

```rust
// my-hello-pack/tests/golden.rs
use my_hello_pack::HelloPack;
use selene_core::{GraphId, Value, intern, resolve};
use selene_graph::SharedGraph;
use selene_gql::{Session, StatementOutput, analyze, execute_statement, parse, plan};

#[test]
fn hello_world_golden() {
    let registry = HelloPack::new().registry_with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(1));
    let stmt = parse("CALL hello.world('Ada') YIELD greeting RETURN greeting").unwrap();
    let analyzed = analyze(stmt, &registry, None).unwrap();
    let planned = plan(&analyzed, &registry).unwrap();
    let mut session = Session::new(&graph);
    let output = execute_statement(&planned, &mut session, &registry).unwrap();

    let StatementOutput::Rows(table) = output else {
        panic!("expected rows");
    };
    let greetings: Vec<String> = table
        .rows()
        .iter()
        .map(|row| match row.values()[0] {
            Value::String(istr) => resolve(istr).to_owned(),
            _ => panic!("expected STRING"),
        })
        .collect();

    insta::assert_yaml_snapshot!(greetings, @r"
    - hello, Ada
    ");
}
```

For an `IndexProvider`, the golden surface is typically the snapshot section bytes (encoded via the renderer into a hex-formatted dump or a structural mirror), the event-replay trace, and the recovery result. A robust provider pins all three.

### Recovery acceptance tests

A provider that adds a new section variant or a new extension-event variant ships a "commit, omit snapshot, recover, assert" test. The pattern is:

1. Build a `SharedGraph` with the provider registered.
2. Run a sequence of mutations that produce the new event/section variant.
3. Open a `SnapshotBuilder`, ask the provider for its sections, write a snapshot envelope.
4. Tear down the graph and rebuild it from disk via `selene_persist::recover`.
5. Assert the recovered provider state matches the pre-tear-down state exactly.

Without this test, a producer + consumer split across crates (provider emits bytes, persistence reads bytes) can drift silently. CI green on each crate independently is not enough.

---

## 11. Versioning your extension

A procedure pack and its provider together form a wire-format contract. Three invariants matter.

### 11.1 Manifest schema version

The top-level `schema_version` field is set by selene-pack and is currently `1`. When the manifest schema evolves, `SCHEMA_VERSION_SUPPORTED` is bumped. A pack manifest declaring a higher `schema_version` than the running binary is rejected with `ManifestError::UnsupportedSchemaVersion`. Pack authors update their manifest when the engine moves the bound.

### 11.2 Pack version

The manifest's `pack_version` field is your own semver string. Bump it whenever a procedure's signature, output columns, mutability, or tier changes, or a procedure is removed. Adding a new procedure is a minor bump; changing a signature on an existing procedure is a major bump. The `content_hash` field re-derives automatically from the canonical manifest payload, so any structural change forces a fresh hash and a fresh activation event.

### 11.3 Provider section format

This is the harder invariant. Once a snapshot section with `(provider, sub_tag)` is written to disk, your provider must be able to decode that section forever, or you must ship a migration.

The recommended pattern is **magic-prefixed bodies**:

```rust
pub(crate) const PAYLOAD_MAGIC_HEAD:    [u8; 4] = *b"HDR1";   // v1
pub(crate) const PAYLOAD_MAGIC_HEAD_V2: [u8; 4] = *b"HDR2";   // v2
```

The decoder reads the magic first, dispatches by version, and returns a unified body type. New versions add new magic constants; old versions remain decodable. The encoder picks the lowest version that can represent the current state (e.g. v1 when no v2-only field is set, v2 when the richer layout is needed).

Three rules for bumping section versions:

1. **Bump the magic, not the type.** A new magic constant is much easier to reason about than a `version: u8` field inside an existing archived struct. Different magics decode through different rkyv-archived types.
2. **Ship a decoder for every magic the provider has ever emitted.** Until you ship a one-shot migration that re-writes old snapshots, your provider's decode dispatch must understand every historical version.
3. **Recovery tests at every version.** For each released version that may still exist on disk, a recovery acceptance test pins the byte-stable replay path.

### 11.4 Byte parity for compatibility

If your provider claims byte parity across builds (so two embedders writing the same logical state produce byte-identical snapshots), the encode path must be deterministic: no `HashMap` iteration order in serialized bytes (use `BTreeMap` or sort explicitly), no non-deterministic numerics (SIMD reductions over `f32` are not order-equivalent across CPUs - pin a scalar fallback for golden tests), no clock/PID material in section payloads, no allocation-address-dependent ordering in archived nodes. A provider whose derived state depends on order-sensitive floating-point reductions may NOT be able to claim byte parity across all builds; where byte parity is needed, harnesses pin a scalar-only configuration. A provider that does claim byte parity must say so in its rustdoc and back the claim with a golden snapshot test in CI.

---

## See also

- [`docs/architecture.md`](architecture.md) - D1-D21 decisions, layered model, persistence design.
- [`docs/embedding-guide.md`](embedding-guide.md) - the embedder workflow, registering providers, wiring the WAL.
- [`docs/persistence-and-recovery.md`](persistence-and-recovery.md) - WAL framing, snapshot envelope, two-step recovery.
- [`docs/graph-algorithms.md`](graph-algorithms.md) - the worked external pack in user-facing form.
- `crates/selene-pack/src/`, `crates/selene-algorithms-pack/` - canonical source references.
