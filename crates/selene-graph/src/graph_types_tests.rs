use selene_core::{PropertyValueType, Value, VectorValue, db_string};

use super::*;

fn label(name: &str) -> DbString {
    db_string(name).unwrap()
}

fn property(name: &str) -> PropertyTypeDef {
    PropertyTypeDef {
        name: label(name),
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
    }
}

fn list_default(items: Vec<PropertyDefaultValue>) -> PropertyDefaultValue {
    PropertyDefaultValue::List(items.into_iter().map(Box::new).collect())
}

fn valid_type() -> GraphTypeDef {
    GraphTypeDef {
        name: label("types.graph"),
        node_types: vec![
            NodeTypeDef {
                name: label("types.person"),
                key_labels: LabelSet::single(label("Person")),
                properties: vec![property("name")],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: label("types.company"),
                key_labels: LabelSet::single(label("Company")),
                properties: vec![property("name")],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![EdgeTypeDef {
            name: label("types.works_at"),
            label: label("WORKS_AT"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(1),
            properties: vec![property("since")],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn validate_accepts_well_formed_type() {
    assert!(valid_type().validate().is_ok());
}

#[test]
fn property_default_float_descriptors_materialize_finite_values() {
    assert_eq!(
        PropertyDefaultValue::Float(1.5_f64.to_bits())
            .to_value()
            .unwrap(),
        Value::Float(1.5)
    );
    assert_eq!(
        PropertyDefaultValue::Float32(2.25_f32.to_bits())
            .to_value()
            .unwrap(),
        Value::Float32(2.25_f32)
    );
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Float(-0.0)),
        Some(PropertyDefaultValue::Float(0.0_f64.to_bits()))
    );
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Float32(-0.0)),
        Some(PropertyDefaultValue::Float32(0.0_f32.to_bits()))
    );
}

#[test]
fn property_default_float_descriptors_reject_non_finite_values() {
    assert!(PropertyDefaultValue::from_value(&Value::Float(f64::INFINITY)).is_none());
    assert!(PropertyDefaultValue::from_value(&Value::Float32(f32::NAN)).is_none());
    assert!(matches!(
        PropertyDefaultValue::Float(f64::NAN.to_bits()).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("FLOAT property default is not finite")
    ));
    assert!(matches!(
        PropertyDefaultValue::Float32(f32::INFINITY.to_bits()).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("FLOAT32 property default is not finite")
    ));
}

#[test]
fn property_default_exact_numeric_descriptors_materialize_values() {
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Uint(u64::MAX)),
        Some(PropertyDefaultValue::Uint(u64::MAX))
    );
    assert_eq!(
        PropertyDefaultValue::Uint(u64::MAX).to_value().unwrap(),
        Value::Uint(u64::MAX)
    );
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Int128(i128::MIN)),
        Some(PropertyDefaultValue::Int128(i128::MIN))
    );
    assert_eq!(
        PropertyDefaultValue::Int128(i128::MIN).to_value().unwrap(),
        Value::Int128(i128::MIN)
    );
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Uint128(u128::MAX)),
        Some(PropertyDefaultValue::Uint128(u128::MAX))
    );
    assert_eq!(
        PropertyDefaultValue::Uint128(u128::MAX).to_value().unwrap(),
        Value::Uint128(u128::MAX)
    );
    let decimal = Value::Decimal("123.450".parse().unwrap());
    assert_eq!(
        PropertyDefaultValue::from_value(&decimal),
        Some(PropertyDefaultValue::Decimal(label("123.450")))
    );
    assert_eq!(
        PropertyDefaultValue::Decimal(label("123.450"))
            .to_value()
            .unwrap(),
        decimal
    );
}

#[test]
fn property_default_decimal_descriptor_rejects_invalid_text() {
    assert!(matches!(
        PropertyDefaultValue::Decimal(label("not-decimal")).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("DECIMAL property default is invalid")
    ));
}

#[test]
fn property_default_vector_descriptor_materializes_values() {
    let vector = VectorValue::new(vec![1.0, -0.0, 2.5]).unwrap();
    assert_eq!(
        PropertyDefaultValue::from_value(&Value::Vector(vector)),
        Some(PropertyDefaultValue::Vector(vec![
            1.0_f32.to_bits(),
            0.0_f32.to_bits(),
            2.5_f32.to_bits(),
        ]))
    );
    assert_eq!(
        PropertyDefaultValue::Vector(vec![1.0_f32.to_bits(), 2.5_f32.to_bits()])
            .to_value()
            .unwrap(),
        Value::Vector(VectorValue::new(vec![1.0, 2.5]).unwrap())
    );
}

#[test]
fn property_default_vector_descriptor_rejects_invalid_bits() {
    assert!(matches!(
        PropertyDefaultValue::Vector(Vec::new()).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("VECTOR property default is invalid")
    ));
    assert!(matches!(
        PropertyDefaultValue::Vector(vec![f32::INFINITY.to_bits()]).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("VECTOR property default is invalid")
    ));
}

#[test]
fn property_default_list_descriptors_materialize_nested_values() {
    let value = Value::List(vec![
        Value::String(label("alpha")),
        Value::List(vec![Value::Int(1), Value::Int(2)]),
        Value::Vector(VectorValue::new(vec![1.0, 0.0]).unwrap()),
    ]);
    let expected = list_default(vec![
        PropertyDefaultValue::String(label("alpha")),
        list_default(vec![
            PropertyDefaultValue::Integer(1),
            PropertyDefaultValue::Integer(2),
        ]),
        PropertyDefaultValue::Vector(vec![1.0_f32.to_bits(), 0.0_f32.to_bits()]),
    ]);

    assert_eq!(
        PropertyDefaultValue::from_value(&value),
        Some(expected.clone())
    );
    assert_eq!(expected.to_value().unwrap(), value);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&expected).unwrap();
    let decoded = rkyv::from_bytes::<PropertyDefaultValue, rkyv::rancor::Error>(&bytes).unwrap();
    assert_eq!(decoded, expected);
    assert!(PropertyDefaultValue::from_value(&Value::List(vec![Value::Float(f64::NAN)])).is_none());
}

#[test]
fn property_default_temporal_descriptors_materialize_values() {
    let zoned = Value::ZonedDateTime(Box::new(
        "2026-05-07T12:34:56-04:00[America/New_York]"
            .parse()
            .unwrap(),
    ));
    assert_eq!(
        PropertyDefaultValue::from_value(&zoned),
        Some(PropertyDefaultValue::ZonedDateTime(label(
            "2026-05-07T12:34:56-04"
        )))
    );
    assert_eq!(
        PropertyDefaultValue::ZonedDateTime(label("2026-05-07T12:34:56-04"))
            .to_value()
            .unwrap()
            .variant_name(),
        "ZonedDateTime"
    );
    assert_eq!(
        PropertyDefaultValue::LocalDateTime(label("2026-05-07T12:34:56"))
            .to_value()
            .unwrap(),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        PropertyDefaultValue::Date(label("2026-05-07"))
            .to_value()
            .unwrap(),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        PropertyDefaultValue::ZonedTime(label("12:34:56-04"))
            .to_value()
            .unwrap()
            .variant_name(),
        "ZonedTime"
    );
    assert_eq!(
        PropertyDefaultValue::LocalTime(label("12:34:56"))
            .to_value()
            .unwrap(),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        PropertyDefaultValue::Duration(label("PT1H2S"))
            .to_value()
            .unwrap(),
        Value::Duration(Box::new("PT1H2S".parse().unwrap()))
    );
}

#[test]
fn property_default_temporal_descriptors_reject_invalid_text() {
    assert!(matches!(
        PropertyDefaultValue::Date(label("not-date")).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("DATE property default is invalid")
    ));
    assert!(matches!(
        PropertyDefaultValue::ZonedTime(label("12:34:56")).to_value(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("ZONED TIME property default requires a time zone displacement")
    ));
}

#[test]
fn validate_rejects_duplicate_node_type_names() {
    let mut graph_type = valid_type();
    graph_type.node_types[1].name = graph_type.node_types[0].name.clone();
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("duplicate node type name")
    ));
}

#[test]
fn validate_rejects_edge_index_out_of_range() {
    let mut graph_type = valid_type();
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::NodeType(99);
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("references node type index")
    ));
}

#[test]
fn validate_rejects_duplicate_property_names() {
    let mut graph_type = valid_type();
    graph_type.node_types[0].properties.push(property("name"));
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("duplicate node property name")
    ));
}

#[test]
fn validate_rejects_empty_node_label_set() {
    let mut graph_type = valid_type();
    graph_type.node_types[0].key_labels = LabelSet::new();
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("empty label set")
    ));
}

#[test]
fn validate_rejects_duplicate_node_key_label_sets() {
    // Two node types with identical key_labels would leave the second
    // unreachable via find_node_type_index (first-match wins); rejecting
    // at construction prevents silent mis-typing of edges and properties.
    let mut graph_type = valid_type();
    graph_type.node_types[1].key_labels = graph_type.node_types[0].key_labels.clone();
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("duplicates the key_labels")
    ));
}

#[test]
fn validate_rejects_ambiguous_overlapping_edge_endpoints() {
    let mut graph_type = valid_type();
    graph_type.edge_types.push(EdgeTypeDef {
        name: label("types.works_at_any"),
        label: label("WORKS_AT"),
        source_node_type: EdgeEndpointDef::Any,
        target_node_type: EdgeEndpointDef::NodeType(1),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
    });

    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("ambiguous edge type endpoints")
    ));
}

#[test]
fn lookup_returns_matching_elements() {
    let graph_type = valid_type();
    let person = LabelSet::single(label("Person"));
    assert_eq!(
        graph_type
            .find_node_type(&person)
            .map(|node_type| node_type.name.clone()),
        Some(label("types.person"))
    );
    assert_eq!(graph_type.find_node_type_index(&person), Some(0));
    assert_eq!(
        graph_type.node_type_index_for(label("types.company")),
        Some(1)
    );
    assert_eq!(
        graph_type
            .find_edge_type(label("WORKS_AT"), 0, 1)
            .map(|edge_type| edge_type.name.clone()),
        Some(label("types.works_at"))
    );
}

#[test]
fn without_helpers_remove_named_type_without_reindexing() {
    let graph_type = valid_type();

    let without_node = graph_type
        .without_node_type(label("types.person"))
        .expect("node type removed");
    assert_eq!(without_node.node_types.len(), 1);
    assert_eq!(
        without_node.edge_types[0].source_node_type,
        EdgeEndpointDef::NodeType(0)
    );
    assert_eq!(
        without_node.edge_types[0].target_node_type,
        EdgeEndpointDef::NodeType(1)
    );

    let without_edge = graph_type
        .without_edge_type(label("types.works_at"))
        .expect("edge type removed");
    assert!(without_edge.edge_types.is_empty());
    assert!(graph_type.without_node_type(label("missing")).is_none());
    assert!(graph_type.without_edge_type(label("missing")).is_none());
}

#[test]
fn rkyv_round_trips_graph_type_def() {
    let graph_type = valid_type();
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&graph_type).unwrap();
    let decoded = rkyv::from_bytes::<GraphTypeDef, rkyv::rancor::Error>(&bytes).unwrap();
    assert_eq!(decoded, graph_type);
}

fn three_node_type_graph(extra_edge: Option<EdgeTypeDef>) -> GraphTypeDef {
    let mut node_types = vec![
        NodeTypeDef {
            name: label("types.person"),
            key_labels: LabelSet::single(label("Person")),
            properties: vec![property("name")],
            validation_mode: ValidationMode::Strict,
        },
        NodeTypeDef {
            name: label("types.company"),
            key_labels: LabelSet::single(label("Company")),
            properties: vec![property("name")],
            validation_mode: ValidationMode::Strict,
        },
        NodeTypeDef {
            name: label("types.school"),
            key_labels: LabelSet::single(label("School")),
            properties: vec![property("name")],
            validation_mode: ValidationMode::Strict,
        },
    ];
    let _ = &mut node_types;
    let mut edge_types = vec![EdgeTypeDef {
        name: label("types.affiliated_with"),
        label: label("AFFILIATED_WITH"),
        source_node_type: EdgeEndpointDef::NodeType(0),
        target_node_type: EdgeEndpointDef::one_of([1, 2]),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
    }];
    if let Some(extra) = extra_edge {
        edge_types.push(extra);
    }
    GraphTypeDef {
        name: label("types.graph"),
        node_types,
        edge_types,
    }
}

#[test]
fn one_of_canonicalizes_singleton_to_node_type() {
    // Per §A.2 F6 fold: constructor collapses a singleton input to
    // NodeType so Eq/Hash/rkyv stay stable across construction paths.
    assert_eq!(EdgeEndpointDef::one_of([5]), EdgeEndpointDef::NodeType(5));
    assert_eq!(
        EdgeEndpointDef::one_of(std::iter::once(7)),
        EdgeEndpointDef::NodeType(7)
    );
}

#[test]
fn one_of_sorts_and_dedupes() {
    let endpoint = EdgeEndpointDef::one_of([3, 1, 3, 2]);
    assert_eq!(endpoint, EdgeEndpointDef::OneOf(vec![1, 2, 3]));
}

#[test]
fn one_of_dedupe_collapses_to_node_type_when_all_equal() {
    // All-equal input collapses to NodeType via dedup + singleton-collapse.
    assert_eq!(
        EdgeEndpointDef::one_of([4, 4, 4]),
        EdgeEndpointDef::NodeType(4)
    );
}

#[test]
fn matches_node_type_oneof_membership() {
    let endpoint = EdgeEndpointDef::OneOf(vec![1, 2, 3]);
    assert!(endpoint.matches_node_type(2));
    assert!(endpoint.matches_node_type(1));
    assert!(endpoint.matches_node_type(3));
    assert!(!endpoint.matches_node_type(0));
    assert!(!endpoint.matches_node_type(4));
}

#[test]
fn overlaps_oneof_intersection_nonempty() {
    let one_two = EdgeEndpointDef::OneOf(vec![1, 2]);
    let two_three = EdgeEndpointDef::OneOf(vec![2, 3]);
    let three_four = EdgeEndpointDef::OneOf(vec![3, 4]);
    assert!(one_two.overlaps(&two_three));
    assert!(!one_two.overlaps(&three_four));
    assert!(one_two.overlaps(&EdgeEndpointDef::NodeType(2)));
    assert!(!one_two.overlaps(&EdgeEndpointDef::NodeType(3)));
    assert!(EdgeEndpointDef::NodeType(2).overlaps(&one_two));
    assert!(one_two.overlaps(&EdgeEndpointDef::Any));
    assert!(EdgeEndpointDef::Any.overlaps(&one_two));
}

#[test]
fn validate_ref_rejects_oneof_index_out_of_range() {
    let mut graph_type = three_node_type_graph(None);
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::OneOf(vec![1, 99]);
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason }) if reason.contains("references node type index")
    ));
}

#[test]
fn validate_ref_rejects_overlapping_oneof_endpoints_same_label() {
    let extra = EdgeTypeDef {
        name: label("types.affiliated_with_alt"),
        label: label("AFFILIATED_WITH"),
        source_node_type: EdgeEndpointDef::NodeType(0),
        target_node_type: EdgeEndpointDef::OneOf(vec![2, 1]),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
    };
    // The first edge already declares target = OneOf([1, 2]); a second edge
    // with the same label and an overlapping target set must be rejected.
    // We bypass the constructor by storing `[2, 1]` unsorted to test BOTH
    // the unsorted/dedup invariant AND the overlap check (the unsorted one
    // is caught first by ensure_endpoint_index).
    let graph_type = three_node_type_graph(Some(extra));
    let result = graph_type.validate();
    assert!(matches!(result, Err(GraphError::Inconsistent { .. })));
    // Use the canonical sorted form for the overlap-only check.
    let extra_sorted = EdgeTypeDef {
        name: label("types.affiliated_with_alt"),
        label: label("AFFILIATED_WITH"),
        source_node_type: EdgeEndpointDef::NodeType(0),
        target_node_type: EdgeEndpointDef::OneOf(vec![1, 2]),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
    };
    let graph_type = three_node_type_graph(Some(extra_sorted));
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("ambiguous edge type endpoints")
    ));
}

#[test]
fn validate_ref_rejects_singleton_oneof_direct_construction() {
    // Defense in depth per §A.2 F6: the `one_of` constructor would have
    // collapsed [5] to NodeType(5), but raw struct construction (e.g. from
    // a corrupted rkyv decode) MUST be rejected by validate_ref.
    let mut graph_type = three_node_type_graph(None);
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::OneOf(vec![1]);
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("OneOf must enumerate at least two")
    ));
}

#[test]
fn validate_ref_rejects_empty_oneof_direct_construction() {
    let mut graph_type = three_node_type_graph(None);
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::OneOf(Vec::new());
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("OneOf must enumerate at least two")
    ));
}

#[test]
fn validate_ref_rejects_unsorted_oneof() {
    let mut graph_type = three_node_type_graph(None);
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::OneOf(vec![2, 1]);
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("not sorted and deduplicated")
    ));
}

#[test]
fn validate_ref_rejects_duplicated_oneof() {
    let mut graph_type = three_node_type_graph(None);
    graph_type.edge_types[0].target_node_type = EdgeEndpointDef::OneOf(vec![1, 1, 2]);
    assert!(matches!(
        graph_type.validate(),
        Err(GraphError::Inconsistent { reason })
            if reason.contains("not sorted and deduplicated")
    ));
}

#[test]
fn node_type_index_returns_none_for_oneof() {
    // §E Q5: OneOf is polymorphic; node_type_index() must return None so
    // callers that assume `Some(_)` means "single concrete type" do not
    // silently misroute.
    let endpoint = EdgeEndpointDef::OneOf(vec![1, 2]);
    assert_eq!(endpoint.node_type_index(), None);
}

#[test]
fn display_oneof_renders_indices() {
    assert_eq!(format!("{}", EdgeEndpointDef::Any), "Any");
    assert_eq!(format!("{}", EdgeEndpointDef::NodeType(4)), "4");
    assert_eq!(
        format!("{}", EdgeEndpointDef::OneOf(vec![1, 2, 3])),
        "OneOf(1, 2, 3)"
    );
}

#[test]
fn rkyv_round_trips_graph_type_def_with_oneof() {
    // Verifies Q3 grounding: appending OneOf as the third variant preserves
    // the prior discriminants (Any=0, NodeType=1) and round-trips the new
    // variant byte-stably.
    let graph_type = three_node_type_graph(None);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&graph_type).unwrap();
    let decoded = rkyv::from_bytes::<GraphTypeDef, rkyv::rancor::Error>(&bytes).unwrap();
    assert_eq!(decoded, graph_type);
    // Pin the OneOf shape explicitly so a future variant reorder breaks
    // this test rather than silently corrupting on-disk data.
    assert_eq!(
        decoded.edge_types[0].target_node_type,
        EdgeEndpointDef::OneOf(vec![1, 2])
    );
}

#[test]
fn list_element_nullability_is_explicit() {
    let int_list = PropertyElementType::Scalar(PropertyValueType::Int);
    assert!(
        int_list.matches(&Value::Int(7)),
        "a typed Int element conforms to LIST<Int>"
    );
    assert!(
        int_list.matches(&Value::Null),
        "a NULL element conforms to nullable LIST<Int>"
    );

    let strict_int_list = PropertyElementType::NotNull(Box::new(PropertyElementType::Scalar(
        PropertyValueType::Int,
    )));
    assert!(strict_int_list.matches(&Value::Int(7)));
    assert!(
        !strict_int_list.matches(&Value::Null),
        "explicit LIST<Int NOT NULL> rejects NULL elements"
    );
}
