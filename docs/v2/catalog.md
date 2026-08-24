# Catalog lifecycle and selected-session contract

M02-PR02 defines catalog identity and immutable metadata. M02-PR03 adds the
in-memory lifecycle service, M02-PR04 routes GQL lifecycle statements through
that service, and M02-PR05 makes catalog-selected sessions the facade execution
root. Catalog changes are not durable yet; M09 owns their persisted
representation and recovery.

## Hierarchy and namespaces

The catalog has one synthetic root directory. Its internal empty name cannot be
constructed as a user name. The root contains schemas and has no child
directories. Within each schema, graphs, graph types, binding tables,
procedures, indexes, and constraints share one canonical-name dictionary. The
same canonical spelling conflicts across object kinds in one schema; different
schemas may reuse it.

The initial `DatabaseState` contains only the catalog descriptor and synthetic
root. It contains no schema, graph, graph type, runtime graph, or hidden working
graph. Kind-local high-water marks start at zero, so the first published user
schema, graph, and graph type each receive ID 1. Published IDs are never reused.

Each descriptor kind has a separate nonzero ID type. Catalog `GraphId` and
`GraphTypeId` are catalog identities, not lower storage IDs. `CatalogName`
retains display spelling for diagnostics and NFC canonical spelling for
dictionary identity. Names are case-sensitive and receive no compatibility or
case folding.

`CatalogPayload` is storage-neutral. A graph payload may refer to a catalog
`GraphTypeId`; a graph-type payload is only a kind marker. Validated runtime
graph-type definitions remain private facade state keyed by catalog
`GraphTypeId`.

## Logical paths

Facade paths are typed absolute logical paths, not filesystem paths:

```text
CatalogPath  /selene
SchemaPath   /selene/memory
ObjectPath   /selene/memory/episodes
```

Each segment is constructed through regular or delimited `CatalogName`
validation. Resolution compares canonical NFC names case-sensitively. Listings
sort by canonical name and use stable ID as the tie-break key.

## One outer publication

`DatabaseInner` has one `ArcSwap<DatabaseState>` and one lifecycle-writer mutex.
`DatabaseState` contains everything that must agree at a catalog generation:

- the immutable catalog snapshot;
- runtime graph instances keyed by catalog `GraphId`;
- validated graph-type definitions keyed by catalog `GraphTypeId`; and
- kind-local ID high-water marks.

A successful lifecycle command builds one complete replacement and performs one
outer swap. Descriptor state and runtime maps are never published separately.
Failed construction or validation retains the prior allocation, generation,
runtime maps, procedure state, and high-water marks. A same-path recreation or
replacement receives a fresh stable ID.

`CatalogReadSnapshot` loads the outer state in O(1). Its facade summaries do not
expose lower graph instances, schema definitions, row positions, mutators,
providers, or persistence types. Retaining a read snapshot retains the complete
publication it observed, but an old runtime instance is not reachable by new
requests after drop or replacement.

## Rust lifecycle API

`Database::catalog()` returns a shared `Catalog` handle. Callers can create and
drop schemas, graphs, and graph types, and resolve or list descriptors from a
read snapshot.

`CreatePolicy::Strict` reports duplicates. `IfNotExists` is a no-op only when
the requested kind already exists at the same canonical path; another kind in
the shared namespace remains a wrong-kind error. `OrReplace` is supported for
graphs and graph types. It applies the same RESTRICT admission as drop and
publishes the removed and created identities together. Schema replacement is
not an ISO form and is rejected.

`DropPolicy::Strict` reports a missing object. `IfExists` returns a no-op only
for a missing leaf; a missing parent remains an invalid reference. All drops are
RESTRICT. The service rejects a graph with live nodes or edges, a nonempty
schema, and a graph type referenced by a graph.

Graph-type binding is same-schema. The public definition slice supports
property-free named node types with one implied singleton key label. Properties,
edges and endpoints, explicit key labels, aliases, and COPY OF, LIKE, inline, or
external graph-type sources remain unsupported.

## Selected facade sessions

`Database::session(&ObjectPath) -> Result<Session>` resolves a graph in the
current catalog publication. A facade `Session` stores only the database,
catalog graph ID, absolute graph path, and schema path. It does not store or
expose a lower graph handle. The type is owned, lifetime-free, `Send + Sync`, and
stateless between requests.

Each execution uses the stable graph ID rather than rebinding by path:

1. load the outer state and find the runtime instance by stable ID;
2. acquire that instance's lifecycle read lease;
3. reload the outer state and confirm the same registered instance; and
4. execute before releasing the lease.

Drop and replacement use catalog writer, target lifecycle write lease, then
graph state lock order. They recheck registration after acquiring the lifecycle
lease. A request that already holds a read lease completes before drop; drop
then observes its writes when applying RESTRICT. A request that loses the race
fails as `StaleGraphSelection`. Dropping and recreating the same path never
makes an old session refer to the replacement.

An idle session does not pin a lifecycle lease. A successful graph drop or
replacement removes the old descriptor and runtime entry in the same outer
publication, then clears procedure-derived graph state. Failed publication does
not clear that state.

## GQL catalog dispatch

A selected session parses, analyzes, and plans each source once under the graph
request lease. Ordinary data statements execute through that lower pipeline. A
plan containing one database-catalog command is returned to the facade before
execution; the request lease is released, and the facade dispatches the command
to the same `Catalog` lifecycle service used by Rust callers. Catalog mutation
therefore never occurs while a graph request read lease is held.

Transaction and session controls remain rejected at the facade boundary. M03
owns persistent session context and transaction state. A bare lower executor
session rejects every database-catalog command with the implementation-defined
status `5GQL0`; it does not reinterpret `DROP GRAPH` as a storage reset.

ISO absolute references do not spell the facade catalog:

- schema `/x` maps to `SchemaPath /selene/x`;
- graph or graph type `/x/g` maps to `ObjectPath /selene/x/g`;
- relative graph or graph type `g` maps to the selected session graph's schema;
- one-segment absolute object references and deeper directory shapes are
  invalid (`42002`).

The parser carries source forms and decoded spellings. `PathSegment`
construction remains the validation choke point for both GQL and Rust paths.
Lifecycle resolution occurs under the writer mutex from typed paths, not
pre-resolved IDs.

Dropping the selected graph through GQL is valid when it is empty. The command
publishes the drop and the session becomes stale. A nonempty selected graph is
still rejected by RESTRICT.

## Outcomes and statuses

Successful database-catalog statements return
`ExecutionOutcome::OmittedResult`. Rust and GQL lifecycle calls share the same
structured facade errors and GQLSTATUS mapping.

| Case | Outcome or error | GQLSTATUS |
|---|---|---|
| successful create or drop | omitted result | `00001` |
| successful create-or-replace | omitted result, one publication | `00001` |
| duplicate or missing conditional no-op | omitted result, no publication | `00001` |
| `DROP GRAPH IF EXISTS` missing leaf | omitted result | `01G03` |
| strict duplicate | `CatalogObjectAlreadyExists` | `42N10` |
| missing object or parent, wrong kind, invalid path shape | structured reference error | `42002` |
| invalid catalog name | `InvalidCatalogName` | `42001` |
| RESTRICT dependency | `CatalogRestrictViolation` | `G1000` |
| stale selected identity | `StaleGraphSelection` | none |
| unsupported catalog source or stateful control | `FeatureNotSupported` | `42N01` |

No-op outcomes retain the same outer allocation and generation. Unsupported
clauses are rejected before a command exists and cannot mutate catalog state.

## Durability and later owners

Catalog lifecycle changes currently have no WAL, snapshot encoding, recovery,
or crash contract. Existing lower `Mutator::factory_reset` remains available
for engine and recovery use, but it is not a GQL database-catalog route.

- M03 owns persistent session context, parameters, cancellation, and
  transaction pinning.
- Later milestones may broaden catalog object families without changing
  selected graph identity semantics.
- M09 owns persisted descriptor encoding, WAL and snapshot records, and
  recovery.
