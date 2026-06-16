use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use smallvec::smallvec;

use crate::*;

mod changes;
mod schema;
mod values;

fn rt<T>(value: &T)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + Debug,
{
    let bytes = postcard::to_allocvec(value)
        .unwrap_or_else(|error| panic!("postcard encode failed for {value:?}: {error:?}"));
    let decoded: T = postcard::from_bytes(&bytes)
        .unwrap_or_else(|error| panic!("postcard decode failed for {value:?}: {error:?}"));
    assert_eq!(&decoded, value);
}

fn dbs(value: &str) -> DbString {
    crate::db_string(value).unwrap()
}

fn graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).unwrap()
}

fn property_def(name: &str) -> PropertyDef {
    PropertyDef {
        name: dbs(name),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable: false,
        default: None,
        immutable: false,
        unique: false,
        record_fields: None,
    }
}

fn graph_type() -> GraphType {
    let mut graph_type = GraphType::new(graph_type_id(), dbs("serde.graph_type"));
    graph_type.node_types.insert(
        dbs("serde.node"),
        NodeTypeDef {
            labels: LabelSet::single(dbs("serde.node")),
            properties: smallvec![property_def("serde.node.name")],
            key: Some(NodeKey {
                property_names: smallvec![dbs("serde.node.name")],
            }),
            validation_mode: ValidationMode::Strict,
        },
    );
    graph_type.edge_types.insert(
        dbs("serde.edge"),
        EdgeTypeDef::new(
            dbs("serde.edge"),
            NodeTypeRef(dbs("serde.node")),
            NodeTypeRef(dbs("serde.node")),
        ),
    );
    graph_type.record_types.insert(
        RecordTypeId::new(1),
        RecordTypeDef {
            id: RecordTypeId::new(1),
            name: dbs("serde.record"),
            fields: smallvec![property_def("serde.record.field")],
        },
    );
    graph_type
}
