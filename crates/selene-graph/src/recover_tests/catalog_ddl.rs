use super::*;

#[test]
fn recover_closed_wal_only_replays_catalog_ddl() {
    let dir = temp_dir("closed-schema-wal-only");
    let graph_id = GraphId::new(19);
    let base = empty_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let sensor = db_string("Sensor").unwrap();
    let serial = db_string("serial").unwrap();
    let payload = db_string("payload").unwrap();
    let device_id = db_string("device_id").unwrap();
    let score = db_string("score").unwrap();
    let small_score = db_string("small_score").unwrap();
    let exact_numeric_defaults = [
        (
            db_string("unsigned_count").unwrap(),
            selene_core::PropertyValueType::Uint,
            PropertyDefaultValue::Uint(42),
        ),
        (
            db_string("wide_signed").unwrap(),
            selene_core::PropertyValueType::Int128,
            PropertyDefaultValue::Int128(i128::MIN),
        ),
        (
            db_string("wide_unsigned").unwrap(),
            selene_core::PropertyValueType::Uint128,
            PropertyDefaultValue::Uint128(u128::MAX),
        ),
        (
            db_string("decimal_score").unwrap(),
            selene_core::PropertyValueType::Decimal,
            PropertyDefaultValue::Decimal(db_string("123.450").unwrap()),
        ),
    ];
    let temporal_defaults = [
        (
            db_string("event_date").unwrap(),
            selene_core::PropertyValueType::Date,
            PropertyDefaultValue::Date(db_string("2026-05-07").unwrap()),
        ),
        (
            db_string("event_local_dt").unwrap(),
            selene_core::PropertyValueType::LocalDateTime,
            PropertyDefaultValue::LocalDateTime(db_string("2026-05-07T12:34:56").unwrap()),
        ),
        (
            db_string("event_zoned_dt").unwrap(),
            selene_core::PropertyValueType::ZonedDateTime,
            PropertyDefaultValue::ZonedDateTime(db_string("2026-05-07T12:34:56-04").unwrap()),
        ),
        (
            db_string("event_local_time").unwrap(),
            selene_core::PropertyValueType::LocalTime,
            PropertyDefaultValue::LocalTime(db_string("12:34:56").unwrap()),
        ),
        (
            db_string("event_zoned_time").unwrap(),
            selene_core::PropertyValueType::ZonedTime,
            PropertyDefaultValue::ZonedTime(db_string("12:34:56-04").unwrap()),
        ),
        (
            db_string("event_duration").unwrap(),
            selene_core::PropertyValueType::Duration,
            PropertyDefaultValue::Duration(db_string("PT1H2S").unwrap()),
        ),
    ];
    let vector_defaults = [(
        db_string("embedding").unwrap(),
        selene_core::PropertyValueType::Vector,
        PropertyDefaultValue::Vector(vec![
            1.0_f32.to_bits(),
            0.0_f32.to_bits(),
            2.5_f32.to_bits(),
        ]),
    )];
    let list_defaults = [(
        db_string("tags").unwrap(),
        PropertyElementType::Scalar(selene_core::PropertyValueType::String),
        PropertyDefaultValue::List(vec![
            Box::new(PropertyDefaultValue::String(db_string("alpha").unwrap())),
            Box::new(PropertyDefaultValue::String(db_string("beta").unwrap())),
        ]),
    )];
    let changes = {
        let mut txn = shared.begin_write();
        let mut properties = base_properties(&serial, &payload, &device_id, &score, &small_score);
        properties.extend(
            exact_numeric_defaults
                .iter()
                .chain(temporal_defaults.iter())
                .chain(vector_defaults.iter())
                .map(|(name, value_type, default)| PropertyTypeDef {
                    name: name.clone(),
                    value_type: *value_type,
                    list_element_type: None,
                    required: false,
                    default: Some(default.clone()),
                    immutable: false,
                    record_field_types: None,
                }),
        );
        properties.extend(list_defaults.iter().map(|(name, element_type, default)| {
            PropertyTypeDef {
                name: name.clone(),
                value_type: selene_core::PropertyValueType::List,
                list_element_type: Some(element_type.clone()),
                required: false,
                default: Some(default.clone()),
                immutable: false,
                record_field_types: None,
            }
        }));
        txn.mutator()
            .create_node_type(
                sensor.clone(),
                LabelSet::single(sensor.clone()),
                properties,
                ValidationMode::Warn,
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    assert!(matches!(
        changes.as_slice(),
        [Change::SchemaChanged {
            change: selene_core::SchemaChange::NodeTypeAddedV2 { .. },
            ..
        }]
    ));
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(graph_type.node_types.len(), 1);
    assert_eq!(graph_type.node_types[0].name, sensor);
    assert_eq!(
        graph_type.node_types[0].key_labels,
        LabelSet::single(sensor)
    );
    assert_eq!(
        graph_type.node_types[0].validation_mode,
        ValidationMode::Warn
    );
    assert_base_properties(
        &graph_type.node_types[0].properties,
        &serial,
        &payload,
        &device_id,
        &score,
        &small_score,
    );
    for (offset, (name, _value_type, default)) in exact_numeric_defaults.iter().enumerate() {
        let property = &graph_type.node_types[0].properties[5 + offset];
        assert_eq!(&property.name, name);
        assert_eq!(property.default.as_ref(), Some(default));
    }
    let temporal_start = 5 + exact_numeric_defaults.len();
    for (offset, (name, _value_type, default)) in temporal_defaults.iter().enumerate() {
        let property = &graph_type.node_types[0].properties[temporal_start + offset];
        assert_eq!(&property.name, name);
        assert_eq!(property.default.as_ref(), Some(default));
    }
    let vector_start = temporal_start + temporal_defaults.len();
    for (offset, (name, _value_type, default)) in vector_defaults.iter().enumerate() {
        let property = &graph_type.node_types[0].properties[vector_start + offset];
        assert_eq!(&property.name, name);
        assert_eq!(property.default.as_ref(), Some(default));
    }
    let list_start = vector_start + vector_defaults.len();
    for (offset, (name, element_type, default)) in list_defaults.iter().enumerate() {
        let property = &graph_type.node_types[0].properties[list_start + offset];
        assert_eq!(&property.name, name);
        assert_eq!(property.list_element_type.as_ref(), Some(element_type));
        assert_eq!(property.default.as_ref(), Some(default));
    }
    let _ = fs::remove_dir_all(dir);
}

fn base_properties(
    serial: &selene_core::DbString,
    payload: &selene_core::DbString,
    device_id: &selene_core::DbString,
    score: &selene_core::DbString,
    small_score: &selene_core::DbString,
) -> Vec<PropertyTypeDef> {
    vec![
        PropertyTypeDef {
            name: serial.clone(),
            value_type: selene_core::PropertyValueType::String,
            list_element_type: None,
            required: false,
            default: Some(PropertyDefaultValue::String(db_string("unknown").unwrap())),
            immutable: true,
            record_field_types: None,
        },
        PropertyTypeDef {
            name: payload.clone(),
            value_type: selene_core::PropertyValueType::Bytes,
            list_element_type: None,
            required: false,
            default: Some(PropertyDefaultValue::Bytes(vec![0xCA, 0xFE])),
            immutable: false,
            record_field_types: None,
        },
        PropertyTypeDef {
            name: device_id.clone(),
            value_type: selene_core::PropertyValueType::Uuid,
            list_element_type: None,
            required: false,
            default: Some(PropertyDefaultValue::Uuid(
                db_string("018f1b6d-7b89-7cc0-9f40-2c6f8d4df101").unwrap(),
            )),
            immutable: false,
            record_field_types: None,
        },
        PropertyTypeDef {
            name: score.clone(),
            value_type: selene_core::PropertyValueType::Float,
            list_element_type: None,
            required: false,
            default: Some(PropertyDefaultValue::Float(1.5_f64.to_bits())),
            immutable: false,
            record_field_types: None,
        },
        PropertyTypeDef {
            name: small_score.clone(),
            value_type: selene_core::PropertyValueType::Float32,
            list_element_type: None,
            required: false,
            default: Some(PropertyDefaultValue::Float32(2.25_f32.to_bits())),
            immutable: false,
            record_field_types: None,
        },
    ]
}

fn assert_base_properties(
    properties: &[PropertyTypeDef],
    serial: &selene_core::DbString,
    payload: &selene_core::DbString,
    device_id: &selene_core::DbString,
    score: &selene_core::DbString,
    small_score: &selene_core::DbString,
) {
    assert_eq!(properties[0].name, *serial);
    assert_eq!(
        properties[0].default,
        Some(PropertyDefaultValue::String(db_string("unknown").unwrap()))
    );
    assert!(properties[0].immutable);
    assert_eq!(properties[1].name, *payload);
    assert_eq!(
        properties[1].default,
        Some(PropertyDefaultValue::Bytes(vec![0xCA, 0xFE]))
    );
    assert_eq!(properties[2].name, *device_id);
    assert_eq!(
        properties[2].default,
        Some(PropertyDefaultValue::Uuid(
            db_string("018f1b6d-7b89-7cc0-9f40-2c6f8d4df101").unwrap()
        ))
    );
    assert_eq!(properties[3].name, *score);
    assert_eq!(
        properties[3].default,
        Some(PropertyDefaultValue::Float(1.5_f64.to_bits()))
    );
    assert_eq!(properties[4].name, *small_score);
    assert_eq!(
        properties[4].default,
        Some(PropertyDefaultValue::Float32(2.25_f32.to_bits()))
    );
}
