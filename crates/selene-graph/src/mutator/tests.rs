use selene_core::{
    Change, DbString, GraphId, LabelDiff, LabelSet, NodeId, PredefinedValueType, PropertyDiff,
    PropertyMap, PropertyValueType, SchemaChange, Value, ValueType, db_string,
};

use super::*;
use crate::SharedGraph;
use crate::graph_types::{GraphTypeDef, NodeTypeDef, PropertyTypeDef, ValidationMode};
use crate::type_validator::{EntityId, TypeViolation};

mod edges;
mod indexes;
mod properties;
mod proptests;
mod transactions;

fn empty_node(mutator: &mut Mutator<'_, '_>) -> NodeId {
    mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .expect("create_node ok")
}
