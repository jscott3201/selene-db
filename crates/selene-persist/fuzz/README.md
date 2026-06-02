# selene-persist fuzz targets

This directory is a `cargo-fuzz` package for PERSIST-26 (node 814): fuzzing the
four hand-rolled crash-recovery decoders against untrusted bytes. The invariant
each target asserts is that **arbitrary input bytes decode to either `Ok` or a
typed `PersistError` — never a panic, OOM, or hang.**

It is intentionally excluded from the root workspace because `cargo-fuzz`
expects a separate nightly-only package; its `libfuzzer-sys` dependency never
enters the root `Cargo.lock` (so `cargo-deny` and `THIRDPARTY.md` are unaffected).

Targets — each drives a `from_bytes`-style slice entry point so the fuzzer feeds
bytes directly (no temp file per iteration):

- `decode_manifest` — `Manifest::decode` (`SLMF`)
- `decode_wal` — `WalReader::from_bytes` (`SLDB`); drains the iterator and calls
  `.body()` on each entry, so it exercises the full per-entry decode: fixed +
  replicated + principal header, payload-length-vs-remaining bound, checksum, the
  bounded zstd decompress, and the postcard `Vec<Change>` decode.
- `decode_audit` — `AuditLog::decode_all` (`SLAU`); the record-scan loop with its
  torn-tail truncation.
- `decode_snapshot` — `SnapshotReader::decode_envelope` (`SLSN`); file header +
  section table + offset/layout validation. The section *payload* rkyv decode is
  out of scope — it lives downstream in `selene-graph` behind a `CheckBytes`
  bound, never in this crate.

Run a single target locally (Linux; `cargo-fuzz`/libFuzzer require a Linux target
triple):

```bash
cargo +nightly fuzz run decode_manifest -- -max_total_time=60 -timeout=20 -max_len=65536
```
