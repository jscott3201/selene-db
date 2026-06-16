use std::time::{Duration, Instant};

use selene_core::{
    CancellationChecker, CancellationToken, CoreError, GraphId, LabelDiff, LabelSet, PropertyDiff,
    PropertyMap, Value, VectorValue, db_string,
};

use super::*;
use crate::VectorIndexKind;

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[path = "tests/ivf.rs"]
mod ivf;
#[path = "tests/turbo_quant.rs"]
mod turbo_quant;

#[path = "tests/approximate.rs"]
mod approximate;
#[path = "tests/exact.rs"]
mod exact;
