//! v1.2 multi-writer BRIEF 2 — WAL-backed group-commit tests (T7..T13).
//!
//! Child of `committer_batch_tests.rs`; shares its helpers + synthetic durable
//! providers via `use super::*`. Split out only to keep each test file under the
//! 700-LOC cap. T7 (compact-boundary durable-before-visible) is load-bearing.

// Everything else (Arc, Barrier, thread, Duration, Instant, atomics, the
// synthetic providers + helpers, SharedGraph, CommitBatching, …) is brought in
// from the parent test module's scope by this glob.
use super::*;
// Items NOT already in the parent's scope.
use std::collections::BTreeMap;

use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig};

#[path = "committer_batch_wal_tests/flush_helpers.rs"]
mod flush_helpers;
use flush_helpers::*;

#[path = "committer_batch_wal_tests/caps.rs"]
mod caps;
#[path = "committer_batch_wal_tests/flush_order.rs"]
mod flush_order;
#[path = "committer_batch_wal_tests/recovery.rs"]
mod recovery;
