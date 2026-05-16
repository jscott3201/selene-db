# BRIEF-109 v1.0.0 Vector Fixture Recipe

Fixture path:

- `crates/selene-testing/fixtures/v1_0_0_vector/snapshot.1.snap`
- `crates/selene-testing/fixtures/v1_0_0_vector/wal.log`

Regenerate from a clean v1.0.0-compatible checkout with:

```sh
cargo test -p selene-vector --test v1_0_0_forward_read regenerate_v1_0_0_vector_fixture -- --ignored
```

The generator writes legacy unwrapped `VECT` and `IVFP` snapshot sections plus
legacy unprefixed `selene-vector` / `selene-vector-ivf` WAL events. The PR2
forward-read test then loads those bytes through the named registries and
asserts they materialize as the `"default"` index.
