//! Change-subscriber impls for multi-index vector registries.

use selene_core::{Change, ChangeKind, ChangeKindSet};
use selene_graph::{ChangeSubscriber, ProviderError, ProviderTag};

use crate::registry::{HnswIndexRegistry, IvfIndexRegistry};
use crate::{VectorError, snapshot};

const SUBSCRIBED_KINDS: ChangeKindSet = ChangeKindSet::EMPTY
    .with(ChangeKind::NodeDeleted)
    .with(ChangeKind::EdgeDeleted);

impl ChangeSubscriber for HnswIndexRegistry {
    fn subscriber_tag(&self) -> ProviderTag {
        snapshot::VECT
    }

    fn change_kinds(&self) -> ChangeKindSet {
        SUBSCRIBED_KINDS
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        match change {
            Change::NodeDeleted { id } => {
                for (_name, provider) in self.entries() {
                    provider
                        .delete_node(*id)
                        .map_err(|error| delete_err("selene-vector registry", error))?;
                }
                Ok(())
            }
            // BRIEF-150: declarative truncate variants are NEVER fanned out to
            // subscribers — the producing side (runtime mutator / recovery CORE)
            // expands them into the per-row NodeDeleted/EdgeDeleted above. These
            // arms only keep the match exhaustive and must stay no-ops, so a
            // subscriber that lacks store access can never silently leak a
            // truncated set's derived vectors.
            Change::EdgeDeleted { .. }
            | Change::NodeCreated { .. }
            | Change::NodeUpdated { .. }
            | Change::NodePropertyRemoved { .. }
            | Change::NodeLabelRemoved { .. }
            | Change::NodesOfTypeTruncated { .. }
            | Change::EdgesOfTypeTruncated { .. }
            | Change::GraphReset { .. }
            | Change::EdgeCreated { .. }
            | Change::EdgeUpdated { .. }
            | Change::EdgePropertyRemoved { .. }
            | Change::SchemaChanged { .. }
            | Change::IndexExtensionEvent { .. } => Ok(()),
        }
    }
}

impl ChangeSubscriber for IvfIndexRegistry {
    fn subscriber_tag(&self) -> ProviderTag {
        snapshot::IVFP
    }

    fn change_kinds(&self) -> ChangeKindSet {
        SUBSCRIBED_KINDS
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        match change {
            Change::NodeDeleted { id } => {
                for (_name, provider) in self.entries() {
                    provider
                        .delete_node(*id)
                        .map_err(|error| delete_err("selene-vector-ivf registry", error))?;
                }
                Ok(())
            }
            // BRIEF-150: declarative truncate variants are NEVER fanned out to
            // subscribers — the producing side (runtime mutator / recovery CORE)
            // expands them into the per-row NodeDeleted/EdgeDeleted above. These
            // arms only keep the match exhaustive and must stay no-ops, so a
            // subscriber that lacks store access can never silently leak a
            // truncated set's derived vectors.
            Change::EdgeDeleted { .. }
            | Change::NodeCreated { .. }
            | Change::NodeUpdated { .. }
            | Change::NodePropertyRemoved { .. }
            | Change::NodeLabelRemoved { .. }
            | Change::NodesOfTypeTruncated { .. }
            | Change::EdgesOfTypeTruncated { .. }
            | Change::GraphReset { .. }
            | Change::EdgeCreated { .. }
            | Change::EdgeUpdated { .. }
            | Change::EdgePropertyRemoved { .. }
            | Change::SchemaChanged { .. }
            | Change::IndexExtensionEvent { .. } => Ok(()),
        }
    }
}

fn delete_err(context: &'static str, error: VectorError) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("{context} delete_node: {error:?}: {error}"),
    }
}
