//! Ordered graph checkpointing through the per-graph committer.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;

use selene_persist::{
    SectionCompression, SnapshotBuilder, SnapshotConfig, WalRotationOutcome, snapshot_path,
};

use crate::core_provider::CoreProvider;
use crate::index_provider::{IndexProvider, ProviderError};
use crate::{GraphError, GraphResult};

/// Configuration for one coordinated graph checkpoint.
///
/// The WAL directory, snapshot sequence, and durability barriers are owned by
/// the live graph's persistence protocol. Callers choose only the section
/// compression mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointConfig {
    /// Compression applied independently to each snapshot section.
    pub compression: SectionCompression,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            compression: SectionCompression::DEFAULT,
        }
    }
}

/// Result of a successful coordinated graph checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointOutcome {
    /// Highest WAL sequence covered by the snapshot and archived WAL.
    pub snapshot_sequence: u64,
    /// Final path of the durable snapshot.
    pub snapshot_path: PathBuf,
    /// Paths and sequence reported by the MANIFEST-backed WAL rotation.
    pub rotation: WalRotationOutcome,
}

/// Result plus the committer-health decision for one checkpoint work item.
pub(crate) struct CheckpointExecution {
    pub(crate) result: GraphResult<CheckpointOutcome>,
    pub(crate) poison_committer: bool,
}

/// Encode every provider at `generation`, then rotate the owned WAL.
///
/// Preparation failures happen before the MANIFEST protocol begins and are
/// therefore safe to report without poisoning the committer. Any error or
/// panic returned after rotation begins has an ambiguous writer/commit-point
/// state, so the caller must require reopen before accepting more writes.
pub(crate) fn execute(
    core: &CoreProvider,
    providers: &[Arc<dyn IndexProvider>],
    generation: u64,
    config: CheckpointConfig,
) -> CheckpointExecution {
    let prepared = std::panic::catch_unwind(AssertUnwindSafe(|| {
        prepare(core, providers, generation, config)
    }));
    let (builder, snapshot_sequence, final_snapshot_path) = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            return CheckpointExecution {
                result: Err(error),
                poison_committer: false,
            };
        }
        Err(payload) => {
            return CheckpointExecution {
                result: Err(provider_panic("checkpoint preparation", &payload)),
                poison_committer: false,
            };
        }
    };

    let rotated = std::panic::catch_unwind(AssertUnwindSafe(|| core.rotate_checkpoint(builder)));
    match rotated {
        Ok(Ok(rotation)) => CheckpointExecution {
            result: Ok(CheckpointOutcome {
                snapshot_sequence,
                snapshot_path: final_snapshot_path,
                rotation,
            }),
            poison_committer: false,
        },
        Ok(Err(error)) => CheckpointExecution {
            result: Err(error),
            poison_committer: true,
        },
        Err(payload) => CheckpointExecution {
            result: Err(GraphError::Durable {
                reason: format!(
                    "checkpoint WAL rotation panicked: {}; the graph must be reopened",
                    crate::panic_payload::describe(&payload)
                ),
            }),
            poison_committer: true,
        },
    }
}

fn prepare(
    core: &CoreProvider,
    providers: &[Arc<dyn IndexProvider>],
    generation: u64,
    config: CheckpointConfig,
) -> GraphResult<(SnapshotBuilder, u64, PathBuf)> {
    let target = core.checkpoint_target()?;
    let final_snapshot_path = snapshot_path(&target.dir, target.sequence);
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: target.dir,
        sequence: target.sequence,
        compression: config.compression,
        fsync: true,
    });
    let _callback_guard = crate::reentry::FanoutGuard::enter();
    for provider in providers {
        let provider_tag = provider.provider_tag();
        for sub_tag in provider.declared_sub_tags() {
            let bytes = provider.write_section_at_generation(*sub_tag, generation)?;
            builder.add_section(provider_tag.0, sub_tag.0, bytes)?;
        }
    }
    Ok((builder, target.sequence, final_snapshot_path))
}

fn provider_panic(phase: &str, payload: &Box<dyn std::any::Any + Send>) -> GraphError {
    GraphError::Provider(ProviderError::SerializationFailed {
        reason: format!(
            "{phase} panicked: {}",
            crate::panic_payload::describe(payload)
        ),
    })
}

#[cfg(test)]
#[path = "checkpoint/tests.rs"]
mod tests;
