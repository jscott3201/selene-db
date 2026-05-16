# BRIEF-109 Recovery Decoder Paths

Recovery surfaces audited in PR2:

| Path | Decoder failure variants | Test coverage |
|---|---|---|
| WAL legacy `0x56` payload prefix | truncated magic, unknown magic, legacy payload decode/validation failure | `v1_0_0_snapshot_and_wal_forward_read_as_default`, payload unit tests |
| WAL named `0x01` prefix | empty payload, truncated name length/name/body, non-UTF8 name, inner body not `V...` | `multi_index_lifecycle` named replay tests; payload unit tests |
| WAL lifecycle `VECC` | wrong/truncated magic, invalid kind length/string, invalid config length/body, trailing bytes | `hnsw_named_lifecycle_wal_replay_and_snapshot_recover`, `create_index_idempotent_and_conflicting_config_errors` |
| WAL lifecycle `VECX` | wrong magic, default-drop rejection, missing index | `drop_default_rejected_and_list_indexes_reports_both_kinds` |
| Snapshot v1 wrapper | bad version, zero entries, duplicate name, truncated name/config/section, config decode failure, trailing bytes | `registry_default_transparency`, `multi_index_lifecycle` snapshot recovery |
| Snapshot v0 fallback | legacy `VG`/`VV`/`VQ`/`VC`/`VI`/`VP` first bytes; empty legacy QUNT as default empty section | `v1_0_0_snapshot_and_wal_forward_read_as_default`, registry transparency tests |

The recovery ordering invariant is covered by replaying captured lifecycle
events before named mutation events in `hnsw_named_lifecycle_wal_replay_and_snapshot_recover`.
An upsert for a name without a lifecycle create remains an error through
`VECT registry has no vector index '<name>'`.
