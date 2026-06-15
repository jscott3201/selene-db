use selene_core::{
    CancellationChecker, DbString, GraphId, HnswIndexConfig, LabelDiff, LabelSet, PropertyDiff,
    PropertyMap, Value, VectorMetric, VectorValue,
};

use super::VectorIndex;
use crate::{ApproximateVectorSearchOptions, GraphError, SharedGraph, VectorIndexKind};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).unwrap()
}

fn vector(components: &[f32]) -> Value {
    Value::Vector(VectorValue::new(components.to_vec()).unwrap())
}

fn props(pairs: impl IntoIterator<Item = (DbString, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/memory_rebuild.rs"]
mod memory_rebuild;
#[path = "tests/metadata.rs"]
mod metadata;
