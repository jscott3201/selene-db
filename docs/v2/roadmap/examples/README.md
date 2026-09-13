# Focused implementation examples

These examples are narrow teaching aids for Luna, not mandatory designs or production patches. Use the actual source, selected profile and owning PR to decide integration details.

## Current facade test pattern

[facade_smoke.rs](facade_smoke.rs) follows the observed public quickstart: create schema/graph, execute an insert, check write summary and query row count. It is intended to move into the actual facade integration-test target. It does not use future persistence APIs or pretend the current builder already opens a store. No Rust compiler was available/run during this review, so the file is source-grounded but **not compile-tested here**.

Extend that fixture in F02 with the actual newly implemented create/open methods, rather than inventing a method name in the plan. Add parameterized data, rollback, a required constraint, reopen and foreign-handle rejection as those contracts land. A green direct SeleneGraph test does not substitute for the facade path.

## Candidate algebra pattern

[candidate_intersection.rs](candidate_intersection.rs) shows lower-layer graph-owned stable-ID binding and checked typed-candidate intersection using the observed API. It is for internal graph/engine tests or advanced code, not a reason for downstream applications to bypass the stable facade. It is also **not compile-tested here**.

The important sequence is stable IDs → pinned graph bind → checked algebra → stable IDs. Do not serialize candidates or turn their private rows into a public iterator. Generic binding remains liveness-only; vector/property filtering is a separate operation.

## Executable supplemental selector reference

[path_selection_reference.py](path_selection_reference.py) is a dependency-free independent reference over **already supplied finite path bindings**. It demonstrates endpoint partitioning, qualifying before selecting, counted shortest versus shortest groups, and the difference between SIMPLE and TRAIL. Its [tests](test_path_selection_reference.py) are runnable:

```sh
python3 -B -m unittest discover -s examples -p 'test_path_selection_reference.py' -v
```

Run from the package root. The tests were executed during package preparation; see [PACKAGE-CHECK.md](../PACKAGE-CHECK.md) for the actual result.

### Boundaries of this reference

It does not parse GQL, construct automata, implement local/nested mode scopes, enforce graph-pattern match modes, resolve variable binding degree, validate actual graph adjacency, implement unbounded traversal or perform reduced-match deduplication. The caller supplies valid candidate bindings and required predicate truth values. It preserves supplied binding multiplicity rather than assuming identical element sequences represent the same binding.

F05-PR02 must still supply a full independent bounded graph-pattern oracle for the supported profile. Translate/adapt these small witnesses into Rust tests, but do not make Python a new runtime dependency or call these examples an ISO conformance suite.
