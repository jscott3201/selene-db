# Identity and runtime references

M04-PR01 defines the facade identity boundary without changing the existing
WAL, snapshot, property, serde, or rkyv formats. The current facade is in-memory;
M09 owns persisted facade open and recovery.

## Identity domains

Selene DB keeps four domains distinct:

1. **Catalog objects.** `SchemaId`, facade `GraphId`, and `GraphTypeId` identify
   logical catalog objects. A drop/recreate at the same path allocates a new ID.
2. **Stable graph elements.** `NodeId` and `EdgeId` identify semantic graph
   elements independently of physical storage position. The facade deliberately
   re-exports these two types. It never exports `RowIndex`.
3. **Runtime references.** `GraphRef`, `NodeRef`, and `EdgeRef` are opaque,
   lifetime-free handles issued only after facade validation. They carry an
   opaque process-local `DatabaseId`, a stable graph ID, and, for an element,
   its stable element ID.
4. **Private physical rows.** Dense node and edge rows are storage details. They
   may change during compaction without changing a live stable element ID or a
   facade reference.

`DatabaseId` identifies one built facade instance. It is not a persisted store
identity, store epoch, UUID, or globally routable address. Cloned `Database`
values and sessions opened from them retain that identity. Independent
`Database::builder().build()` calls receive distinct identities. It has no
public raw constructor or numeric accessor; debug output is diagnostic, not a
parsing contract.

## Equality, generation, and liveness

Reference equality and hashing use only:

- `GraphRef`: database ID + graph ID;
- `NodeRef`: database ID + graph ID + node ID;
- `EdgeRef`: database ID + graph ID + edge ID.

Generation is deliberately absent. `CatalogGeneration` and `GraphGeneration`
are checked monotonic cache/state tokens: they can invalidate catalog snapshots,
plans, candidates, or other derived state without changing semantic reference
identity. The session plan-cache stamp uses typed database/catalog/schema/graph/
graph-type identities and typed catalog/graph generations while retaining
schema-version, procedure-registry, profile, and session-characteristic tokens.

Dereference still checks current liveness under the graph lifecycle lease. A
same-database reference for the session's selected graph resolves while the
catalog graph and element are live. A deleted element can retain an internal ID
mapping for tombstone behavior; that mapping does not make the element live.
Compaction may remap physical rows but does not invalidate a surviving element.

The following runtime cases return GQLSTATUS `42002` (invalid reference):

- another database instance, including an old handle presented to a newly built
  or eventually reopened facade;
- another selected graph;
- a deleted or absent node/edge;
- a dropped graph; or
- an old graph/element handle after drop and recreation at the same path.

GQLSTATUS `42N03` (undefined reference) remains an analyzer/name-resolution
diagnostic. Runtime handle validation does not change analyzer behavior.

## Durability and reopen

Facade handles are process-local and intentionally implement no serde, rkyv, or
property-storage contract. Stable catalog, graph, node, and edge IDs are the
semantic IDs that future supported persistence/reopen paths must preserve, but a
newly opened facade `Database` receives a new `DatabaseId`. An old runtime handle
therefore fails `42002`; it never retargets merely because recovered IDs or paths
match. M09 defines and tests the actual persisted open/recovery authority.

The infallible in-memory builder allocates `DatabaseId` from a non-wrapping safe
process-local atomic. It panics only if that finite process-local domain is
exhausted rather than reusing an identity.

## Temporary lower-engine bridge

`selene_core::Value::{GraphRef, NodeRef, EdgeRef}` remain bare-ID, durable,
property-compatible lower-engine variants. They are not the facade handle types
and do not silently convert to them. An embedder must extract the stable ID and
call the selected session's issuance API, which validates database/graph
ownership and liveness.

This bridge is explicit and temporary:

- M05-PR03 owns semantic/runtime request, result, and carrier migration to the
  database-scoped reference contract.
- M09-PR08 owns deletion of the legacy encoded `Value` variants and codecs once
  their runtime and persistence consumers are gone.

No M04-PR01 change alters the existing encoded `Value` discriminants, property
schema semantics, WAL/snapshot codecs, or graph recovery format.
