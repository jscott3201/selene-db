# Selene DB 2.0 risk register

| ID | Risk | Severity | Mitigation | Owner |
|---|---|---:|---|---|
| R-01 | Claim overstatement | Critical | Generated profile, evidence, and claim gate. | M01, M10 |
| R-02 | Catalog/graph ownership cycle | Critical | Leaf descriptors; graph instances remain in the database registry; review the crate DAG. | M02 |
| R-03 | Partial catalog/data publication | Critical | One staged transaction bundle and publish point; failure and model tests. | M03, M06, M09 |
| R-04 | Physical rows escape | High | Private row types, typed candidate sets, API/lint gates. | M04 |
| R-05 | Undirected edge duplicates identity | Critical | One edge ID with explicit directionality and mixed-graph tests. | M04, M07 |
| R-06 | Compiler layers collapse | High | Immutable syntax/semantic/logical/physical contracts. | M05, M06 |
| R-07 | Batch engine regresses small queries | High | One-row and small-batch guards beside scan/join/memory rows. | M06 |
| R-08 | Path explosion or invalid pruning | Critical | Finite-result analysis, reference evaluator, resource errors, differential tests. | M07 |
| R-09 | Constraint accepts an incomplete index | Critical | Activation requires a ready, complete provider; no ordinary scan fallback. | M08 |
| R-10 | Map choice optimizes one workload side | High | Adjacent read/write/memory A/B evidence and section baselines. | M08 |
| R-11 | Pathname TOCTOU retargets persistence | Critical | Anchored directory capability and deterministic replacement tests. | M09 |
| R-12 | Commit outcome exceeds durable evidence | Critical | Explicit phases, watermarks, and crash-state model. | M09 |
| R-13 | Recovery repairs ambiguity destructively | Critical | Non-destructive default and typed corruption classes. | M09 |
| R-14 | Hidden 1.x reader restores compatibility | High | Header-only rejection and stale-code searches. | M00, M09, M10 |
| R-15 | Derived provider becomes durable authority | Critical | Primary values and catalog registrations remain authoritative. | M09, M10 |
| R-16 | Agent or PR scope drift | High | One invariant, size cap, stop conditions, non-draft PR, and review. | All |

## Dependency gates

- No architecture implementation before M00 governance and baseline exit.
- No manual formal-claim ledger after the M01 cutover.
- No session or compiler ownership rooted in one `SharedGraph` after M02.
- No unresolved names or public rows enter physical execution after M05/M04.
- No active constraint lacks a complete backing index after M08.
- No format 2 recovery publishes partial state or resolves independent paths.
- No release retains temporary bridges, old format/API paths, or stronger claim
  wording than its generated evidence.

Release requires the target architecture, selected conformance evidence,
durability/crash matrix, balanced performance guards, complete repository
gates, a clean exact candidate SHA, assistant PASS, and owner action.
