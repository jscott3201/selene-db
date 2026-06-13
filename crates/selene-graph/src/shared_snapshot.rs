//! Snapshot writing facade for [`SharedGraph`](crate::SharedGraph).

use selene_persist::{SnapshotBuilder, SnapshotConfig, SnapshotFinalizeOutcome};

use crate::{GraphResult, SharedGraph};

impl SharedGraph {
    /// Write one snapshot containing every registered provider section.
    ///
    /// This is the graph-layer facade over [`SnapshotBuilder`]. It walks the
    /// fixed provider registry, asks each provider to encode every declared
    /// subsection, and finalizes the snapshot envelope from `config`.
    ///
    /// Call after the commit whose state should be checkpointed has returned.
    /// Callers that need a deterministic checkpoint should avoid concurrent
    /// writes while this method is collecting provider sections.
    ///
    /// # Errors
    ///
    /// Returns [`crate::GraphError::Provider`] when a provider cannot encode one
    /// of its declared sections, or [`crate::GraphError::Persist`] when the
    /// snapshot envelope cannot be finalized.
    pub fn write_snapshot(&self, config: SnapshotConfig) -> GraphResult<SnapshotFinalizeOutcome> {
        let mut builder = SnapshotBuilder::new(config);
        for provider in self.index_providers() {
            let provider_tag = provider.provider_tag();
            for sub_tag in provider.declared_sub_tags() {
                let bytes = provider.write_section(*sub_tag)?;
                builder.add_section(provider_tag.0, sub_tag.0, bytes)?;
            }
        }
        builder.finalize().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use selene_core::{Change, GraphId};
    use selene_persist::{SectionCompression, SnapshotConfig, SnapshotReader, snapshot_path};

    use crate::SharedGraph;
    use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};

    const TEST_PROVIDER: [u8; 4] = *b"TST1";
    const TEST_SUB: [u8; 4] = *b"BODY";
    const TEST_SUB_TAGS: &[SubTag] = &[SubTag(TEST_SUB)];

    struct SnapshotOnlyProvider;

    impl IndexProvider for SnapshotOnlyProvider {
        fn provider_tag(&self) -> ProviderTag {
            ProviderTag(TEST_PROVIDER)
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            assert_eq!(sub_tag, SubTag(TEST_SUB));
            Ok(b"provider-body".to_vec())
        }

        fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            TEST_SUB_TAGS
        }
    }

    #[test]
    fn write_snapshot_includes_registered_provider_sections() {
        let dir = temp_dir("shared-snapshot");
        let provider = Arc::new(SnapshotOnlyProvider);
        let shared = SharedGraph::builder(GraphId::new(82_001))
            .with_provider(provider as Arc<dyn IndexProvider>)
            .build()
            .unwrap();

        let outcome = shared
            .write_snapshot(SnapshotConfig {
                dir: dir.clone(),
                sequence: 7,
                compression: SectionCompression::None,
                fsync: false,
            })
            .unwrap();

        assert_eq!(outcome.snapshot_seq, 7);
        assert!(outcome.section_count > 1);
        let mut reader = SnapshotReader::open(&snapshot_path(&dir, 7)).unwrap();
        assert_eq!(
            reader.read_section(TEST_PROVIDER, TEST_SUB).unwrap(),
            b"provider-body"
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "selene-graph-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        dir
    }
}
