# selene-gql fuzz targets

This directory is a `cargo-fuzz` package for spec 10 section 4 parser fuzzing.
It is intentionally excluded from the root workspace because `cargo-fuzz`
expects a separate nightly-only package.

Run:

```bash
cargo +nightly fuzz run parse_gql -- -max_total_time=60
```
