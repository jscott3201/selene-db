# Public API and examples

> Historical evidence for source `b8782bec34ff0b815b62711ac7e33cac09d8ea71` only.
> Not a 2.0 compatibility, signature, format-reader, alias, or migration contract.
> Benchmarks ran on an intentionally busy machine. They are non-green observations, not guards, comparisons, thresholds, or stable percentage baselines; issue #1137 / M08-PR06 owns future stable measurement.

Dispositions are semantic capability intent, not a 2.0 Rust path or signature promise.

Nightly rustdoc JSON (`includes_private=false`) supplies the inventory. The inventory contains local public rustdoc paths, items declared by local traits, and public inherent impl items only; trait and blanket impl entries and dependency-associated surfaces are excluded.

| Published package | Crate | Disposition | Owner | Paths | Declared items | Examples |
|---|---|---|---|---:|---:|---:|
| `selene-db-core` | `selene_core` | `internalize` | M02-PR05 and capability milestones M04/M10 | 504 | 361 | 0 |
| `selene-db-graph` | `selene_graph` | `internalize` | M02-PR05 and retrieval milestones M07/M10 | 614 | 440 | 1 |
| `selene-db-persist` | `selene_persist` | `replace` | M09-PR08 | 186 | 74 | 1 |
| `selene-db-algorithms` | `selene_algorithms` | `preserve` | M10-PR01 | 129 | 43 | 0 |
| `selene-db-gql` | `selene_gql` | `replace` | M05 and M06 | 946 | 310 | 0 |

## Unpublished support

`selene-db-testing` (`selene_testing`) has `publish = false`; it owns test, corpus, fixture, embedding-client, and benchmark support.

## Generated inventory

`docs/v2/baseline/api-inventory.json` SHA-256 `8fa3cf23f512e153699e7aeb1c66f11ca61b3921b2867579c326dca13351fa53`. It holds 2379 paths, 1228 local declared items, and 2 doc examples.

This generated file is the bounded D-021 exception; this report is its review entry point.
