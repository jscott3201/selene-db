//! Stateful extension-provider protocol per spec 06.

use std::fmt;

use selene_core::Change;

/// Stable 4-byte ASCII identifier for an [`IndexProvider`] registration.
///
/// Reserved tag space:
/// - `META`/`NODE`/`EDGE`/`SCMA` are reserved for engine-owned snapshot sections.
/// - First-party extension allocations include `VECT`, `FULL`, `TIMS`, `GRPR`.
/// - Other ASCII uppercase 4-byte sequences are provider-allocated.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderTag(
    /// Raw 4-byte provider tag.
    pub [u8; 4],
);

impl fmt::Display for ProviderTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tag(self.0, f)
    }
}

/// 4-byte subsection identifier within a provider's snapshot footprint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubTag(
    /// Raw 4-byte provider-local subsection tag.
    pub [u8; 4],
);

impl fmt::Display for SubTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_tag(self.0, f)
    }
}

/// Stateful extension hook for derived state participation.
///
/// Spec 06 originally used `&mut self` for [`IndexProvider::read_section`] and
/// [`IndexProvider::on_change`]. `selene-graph` stores providers as
/// `Arc<dyn IndexProvider>`, so providers use interior mutability for owned
/// state. The engine guarantees serialized calls per graph.
pub trait IndexProvider: Send + Sync + 'static {
    /// Stable 4-byte ASCII tag uniquely identifying this provider.
    fn provider_tag(&self) -> ProviderTag;

    /// Snapshot bootstrap for one provider-owned section.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the payload is missing, malformed, or
    /// inconsistent with provider state.
    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError>;

    /// Snapshot publish for one provider-owned section.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when serialization cannot produce a stable
    /// section payload.
    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError>;

    /// Observe one committed mutation.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] for provider-local failures. Live commits log
    /// and continue after these errors because the graph snapshot has already
    /// been published.
    fn on_change(&self, change: &Change) -> Result<(), ProviderError>;

    /// Provider-owned snapshot subsection tags.
    ///
    /// Empty means the provider consumes mutation events but owns no persisted
    /// snapshot state.
    fn declared_sub_tags(&self) -> &[SubTag];
}

/// Errors returned by [`IndexProvider`] implementations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ProviderError {
    /// Provider payload could not be decoded or validated.
    #[error("invalid provider payload: {reason}")]
    #[diagnostic(code(SLENE_G_010))]
    InvalidPayload {
        /// Human-readable provider failure reason.
        reason: String,
    },

    /// A required snapshot subsection was absent.
    #[error("snapshot section missing: {sub_tag:?}")]
    #[diagnostic(code(SLENE_G_011))]
    SectionMissing {
        /// Missing subsection tag.
        sub_tag: SubTag,
    },

    /// Provider state could not be serialized.
    #[error("provider serialization failed: {reason}")]
    #[diagnostic(code(SLENE_G_012))]
    SerializationFailed {
        /// Human-readable serialization failure reason.
        reason: String,
    },

    /// Snapshot recovery found a section whose provider is not registered.
    #[error("unknown provider for tag {tag:?} sub_tag {sub_tag:?}")]
    #[diagnostic(code(SLENE_G_013))]
    UnknownProvider {
        /// Unknown provider tag.
        tag: ProviderTag,
        /// Provider-local subsection tag.
        sub_tag: SubTag,
    },

    /// Provider state or registration is inconsistent.
    #[error("provider state inconsistency: {reason}")]
    #[diagnostic(code(SLENE_G_014))]
    Inconsistent {
        /// Human-readable inconsistency reason.
        reason: String,
    },
}

fn fmt_tag(bytes: [u8; 4], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if bytes.iter().all(|byte| byte.is_ascii_graphic()) {
        for byte in bytes {
            f.write_str(char::from(byte).encode_utf8(&mut [0; 4]))?;
        }
        Ok(())
    } else {
        write!(
            f,
            "0x{:02X}{:02X}{:02X}{:02X}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;
    use rstest::rstest;
    use selene_core::{LabelSet, NodeId, PropertyMap};

    use super::*;
    use crate::{GraphError, GraphResult};

    struct RecordingProvider {
        tag: ProviderTag,
        changes: Mutex<Vec<Change>>,
    }

    impl RecordingProvider {
        fn new(tag: ProviderTag) -> Self {
            Self {
                tag,
                changes: Mutex::new(Vec::new()),
            }
        }
    }

    impl IndexProvider for RecordingProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
            self.changes.lock().push(change.clone());
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    fn assert_send_sync_static<T: Send + Sync + 'static>() {}

    #[test]
    fn provider_tag_equality_and_ordering() {
        let vect = ProviderTag(*b"VECT");
        let full = ProviderTag(*b"FULL");
        assert_eq!(vect, ProviderTag(*b"VECT"));
        assert!(full < vect);
        assert_eq!(vect.to_string(), "VECT");
    }

    #[test]
    fn sub_tag_equality_and_ordering() {
        let graph = SubTag(*b"GRPH");
        let vecs = SubTag(*b"VECS");
        assert_eq!(graph, SubTag(*b"GRPH"));
        assert!(graph < vecs);
        assert_eq!(graph.to_string(), "GRPH");
    }

    #[rstest]
    #[case(ProviderError::InvalidPayload { reason: "bad".to_owned() })]
    #[case(ProviderError::SectionMissing { sub_tag: SubTag(*b"MISS") })]
    #[case(ProviderError::SerializationFailed { reason: "io".to_owned() })]
    #[case(ProviderError::UnknownProvider { tag: ProviderTag(*b"VECT"), sub_tag: SubTag(*b"VECS") })]
    #[case(ProviderError::Inconsistent { reason: "duplicate".to_owned() })]
    fn provider_error_gqlstatus_mappings(#[case] provider_error: ProviderError) {
        let graph_error = GraphError::Provider(provider_error);
        assert_eq!(graph_error.gqlstatus(), "XX500");
    }

    #[test]
    fn dummy_provider_with_interior_mutability() -> GraphResult<()> {
        assert_send_sync_static::<RecordingProvider>();
        let provider = RecordingProvider::new(ProviderTag(*b"TEST"));
        provider.on_change(&Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::new(),
            properties: PropertyMap::new(),
        })?;
        assert_eq!(provider.changes.lock().len(), 1);
        assert_eq!(provider.provider_tag(), ProviderTag(*b"TEST"));
        Ok(())
    }
}
