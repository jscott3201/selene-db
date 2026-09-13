# selene-persist fuzz targets

This directory is a `cargo-fuzz` package for current format-2 crash-recovery
decoders and bounded control envelopes against untrusted bytes. The invariant
each target asserts is that **arbitrary input bytes decode to either `Ok` or a
typed error — never a panic, OOM, or hang.**

It is intentionally excluded from the root workspace because `cargo-fuzz`
expects a separate nightly-only package; its `libfuzzer-sys` dependency never
enters the root `Cargo.lock` (so `cargo-deny` and `THIRDPARTY.md` are unaffected).

Targets — each drives a `from_bytes`-style slice entry point so the fuzzer feeds
bytes directly (no temp file per iteration):

- `decode_manifest` — current format-2 `EmptyManifest::decode` (`SLEM`) and
  `logical_stream::validate_manifest` (`SLLM` / `SLDM` / `SLRM`), with raw bytes,
  integrity-repaired arbitrary bodies and a mutated valid rotating-manifest seed
  to reach bounded payload, identity, lineage and selected-layout validation.
  This performs no filesystem selection and does not decode legacy `SLMF` files.
- `decode_logical` — new format-2 `logical_frame::decode` and complete
  `LogicalTransaction::decode`, with raw semantic input and repaired header/body
  BLAKE3 integrity to reach the payload, identity/length and Zstd paths.
  It also mutates a complete named-type revision over two retained graphs and
  repairs framing integrity before `ReplayState::apply_frame`, exercising named
  type/body/binding coherence and retained graph validation without publication.
- `decode_logical_value` — explicit format-2 stored-value tags, round trips and
  bounded hostile recursive containers. Neither target calls the legacy WAL decoder.
- `decode_control` — `CurrentSelector::decode` / `EmptyManifest::decode`
  (`SLCU` / `SLEM`), including the 4096-byte cap, postcard framing, checksums,
  generation/epoch/lineage validation and canonical single-component selector names.
- `decode_wal` — `logical_frame::decode` (`SLTXN2`); consumes concatenated frames
  with exact sequence/digest advancement and bounded `LogicalTransaction` decode.
  Raw bytes, valid raw/Zstd frames, truncated streams and integrity-repaired header
  mutations cover all three trusted end classifications without opening files.
- `decode_snapshot` — `logical_snapshot::required_length` / `decode` (`SLSNP2`);
  raw bytes, valid envelopes, integrity-repaired mutations, truncation and excess
  bytes exercise length, identity, selected-boundary and complete-digest checks.
- `decode_logical_snapshot` — format-2 snapshot framing plus bounded checkpoint
  section decoding and materialization, including mutated named-type/graph seeds.

F06-QUAL-02 retires `decode_audit`: F02-PR08 removed the legacy `AuditLog` and
`SLAU` record decoder, and audit logging is excluded from this alpha. The only
remaining audit handling is read-only legacy-header recognition/rejection, covered
by control tests; there is no format-2 audit decoder to substitute. Do not restore
legacy APIs or repurpose an audit target to claim coverage of a nonexistent log.
Both release/nightly selections now run all seven current targets, including the
existing logical transaction/value/snapshot semantic targets. Historical results
in `docs/v2/baseline/` remain evidence of their captured source only, not current
target registrations or release qualification.

Run a single target on the native Linux or macOS host; do not cross-compile:

```bash
cargo +nightly fuzz run decode_manifest -- -max_total_time=60 -timeout=20 -max_len=65536
cargo +nightly fuzz run decode_control -- -max_total_time=60 -timeout=20 -max_len=65536
cargo +nightly fuzz build
cargo +nightly fuzz run decode_logical -- -max_total_time=60 -max_len=16384
cargo +nightly fuzz run decode_logical_value -- -max_total_time=60 -max_len=8192
```
