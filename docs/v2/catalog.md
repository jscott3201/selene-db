# Catalog descriptor and read-snapshot contract

M02-PR02 defines catalog identity and immutable metadata. It does not create,
drop, persist, or open named graph instances.

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

## Later owners

- M02-PR03 owns the serialized writer transaction, lifecycle API, graph-instance
  registry, and atomic publication.
- M02-PR05 owns bootstrap-catalog and temporary core-schema bridge deletion.
- M09 owns persisted descriptor encoding, WAL/snapshot records, and recovery.
