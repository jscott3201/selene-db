# BRIEF-106 — what-not-why trail

## crates/selene-persist/src/writer.rs

### writer.rs:528

- Before: `// returns PrincipalTooLarge; scan_existing must treat as torn-tail.`
- After: `// reports PrincipalTooLarge, and scan_existing must treat that as a torn tail rather than a durable corruption.`
- Rationale: kept the non-obvious recovery classification, but rewrote the line so it explains why the error is interpreted that way rather than restating a return value.

## crates/selene-gql/src/parser/grammar.pest

### grammar.pest:405

- Before: `// does not greedily match as a user function. A plain call like any(x)`
- After: `// avoids greedy matching as a user function. A plain call like any(x)`
- Rationale: kept the PEG-ordering invariant because it explains why list iteration rules are ordered before generic calls; rewrote to avoid the what-not-why candidate prefix.
