# Catalog lifecycle and read-snapshot contract

M02-PR02 defines catalog identity and immutable metadata. M02-PR03 adds the
in-memory lifecycle service and named runtime graph registry. Catalog changes
are not durable yet; M09 owns their persisted representation and recovery.

```text
CatalogId + CatalogDescriptor
└── DirectoryId + synthetic root name ""
    ├── SchemaId + CatalogName
    │   └── one shared canonical-name dictionary
    │       ├── GraphId
    │       ├── GraphTypeId
    │       ├── BindingTableId
    │       ├── ProcedureId
    │       ├── IndexId
    │       └── ConstraintId
    └── SchemaId + CatalogName
        └── one shared canonical-name dictionary
```

## Hierarchy and namespaces

The catalog has exactly one synthetic root directory. Its internal zero-length
name is unavailable from the regular and delimited user-name constructors, and
descriptor validation accepts it only for the root directory. User names must
be nonempty. The root contains schemas and has maximum child-directory depth 0.
`CatalogSnapshotBuilder::insert_child_directory` returns
`CatalogError::UnsupportedDirectoryDepth { maximum_depth: 0 }`.

Schema names share the root-level namespace. Within each schema, graphs, graph
types, binding tables, procedures, indexes, and constraints use one dictionary.
The same canonical spelling conflicts across object kinds in one schema. Two
different schemas may each contain that spelling.

## IDs and names

Each descriptor kind has a separate nonzero ID type. Catalog `GraphId`,
`GraphTypeId`, and `BindingTableId` are not the existing core storage/request
IDs. Constructors reject zero; `get` is the explicit raw-value boundary.

`CatalogName` retains two spellings:

- `display`: decoded source spelling for diagnostics;
- `canonical`: NFC of that spelling for dictionary identity.

The Unicode data version is 17.0.0. Regular identifiers use the UAX #31 R1-2
profile of R1-1: XID_Start/XID_Continue plus U+005F LOW LINE at start and
continuation. Delimited names use their decoded spelling and are not limited to
XID characters. Both forms reject General Category Co private-use characters.

Names are case-sensitive and receive no case folding. Canonically equivalent
spellings compare equal; compatibility-only equivalents do not. Ordering is
lexicographic by Unicode scalar value, matching selected ID022.

## Descriptor and snapshot equality

`CatalogDescriptor` equality compares the typed ID, kind, canonical and display
name fields, source form, parent, descriptor generation, creation metadata, and
payload. `same_identity` compares only the typed ID. Creation metadata records a
generation and an optional opaque principal; it has no clock or timestamp.

Payloads are storage-neutral markers plus catalog references needed by this
slice. Graph payloads may refer to a catalog `GraphTypeId`. Graph-type payloads
may carry `CoreGraphTypeBridge`, which explicitly wraps the current
`selene_core::GraphTypeId` without treating it as catalog identity. M02-PR05 owns
deletion of this bridge after core schema metadata migration.

`CatalogSnapshot` owns an `Arc` to immutable descriptor and `BTreeMap` state.
Cloning a snapshot is O(1); lookups borrow descriptors without cloning the
catalog. One-shot construction validates root shape, typed kind/payload/parent
relationships, duplicate IDs, shared-name conflicts, generation bounds, and
graph-to-graph-type references. A prior snapshot remains unchanged when a later
generation is built.

`CatalogTransaction` clones a prior snapshot into a pure draft, assigns the
checked next generation, and stages descriptor inserts and removals. It neither
serializes writers nor publishes state. `selene-db` owns those responsibilities
because catalog metadata cannot construct or own graph storage without breaking
the crate dependency boundary. Insertion rejects a typed ID already present in
the draft; it cannot replace a descriptor and silently rebind stable identity.

## Logical paths

Facade paths are typed absolute logical paths:

```text
CatalogPath  /selene
SchemaPath   /selene/public
ObjectPath   /selene/public/default
```

They are not filesystem paths and there is no string path parser in this slice.
Each segment is a `PathSegment` constructed with the regular or delimited
`CatalogName` validation rules. Mixed regular/delimited paths are built from
validated segments. Resolution compares canonical NFC names case-sensitively.
Listings sort by canonical name; stable ID is the tie-break key, although the
shared namespace normally prevents a canonical-name tie.

## One outer publication

`DatabaseInner` has one `ArcSwap<DatabaseState>` and one lifecycle-writer mutex.
`DatabaseState` contains all state that must agree at a catalog generation:

- the lower `CatalogSnapshot`;
- graph instances keyed by catalog `GraphId`;
- validated runtime graph-type definitions keyed by catalog `GraphTypeId`; and
- nonzero per-kind ID high-water marks.

A successful lifecycle command builds a complete replacement and performs one
outer swap. There is no separately published graph registry or graph-type map.
The per-kind high-water marks advance only in the replacement state, so a failed
draft can reuse its unpublished reservation. A dropped ID is never reused, and
recreating a path receives a new ID. Create operations validate the facade
descriptor against the unpublished replacement before the swap. After that
validation, `ArcSwap::store` is the final state-publication step and the method
returns the precomputed descriptor without another fallible lookup.

`CatalogReadSnapshot` loads that outer `Arc` in O(1). Its descriptor summaries
are facade-owned and do not expose `SharedGraph`, runtime schema definitions,
row positions, mutators, providers, or persistence types. Retaining a read
snapshot intentionally retains its complete `DatabaseState`, including runtime
graph and graph-type `Arc`s from that publication. A dropped graph can therefore
remain allocated until old snapshots are released, but it is absent from new
snapshots and cannot be reached by a `GraphHandle` request.

## Rust lifecycle API

`Database::catalog()` returns a shared `Catalog` handle that keeps the database
alive. Rust callers can:

- create and drop schemas, graphs, and graph types;
- resolve and list descriptors from `CatalogReadSnapshot`; and
- open a named graph as a `GraphHandle`.

`CreatePolicy::Strict` reports a duplicate as a structured error.
`CreatePolicy::IfNotExists` returns `CreateOutcome::AlreadyExists` only when the
requested kind exists at the same canonical path. An object of another kind in
the schema's shared namespace is still `CatalogObjectWrongKind`.

`DropPolicy::Strict` reports a missing object as a structured error.
`DropPolicy::IfExists` returns `DropOutcome::NotFound`. Duplicate and missing
no-op outcomes retain the same outer allocation and catalog generation.
Conditional policy applies only to the requested leaf object. A missing catalog
or schema parent is still `CatalogObjectNotFound`, including for `IfNotExists`
and `IfExists`; the service does not turn an invalid absolute path into a no-op.

`Database`, `Catalog`, `CatalogReadSnapshot`, `GraphHandle`, and the temporary
bootstrap `Session` are `Send + Sync`. Lifecycle writes still serialize through
the database writer mutex.

Graph-type binding is same-schema; graph creation rejects a type path from
another schema. All drops are RESTRICT. The service rejects:

- a graph with live nodes or edges, reporting both counts;
- a schema with catalog objects;
- a graph type referenced by one or more graphs; and
- the bootstrap `/selene/public` schema or `/selene/public/default` graph.

One graph type may constrain multiple graphs. The M02-PR03 public definition
surface supports property-free named node types with one or more defining
labels. Conversion to `selene_graph::GraphTypeDef`, validation, the temporary
`CoreGraphTypeBridge`, and core ID conversion stay private. Property definitions
and named edge endpoints are deferred rather than exposing lower schema types or
claiming full schema-builder parity.

## Graph handles and drop ordering

`GraphHandle` stores `Arc<DatabaseInner>`, catalog `GraphId`, and path metadata.
It does not store `Arc<SharedGraph>`. Each execution:

1. loads the outer state and finds the instance by stable ID;
2. acquires that instance's lifecycle read lease;
3. reloads the outer state and checks the same registered instance; and
4. executes before releasing the lease.

Named graph execution parses and plans through the existing GQL pipeline, then
rejects catalog, transaction-control, and session-control categories before
execution. Ordinary stateless reads and data mutations remain available. GQL
catalog DDL is M02-PR04 and will call the catalog service directly.

Graph drop acquires locks in this order: catalog writer, target lifecycle write
lease, then graph state. It rechecks registration after obtaining the lifecycle
lease. A request that already holds a read lease completes; drop waits and then
checks the resulting node and edge counts. A request that loses the race fails
as stale. Idle handles and facade `Session` values do not pin a graph instance.
There are no persistent named transactions in this slice.

Successful graph drop removes the descriptor and runtime entry in the same
outer publication. Procedure-derived graph state is cleared after that swap.
An old handle fails after drop and cannot alias a same-path replacement.

## Bootstrap and durability boundary

The initial outer state contains the synthetic catalog/root and
`/selene/public/default`. `BootstrapCatalog` remains the compatibility identity,
but the default `Session` resolves the same registry instance used by
`Catalog::open_graph`; it does not own a second graph. The compatibility session
still permits its existing `DROP GRAPH default` factory reset and projection
cleanup. Rust lifecycle drop protects the bootstrap schema and graph until
M02-PR05 removes the bridge.

Catalog lifecycle changes have no WAL, snapshot encoding, recovery, or crash
contract in M02-PR03. Test-only failures after descriptor staging, after graph
construction, and at each create path's final prepublication boundary prove that
the prior outer allocation, generation, runtime maps, and high-water marks
remain unchanged when publication does not occur.

## Later owners

- M02-PR04 owns GQL catalog DDL routing through this service.
- M02-PR05 owns bootstrap-catalog and `CoreGraphTypeBridge` deletion.
- M03 owns persistent sessions and transaction pinning.
- M09 owns persisted descriptor encoding, WAL/snapshot records, and recovery.
