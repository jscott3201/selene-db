# Final 1.x executable baseline

> Historical evidence for source `b8782bec34ff0b815b62711ac7e33cac09d8ea71` only.
> Not a 2.0 compatibility, signature, format-reader, alias, or migration contract.
> Benchmarks ran on an intentionally busy machine. They are non-green observations, not guards, comparisons, thresholds, or stable percentage baselines; issue #1137 / M08-PR06 owns future stable measurement.

This package fixes deterministic inventories and one observed run. Raw evidence stays under ignored `target/v2-baseline/`.

## Provenance

| Identity | Value |
|---|---|
| Source commit | `b8782bec34ff0b815b62711ac7e33cac09d8ea71` |
| Source tree | `087cc06836c560ac55e3579f651dba775f6fe32e` |
| Source commit time | `2026-08-16T20:44:15-04:00` |
| 1.x coordinate | `1.4.0` |
| Initial harness base | `b7ea652bbf79b48efb6c9ae63deb485f26a69bb9` |
| Initial base tree | `8e3df4fd4225df1df12128ce81d51f2fe565eb0b` |
| Capture HEAD | `b7ea652bbf79b48efb6c9ae63deb485f26a69bb9` |
| Capture HEAD tree | `8e3df4fd4225df1df12128ce81d51f2fe565eb0b` |
| Runner SHA-256 | `4d3d5ea335cb63637c023cd7281d596b24b1471087f996ac75e1eeda21512d4c` |
| Helper SHA-256 | `8cdb5498e8133eddb8d9c29fba61b7c6ddaa1d06c99dc08266602e1fcf40c20d` |
| Archive refs | `pending_owner_only` |

Harness file hashes are separate from source provenance; no self-referential final harness commit is claimed.

## Reports

- [`gates.md`](gates.md): commands, tests, corpora, and fuzz.
- [`public-api.md`](public-api.md): published APIs and semantic dispositions.
- [`formats.md`](formats.md): persistence, packages, procedures, and feature register.
- [`benchmarks.md`](benchmarks.md): absolute Criterion observations.

Inventory: 2379 paths, 1228 local trait/inherent items, 2 doc examples, 0 Cargo example targets.
Command dispositions: failed: 1, not_applicable: 0, passed: 43, skipped: 0, unavailable: 0.

## Commands

```bash
scripts/v2-baseline.sh capture
scripts/v2-baseline.sh install
scripts/v2-baseline.sh verify
```

Capture uses an isolated detached local clone, disables Cargo network access, removes service variables, and retains redacted logs.
