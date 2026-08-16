//! Group-commit batch body for the single per-graph committer thread
//! (v1.2 multi-writer, BRIEF 2).
//!
//! This module holds everything the committer's batched driver
//! ([`crate::committer::run_committer`]) needs **per contiguous run** of
//! commits, so `committer.rs` stays a thin driver under the 700-LOC file cap:
//!
//! - [`CommitBatching`] — the embedder-facing group-commit policy.
//! - [`drain_contiguous_batch`] — Phase 1: form the contiguous-`seal_seq` run
//!   starting at `next_publish_seq` by appending (fsync deferred) each
//!   [`crate::write_txn::SealedCommit`], capped by count + aggregate bytes.
//! - [`flush_and_publish_batch`] — Phase 2 (the R1 fsync-before-publish
//!   barrier: ONE [`crate::write_txn::flush_durables`] for the whole run) +
//!   Phase 3/4 (publish + ack each member in `seal_seq` order).
//! - [`ack_appended_with_error`] — fail every already-appended-but-unpublished
//!   member of a run so no reply [`std::sync::mpsc::SyncSender`] is dropped
//!   silently (which would hang its `recv()`).
//!
//! # Why a single flush per run (the R1 barrier)
//!
//! For every commit `C` in a run `B`: append (bytes written, fsync deferred) →
//! ONE [`flush_durables`](crate::write_txn::flush_durables) fsyncs **all** of
//! `B` → [`publish_appended`](crate::write_txn::publish_appended) makes `C`
//! visible → ack delivers `durable_at`. The flush strictly precedes both store
//! and ack, so neither the published snapshot nor `durable_at` is observable
//! before fsync (durable-before-visible). An appended-but-unflushed commit is
//! never published alone, so it stays invisible **and** unacked.
//!
//! On a **crash** its bytes are lost with the page cache — exactly what
//! durability permits. On a **reopen** they are not: the frame is complete and
//! checksum-valid, recovery's tail repair truncates only a torn tail, and the
//! frame is not torn, so it replays. Poisoning the committer forces a reopen,
//! not a crash, which is why every poison-exit ack carries
//! [`GraphError::IndeterminateOutcome`] instead of a rollback.
//!
//! # OFF == BRIEF 1
//!
//! With [`CommitBatching::Off`] the run length is capped at 1
//! ([`BatchLimits::resolve`]), so a run is always one append + one flush + one
//! publish + one ack — the same syscall sequence, in the same order, as BRIEF
//! 1's `EveryN(1)` per-commit fsync. OFF is the degenerate `N=1` case of the
//! *identical* batched code, never a second path.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::committer::{
    CommitterHandles, Work, committer_dead, drain_buffer_with_error, run_protected,
    unwrap_protected,
};
use crate::error::GraphResult;
use crate::write_txn::{
    AppendedCommit, CommitOutcome, SealedCommit, append_sealed, flush_durables, publish_appended,
};

/// Group-commit batching policy for the single per-graph committer thread.
/// Default [`Off`](CommitBatching::Off).
///
/// Composes with [`crate::SharedGraphBuilder::with_wal`]: the committer-managed
/// WAL is always driven in [`SyncPolicy::OnFlushOnly`](crate::SyncPolicy::OnFlushOnly)
/// regardless of the [`WalConfig`](crate::WalConfig) passed (the committer owns
/// fsync), so the chosen `CommitBatching` policy — not the `WalConfig` —
/// determines how many commits coalesce into one fsync.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommitBatching {
    /// One append + one fsync per commit. Behaviorally identical to BRIEF 1
    /// (durable-before-visible, same crash semantics, one fsync per commit). The
    /// default.
    #[default]
    Off,
    /// Coalesce up to `max_commits` contiguous commits (capped by aggregate
    /// `max_bytes`) into one group fsync. A single commit larger than `max_bytes`
    /// is still taken alone (the `>= 1` progress rule), so the byte cap only
    /// bounds *accumulation* and never rejects a commit.
    On {
        /// Maximum number of contiguous commits coalesced into one fsync.
        max_commits: std::num::NonZeroUsize,
        /// Aggregate (estimated) payload-byte cap for one batch — a DoS bound on
        /// how much un-fsynced work the committer accumulates before a barrier.
        max_bytes: u64,
    },
}

impl CommitBatching {
    /// Conventional ON policy: coalesce up to 64 commits / 8 MiB aggregate.
    pub const DEFAULT_ON: Self = Self::On {
        // SAFETY-of-unwrap: 64 != 0. `NonZeroUsize::new(..).unwrap()` is const
        // since Rust 1.83, evaluated at compile time.
        max_commits: match std::num::NonZeroUsize::new(64) {
            Some(value) => value,
            None => unreachable!(),
        },
        max_bytes: 8 * 1024 * 1024,
    };
}

/// Resolved per-run caps derived from a [`CommitBatching`] policy.
///
/// `Off` resolves to `(1, u64::MAX)` so the batched driver runs exactly one
/// commit per run (OFF == BRIEF 1); `On { max_commits, max_bytes }` resolves to
/// `(max_commits, max_bytes)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BatchLimits {
    /// Maximum commits in one run (clamped to 1 when `Off`).
    pub(crate) max_commits: usize,
    /// Aggregate estimated payload-byte cap for one run.
    pub(crate) max_bytes: u64,
}

impl BatchLimits {
    /// Resolve a [`CommitBatching`] policy into concrete per-run caps.
    pub(crate) fn resolve(batching: CommitBatching) -> Self {
        match batching {
            CommitBatching::Off => Self {
                max_commits: 1,
                max_bytes: u64::MAX,
            },
            CommitBatching::On {
                max_commits,
                max_bytes,
            } => Self {
                max_commits: max_commits.get(),
                max_bytes,
            },
        }
    }
}

/// Documented structural estimate of a sealed commit's encoded WAL-payload size,
/// used only for the F4 aggregate-byte cap.
///
/// The exact encoded length is computed by `encode_changes` deep inside
/// `WalWriter::append` and is **not** threaded back out (that would require
/// changing the [`DurableProvider`](crate::DurableProvider) trait signature for
/// a non-load-bearing accounting number). Instead this approximates each
/// [`Change`] at a flat [`PER_CHANGE_ESTIMATE_BYTES`] plus a small per-commit
/// header allowance. The estimate is intentionally conservative-high so the
/// `max_bytes` cap triggers a barrier *no later* than the real bytes would; the
/// per-entry hard cap (`entry_header::ensure_payload_len`) still bounds any
/// single commit's real size, so this estimate only governs how many commits
/// accumulate before a flush — never correctness.
pub(crate) fn encoded_estimate(sealed: &SealedCommit) -> u64 {
    const PER_COMMIT_HEADER_BYTES: u64 = 64;
    let changes = sealed.changes.len() as u64;
    PER_COMMIT_HEADER_BYTES.saturating_add(changes.saturating_mul(PER_CHANGE_ESTIMATE_BYTES))
}

/// Flat per-[`Change`] byte allowance for [`encoded_estimate`]. Sized generously
/// above the typical encoded change so the F4 cap errs toward flushing sooner.
const PER_CHANGE_ESTIMATE_BYTES: u64 = 256;

/// Outcome of [`drain_contiguous_batch`]: the appended (fsync-deferred) run plus
/// whether the driver must stop after handling it.
pub(crate) enum BatchDrain {
    /// A (possibly empty) contiguous run of appended commits to flush + publish.
    /// Empty only when the head at `next_publish_seq` was a gap (no work to do
    /// this iteration) — the caller simply loops back to `recv()`.
    Run {
        /// The appended-but-unflushed run, in ascending `seal_seq` order.
        batch: Vec<AppendedCommit>,
    },
    /// An [`append_sealed`] failed partway through the run. The committer is
    /// already poisoned and every member listed here (including any successfully
    /// appended earlier in this run) must be acked with `committer_dead`, the
    /// reorder buffer drained, and the loop exited.
    AppendFailed {
        /// Members already appended before the failure — must be error-acked.
        appended: Vec<AppendedCommit>,
    },
}

/// Phase 1 — form a contiguous-`seal_seq` run starting at `next_publish_seq` by
/// [`append_sealed`]ing each [`Work::Commit`] (fsync deferred).
///
/// The run is bounded by [`BatchLimits`] (count + aggregate estimated bytes,
/// both conjunctive) with a `>= 1` progress rule (a single commit over
/// `max_bytes` is still taken alone). It ends at:
/// - a **gap** (the next `seal_seq` is absent after a non-blocking `try_recv`
///   refill) — the caller blocks on `recv()` for it;
/// - **snapshot-maintenance or checkpoint work** at head — a hard flush
///   boundary: the pending commit run is flushed/published first, then the solo
///   item runs.
///
/// `next_publish_seq` is advanced past every commit appended here. On an
/// [`append_sealed`] error the already-appended members are returned in
/// [`BatchDrain::AppendFailed`] for error-acking; the failed commit itself has
/// already been error-acked and `next_publish_seq` advanced past it.
pub(crate) fn drain_contiguous_batch(
    receiver: &Receiver<Work>,
    reorder: &mut BTreeMap<u64, Work>,
    next_publish_seq: &mut u64,
    limits: BatchLimits,
    handles: &CommitterHandles,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) -> BatchDrain {
    let mut batch: Vec<AppendedCommit> = Vec::new();
    let mut batch_bytes: u64 = 0;
    loop {
        // Opportunistically refill the contiguous slot from the channel without
        // blocking, so a run can extend past what was already buffered. We only
        // drain until the next contiguous seq is present (or the channel is
        // momentarily empty); anything pulled is buffered by seal_seq, never
        // published out of order.
        if !reorder.contains_key(next_publish_seq) {
            refill_until_contiguous(receiver, reorder, *next_publish_seq);
        }
        match reorder.get(next_publish_seq) {
            // Gap: the next seal_seq is in flight on a session that already
            // dropped the write lock and WILL send it. End the run; the caller
            // blocks on recv() for it (no deadlock — see the module + committer
            // docs).
            None => return BatchDrain::Run { batch },
            // Snapshot maintenance and checkpoints are hard flush boundaries:
            // never co-batched. Flush/publish the pending commit run first; the
            // caller re-iterates with the solo item now at head + an empty
            // batch.
            Some(
                Work::Compact { .. } | Work::VectorIndexRebuild { .. } | Work::Checkpoint { .. },
            ) => {
                return BatchDrain::Run { batch };
            }
            Some(Work::Commit { sealed, .. }) => {
                let estimate = encoded_estimate(sealed);
                // Cap check with the >= 1 progress rule: only stop if the batch
                // already has a member (so a single over-cap commit is taken
                // alone, never rejected).
                if !batch.is_empty()
                    && (batch.len() >= limits.max_commits
                        || batch_bytes.saturating_add(estimate) > limits.max_bytes)
                {
                    return BatchDrain::Run { batch };
                }
                // Pop the head commit and advance the publish cursor.
                let Some(Work::Commit { sealed, reply }) = reorder.remove(next_publish_seq) else {
                    unreachable!("checked Work::Commit at next_publish_seq above");
                };
                *next_publish_seq += 1;
                match run_protected(|| append_sealed(sealed, &handles.durable_providers)) {
                    Ok(Ok(mut appended)) => {
                        appended.reply = Some(reply);
                        batch_bytes = batch_bytes.saturating_add(estimate);
                        batch.push(appended);
                    }
                    failed => {
                        // Partial-batch append failure: poison, Err THIS waiter,
                        // and hand the already-appended members back for
                        // error-acking. None of the run is flushed or published.
                        // `unwrap_protected` poisons + yields the GraphError; the
                        // append produced no AppendedCommit, so re-wrap the error
                        // into this waiter's CommitOutcome reply channel.
                        let error = match unwrap_protected(failed, poisoned) {
                            Ok(_appended) => unreachable!("append-failure arm is never Ok"),
                            // Already indeterminate: `unwrap_protected` set the
                            // poison bit and reclassified. `append_sealed` walks
                            // the providers in order, so one may have taken the
                            // record before the next refused, and the WAL
                            // writer's own rollback is best-effort.
                            Err(error) => error,
                        };
                        let _: Result<(), std::sync::mpsc::SendError<GraphResult<CommitOutcome>>> =
                            reply.send(Err(error));
                        return BatchDrain::AppendFailed { appended: batch };
                    }
                }
            }
        }
    }
}

/// Non-blocking refill: pull ready items from the channel into the reorder
/// buffer until either the contiguous `next` seq is present or the channel has
/// no immediately-ready item. Disconnect is a no-op here (the outer driver's
/// blocking `recv()` observes it next iteration).
fn refill_until_contiguous(
    receiver: &Receiver<Work>,
    reorder: &mut BTreeMap<u64, Work>,
    next: u64,
) {
    loop {
        match receiver.try_recv() {
            Ok(work) => {
                let seq = work.seal_seq();
                reorder.insert(seq, work);
                if seq == next {
                    return;
                }
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
        }
    }
}

/// Phase 2 (R1 barrier) + Phase 3/4 — flush the whole run once, then publish +
/// ack each member in `seal_seq` order.
///
/// Returns `true` when the committer must **stop** (a flush error or a
/// publish-tail panic poisoned the engine and the buffer was drained); the
/// caller then returns from the loop. An empty `batch` is a no-op returning
/// `false` (the run ended on a gap; nothing to flush).
///
/// Failure handling mirrors [`crate::write_txn`]'s contract:
/// - **flush Err** ⇒ publish NOTHING, poison, error-ack every member, drain the
///   buffer, stop.
/// - **publish panic** ⇒ `catch_unwind` poisons; error-ack the remaining members
///   (this one already had its reply consumed), drain the buffer, stop.
pub(crate) fn flush_and_publish_batch(
    batch: Vec<AppendedCommit>,
    reorder: &mut BTreeMap<u64, Work>,
    handles: &CommitterHandles,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    if batch.is_empty() {
        return false;
    }

    // Phase 2: the single group flush == the R1 fsync-before-publish barrier.
    match run_protected(|| flush_durables(&handles.durable_providers)) {
        Ok(Ok(())) => {}
        failed => {
            // Poison via unwrap_protected (its returned Err is the flush error,
            // already surfaced to each member below via committer_dead); publish
            // NOTHING from the run.
            let _ = unwrap_protected(failed, poisoned);
            ack_appended_with_error(batch);
            drain_buffer_with_error(reorder);
            return true;
        }
    }

    // Phase 3+4: publish-all in seal_seq (== batch) order, ack-all. Durable now,
    // so each publish makes its commit visible only after the barrier above.
    let mut batch = batch.into_iter();
    while let Some(mut appended) = batch.next() {
        let reply = appended.reply.take();
        // publish_appended is infallible (returns CommitOutcome) — the only way
        // to fail is a panic (store / debug assert / fan-out boundary), which
        // catch_unwind converts to a Durable error + poison.
        let result = unwrap_protected(
            run_protected(|| {
                Ok(publish_appended(
                    appended,
                    &handles.snapshot,
                    &handles.schema_version,
                    &handles.providers,
                ))
            }),
            poisoned,
        );
        if let Some(reply) = reply {
            // `unwrap_protected` already reclassified. A panic here is past
            // the group flush, so this member's record is fsynced and will
            // replay — the sharpest case for `40003`: the commit is durable and
            // only its publication was lost.
            let _ = reply.send(result);
        }
        if poisoned.load(Ordering::Acquire) {
            // A publish-tail panic poisoned us. Every remaining member is
            // appended + flushed (durable) but unpublished; we cannot trust the
            // diverged in-memory graph, so Err them and drain the buffer.
            ack_appended_with_error(batch.collect());
            drain_buffer_with_error(reorder);
            return true;
        }
    }
    false
}

/// Fail every member of a run that was appended but never published, so no reply
/// [`SyncSender`] is dropped silently (which would hang its `recv()` with a
/// `RecvError`). Used on a partial-append failure, a flush failure, and a
/// publish-tail panic — every poison exit drains its in-flight run through here.
pub(crate) fn ack_appended_with_error(batch: Vec<AppendedCommit>) {
    for appended in batch {
        if let Some(reply) = appended.reply {
            let _: Result<(), std::sync::mpsc::SendError<GraphResult<CommitOutcome>>> =
                reply.send(Err(committer_dead()));
        }
    }
}

#[cfg(test)]
#[path = "committer_batch_tests.rs"]
mod tests;
