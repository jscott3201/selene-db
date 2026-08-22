# Post-GA and deferred backlog

These items are outside the 64-work-item initial program. Their absence is a
scope decision, not work to absorb opportunistically.

| Deferred item | Boundary |
|---|---|
| Network server and wire protocol | 2.0 remains embedded; protocol, auth, and connection semantics need separate architecture. |
| Bundled user administration | The engine exposes embedder policy, not an auth service. |
| MVCC, multiple writers, weaker isolation | Keep serializable single-writer publication until workload evidence requires redesign. |
| Nested directories or multiple catalogs | Initial catalog is one synthetic root with schemas only. |
| 1.x reader, import, or migration | Rebuild from source data; no compatibility path is planned. |
| Loadable dynamic or WASM ABI | Native procedures stay in-tree; a third-party ABI needs security and versioning design. |
| Distributed query or graph execution | No clustering, sharding, replication, or distributed transactions. |
| JIT or data-centric code generation | Build and measure the batch interpreter first. |
| Join/group/sort disk spill | Initial engine uses bounded memory and typed resource errors. |
| JSON containment index | Implement deterministic scalar-path expression indexes first. |
| Weighted path language extensions | Keep standard path semantics separate from namespaced algorithms. |
| Durable ANN/text accelerator internals | Derived providers rebuild from primary data and registrations. |
| Production GPU execution | Experimental benchmark rows are not release requirements. |
| Every optional GQL feature | Admit only implication-closed, evidence-complete profile increments. |

A deferred item enters planning only after focused research or an ADR defines
need, ownership, public and persisted compatibility, conformance impact,
safety/threat model, evidence, and PR-sized slices.
