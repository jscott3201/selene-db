use std::sync::Arc;

use selene_core::{DbString, GraphId, LabelSet, PropertyDiff, PropertyMap, Value, db_string};

use super::*;
use crate::graph::PropertyIndexEntry;
use crate::typed_index::{TypedIndexKind, TypedIndexValueError};

fn property_map(pairs: impl IntoIterator<Item = (DbString, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn decimal(value: &str) -> rust_decimal::Decimal {
    value.parse().expect("test decimal parses")
}

fn zoned(value: &str) -> jiff::Zoned {
    value.parse().expect("test zoned datetime parses")
}

fn entry(kind: TypedIndexKind) -> PropertyIndexEntry {
    PropertyIndexEntry::new(TypedIndex::new(kind), None)
}

fn rows(
    indexes: &PropertyIndexMap,
    label: DbString,
    property: DbString,
    value: &Value,
) -> RoaringRows {
    RoaringRows(
        indexes
            .get(&(label, property))
            .and_then(|index| index.index.lookup_eq(value))
            .map(std::borrow::Cow::into_owned)
            .unwrap_or_default(),
    )
}

struct RoaringRows(roaring::RoaringBitmap);

impl RoaringRows {
    fn contains(&self, row: u32) -> bool {
        self.0.contains(row)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[path = "property_index_tests/candidate_keys.rs"]
mod candidate_keys;
#[path = "property_index_tests/drift.rs"]
mod drift;
#[path = "property_index_tests/lifecycle.rs"]
mod lifecycle;
#[path = "property_index_tests/rebuild.rs"]
mod rebuild;
#[path = "property_index_tests/typed_values.rs"]
mod typed_values;
