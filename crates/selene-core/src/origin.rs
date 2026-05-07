//! Mutation origin metadata per spec 02 section 8.

use serde::{Deserialize, Serialize};

use crate::NodeId;

/// Origin of a graph mutation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Origin {
    /// Mutation originated in this selene-db instance.
    #[default]
    Local,
    /// Mutation was replicated from a federation peer.
    ///
    /// `source_node_id` names a federation node, not a property-graph node.
    /// Sequence values are caller-defined; `0` is allowed.
    Replicated {
        /// Federation source node.
        source_node_id: NodeId,
        /// Source sequence number.
        source_seq: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_replicated_distinct() {
        assert_ne!(
            Origin::Local,
            Origin::Replicated {
                source_node_id: NodeId::new(1),
                source_seq: 1,
            }
        );
    }

    #[test]
    fn replicated_carries_source_metadata() {
        let origin = Origin::Replicated {
            source_node_id: NodeId::new(7),
            source_seq: 42,
        };
        match origin {
            Origin::Replicated {
                source_node_id,
                source_seq,
            } => {
                assert_eq!(source_node_id, NodeId::new(7));
                assert_eq!(source_seq, 42);
            }
            Origin::Local => panic!("expected replicated origin"),
        }
    }

    #[test]
    fn default_is_local() {
        assert_eq!(Origin::default(), Origin::Local);
    }

    #[test]
    fn replicated_sequence_zero_is_allowed() {
        let origin = Origin::Replicated {
            source_node_id: NodeId::new(1),
            source_seq: 0,
        };
        assert!(matches!(origin, Origin::Replicated { source_seq: 0, .. }));
    }
}
