use super::*;
use parking_lot::Mutex;
use selene_core::{
    Change, HlcTimestamp, LabelSet, PropertyMap, PropertyValueType, SchemaChange, db_string,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::CORE_PROVIDER_TAG;
use crate::index_provider::ProviderError;
use crate::typed_index::TypedIndexKind;

struct TestProvider {
    tag: ProviderTag,
    seen: Mutex<Vec<Change>>,
}

impl TestProvider {
    fn new(tag: ProviderTag) -> Self {
        Self {
            tag,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl IndexProvider for TestProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: crate::SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: crate::SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.seen.lock().push(change.clone());
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[crate::SubTag] {
        &[]
    }
}

struct FailingDurableProvider;

impl DurableProvider for FailingDurableProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"FAIL")
    }

    fn write_commit(
        &self,
        _principal: Option<&Arc<[u8]>>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "synthetic durable failure".to_owned(),
        })
    }
}

fn sample_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("shared.type").unwrap(),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("shared.node").unwrap(),
            key_labels: LabelSet::single(db_string("SharedNode").unwrap()),
            properties: vec![crate::PropertyTypeDef {
                name: db_string("shared.name").unwrap(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
                record_field_types: None,
            }],
            validation_mode: crate::ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

mod basic;
mod builder;
mod concurrency;
mod from_graph;
mod schema_version;
