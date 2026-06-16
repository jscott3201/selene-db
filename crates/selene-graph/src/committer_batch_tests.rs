//! v1.2 multi-writer BRIEF 2 — WAL group-commit tests (T1..T13).
//!
//! These exercise the batched committer driver: `CommitBatching::Off` is
//! behaviorally identical to BRIEF 1 (one fsync per commit), `On` coalesces a
//! contiguous `seal_seq` run into one group flush (the R1 fsync-before-publish
//! barrier) without losing or reordering, durable-before-visible holds at
//! commits and compacts, and a partial-append / flush failure / publish panic
//! poisons the whole in-flight run (every waiter Err'd, no hang).
//!
//! T1, T5, T7 are load-bearing.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use selene_core::{Change, DbString, GraphId, HlcTimestamp, LabelSet, PropertyMap};

use crate::committer_batch::CommitBatching;
use crate::durable_provider::DurableProvider;
use crate::error::GraphError;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{SeleneGraph, SharedGraph};

#[path = "committer_batch_tests/helpers.rs"]
mod helpers;
use helpers::*;

#[path = "committer_batch_tests/failures.rs"]
mod failures;
#[path = "committer_batch_tests/in_memory.rs"]
mod in_memory;

/// WAL-backed, compaction-boundary, and recovery group-commit tests (T7..T13).
#[path = "committer_batch_wal_tests.rs"]
mod wal;

/// F4 byte-cap + `encoded_estimate` tests (GRAPH-11).
#[path = "committer_batch_estimate_tests.rs"]
mod estimate;
