use selene_core::{IStr, LabelSet};

use super::{decode_rkyv, encode_rkyv, ensure_section_within_cap, validate_sorted_unique};
use crate::graph::SeleneGraph;
use crate::graph_types::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyDefaultValue, PropertyTypeDef,
    ValidationMode,
};

const GTYP_V2_MAGIC: u8 = 0xB6;
const GTYP_V3_MAGIC: u8 = 0xB7;

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct GraphTypeDefV1 {
    name: IStr,
    node_types: Vec<NodeTypeDefV1>,
    edge_types: Vec<EdgeTypeDefV1>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct NodeTypeDefV1 {
    name: IStr,
    key_labels: LabelSet,
    properties: Vec<PropertyTypeDefV1>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct EdgeTypeDefV1 {
    name: IStr,
    label: IStr,
    source_node_type: u32,
    target_node_type: u32,
    properties: Vec<PropertyTypeDefV1>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct PropertyTypeDefV1 {
    name: IStr,
    value_type: selene_core::PropertyValueType,
    required: bool,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct GraphTypeDefV2 {
    name: IStr,
    node_types: Vec<NodeTypeDefV2>,
    edge_types: Vec<EdgeTypeDefV2>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct NodeTypeDefV2 {
    name: IStr,
    key_labels: LabelSet,
    properties: Vec<PropertyTypeDefV2>,
    validation_mode: ValidationMode,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct EdgeTypeDefV2 {
    name: IStr,
    label: IStr,
    source_node_type: u32,
    target_node_type: u32,
    properties: Vec<PropertyTypeDefV2>,
    validation_mode: ValidationMode,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
struct PropertyTypeDefV2 {
    name: IStr,
    value_type: selene_core::PropertyValueType,
    required: bool,
    default: Option<PropertyDefaultValue>,
    immutable: bool,
}

impl GraphTypeDefV1 {
    fn into_runtime(self) -> GraphTypeDef {
        GraphTypeDef {
            name: self.name,
            node_types: self
                .node_types
                .into_iter()
                .map(NodeTypeDefV1::into_runtime)
                .collect(),
            edge_types: self
                .edge_types
                .into_iter()
                .map(EdgeTypeDefV1::into_runtime)
                .collect(),
        }
    }
}

impl NodeTypeDefV1 {
    fn into_runtime(self) -> NodeTypeDef {
        NodeTypeDef {
            name: self.name,
            key_labels: self.key_labels,
            properties: self
                .properties
                .into_iter()
                .map(PropertyTypeDefV1::into_runtime)
                .collect(),
            validation_mode: ValidationMode::Strict,
        }
    }
}

impl EdgeTypeDefV1 {
    fn into_runtime(self) -> EdgeTypeDef {
        EdgeTypeDef {
            name: self.name,
            label: self.label,
            source_node_type: EdgeEndpointDef::NodeType(self.source_node_type),
            target_node_type: EdgeEndpointDef::NodeType(self.target_node_type),
            properties: self
                .properties
                .into_iter()
                .map(PropertyTypeDefV1::into_runtime)
                .collect(),
            validation_mode: ValidationMode::Strict,
        }
    }
}

impl PropertyTypeDefV1 {
    fn into_runtime(self) -> PropertyTypeDef {
        PropertyTypeDef {
            name: self.name,
            value_type: self.value_type,
            list_element_type: None,
            required: self.required,
            default: None,
            immutable: false,
        }
    }
}

impl GraphTypeDefV2 {
    fn into_runtime(self) -> GraphTypeDef {
        GraphTypeDef {
            name: self.name,
            node_types: self
                .node_types
                .into_iter()
                .map(NodeTypeDefV2::into_runtime)
                .collect(),
            edge_types: self
                .edge_types
                .into_iter()
                .map(EdgeTypeDefV2::into_runtime)
                .collect(),
        }
    }
}

impl NodeTypeDefV2 {
    fn into_runtime(self) -> NodeTypeDef {
        NodeTypeDef {
            name: self.name,
            key_labels: self.key_labels,
            properties: self
                .properties
                .into_iter()
                .map(PropertyTypeDefV2::into_runtime)
                .collect(),
            validation_mode: self.validation_mode,
        }
    }
}

impl EdgeTypeDefV2 {
    fn into_runtime(self) -> EdgeTypeDef {
        EdgeTypeDef {
            name: self.name,
            label: self.label,
            source_node_type: EdgeEndpointDef::NodeType(self.source_node_type),
            target_node_type: EdgeEndpointDef::NodeType(self.target_node_type),
            properties: self
                .properties
                .into_iter()
                .map(PropertyTypeDefV2::into_runtime)
                .collect(),
            validation_mode: self.validation_mode,
        }
    }
}

impl PropertyTypeDefV2 {
    fn into_runtime(self) -> PropertyTypeDef {
        PropertyTypeDef {
            name: self.name,
            value_type: self.value_type,
            list_element_type: None,
            required: self.required,
            default: self.default,
            immutable: self.immutable,
        }
    }
}

pub(in crate::core_provider) fn encode_graph_types(
    graph: &SeleneGraph,
) -> Result<Vec<u8>, crate::ProviderError> {
    let rows = graph
        .meta
        .bound_type
        .as_ref()
        .map(|type_def| vec![(0_u32, (**type_def).clone())])
        .unwrap_or_default();
    let mut payload = Vec::with_capacity(1);
    payload.push(GTYP_V3_MAGIC);
    payload.extend(encode_rkyv(&rows, "CORE/GTYP")?);
    ensure_section_within_cap("CORE/GTYP", payload.len())?;
    Ok(payload)
}

pub(in crate::core_provider) fn decode_graph_types(
    bytes: &[u8],
) -> Result<Vec<(u32, GraphTypeDef)>, crate::ProviderError> {
    let rows = if bytes.first() == Some(&GTYP_V3_MAGIC) {
        decode_graph_types_v3(&bytes[1..])?
    } else if bytes.first() == Some(&GTYP_V2_MAGIC) {
        decode_graph_types_v2(&bytes[1..]).or_else(|_| decode_graph_types_v1(bytes))?
    } else {
        decode_graph_types_v1(bytes)?
    };
    validate_sorted_unique(&rows, "CORE/GTYP")?;
    Ok(rows)
}

fn decode_graph_types_v1(bytes: &[u8]) -> Result<Vec<(u32, GraphTypeDef)>, crate::ProviderError> {
    let rows: Vec<(u32, GraphTypeDefV1)> = decode_rkyv(bytes, "CORE/GTYP")?;
    Ok(rows
        .into_iter()
        .map(|(index, graph_type)| (index, graph_type.into_runtime()))
        .collect())
}

fn decode_graph_types_v2(bytes: &[u8]) -> Result<Vec<(u32, GraphTypeDef)>, crate::ProviderError> {
    let rows: Vec<(u32, GraphTypeDefV2)> = decode_rkyv(bytes, "CORE/GTYP")?;
    Ok(rows
        .into_iter()
        .map(|(index, graph_type)| (index, graph_type.into_runtime()))
        .collect())
}

fn decode_graph_types_v3(bytes: &[u8]) -> Result<Vec<(u32, GraphTypeDef)>, crate::ProviderError> {
    decode_rkyv(bytes, "CORE/GTYP")
}

#[cfg(test)]
mod tests {
    use selene_core::{GraphId, PropertyValueType, intern};

    use super::*;
    use crate::SharedGraph;

    #[test]
    fn legacy_gtyp_rows_decode_with_default_v2_fields() {
        let person = intern("LegacyPerson").unwrap();
        let name = intern("name").unwrap();
        let tags = intern("tags").unwrap();
        let rows = vec![(
            0_u32,
            GraphTypeDefV1 {
                name: intern("legacy.graph").unwrap(),
                node_types: vec![NodeTypeDefV1 {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: vec![
                        PropertyTypeDefV1 {
                            name,
                            value_type: PropertyValueType::String,
                            required: true,
                        },
                        PropertyTypeDefV1 {
                            name: tags,
                            value_type: PropertyValueType::List,
                            required: false,
                        },
                    ],
                }],
                edge_types: Vec::new(),
            },
        )];
        let bytes = encode_rkyv(&rows, "CORE/GTYP").unwrap();

        let decoded = decode_graph_types(&bytes).unwrap();

        assert_eq!(decoded.len(), 1);
        decoded[0].1.validate_ref().unwrap();
        let node_type = &decoded[0].1.node_types[0];
        assert_eq!(node_type.validation_mode, ValidationMode::Strict);
        assert_eq!(node_type.properties[0].default, None);
        assert!(!node_type.properties[0].immutable);
        assert_eq!(node_type.properties[1].value_type, PropertyValueType::List);
        assert_eq!(node_type.properties[1].list_element_type, None);
    }

    #[test]
    fn gtyp_v2_rows_decode_with_legacy_untyped_list() {
        let person = intern("V2LegacyPerson").unwrap();
        let tags = intern("v2.tags").unwrap();
        let rows = vec![(
            0_u32,
            GraphTypeDefV2 {
                name: intern("legacy.v2.graph").unwrap(),
                node_types: vec![NodeTypeDefV2 {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: vec![PropertyTypeDefV2 {
                        name: tags,
                        value_type: PropertyValueType::List,
                        required: false,
                        default: None,
                        immutable: true,
                    }],
                    validation_mode: ValidationMode::Warn,
                }],
                edge_types: Vec::new(),
            },
        )];
        let mut bytes = vec![GTYP_V2_MAGIC];
        bytes.extend(encode_rkyv(&rows, "CORE/GTYP").unwrap());

        let decoded = decode_graph_types(&bytes).unwrap();

        assert_eq!(decoded.len(), 1);
        decoded[0].1.validate_ref().unwrap();
        let property = &decoded[0].1.node_types[0].properties[0];
        assert_eq!(
            decoded[0].1.node_types[0].validation_mode,
            ValidationMode::Warn
        );
        assert_eq!(property.value_type, PropertyValueType::List);
        assert_eq!(property.list_element_type, None);
        assert!(property.immutable);
    }

    #[test]
    fn gtyp_v2_rows_decode_legacy_edge_endpoints_as_node_type_endpoints() {
        let person = intern("V2EndpointPerson").unwrap();
        let knows = intern("V2_ENDPOINT_KNOWS").unwrap();
        let rows = vec![(
            0_u32,
            GraphTypeDefV2 {
                name: intern("legacy.v2.endpoint.graph").unwrap(),
                node_types: vec![NodeTypeDefV2 {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                }],
                edge_types: vec![EdgeTypeDefV2 {
                    name: knows,
                    label: knows,
                    source_node_type: 0,
                    target_node_type: 0,
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                }],
            },
        )];
        let mut bytes = vec![GTYP_V2_MAGIC];
        bytes.extend(encode_rkyv(&rows, "CORE/GTYP").unwrap());

        let decoded = decode_graph_types(&bytes).unwrap();

        let edge_type = &decoded[0].1.edge_types[0];
        assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
        assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(0));
    }

    #[test]
    fn encode_graph_types_writes_gtyp_v3_magic() {
        let person = intern("V3Person").unwrap();
        let graph_type = GraphTypeDef {
            name: intern("v3.graph").unwrap(),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
            edge_types: Vec::new(),
        };
        let graph = SharedGraph::builder(GraphId::new(211))
            .bound_to(graph_type)
            .unwrap()
            .build()
            .unwrap()
            .read()
            .as_ref()
            .clone();

        let bytes = encode_graph_types(&graph).unwrap();

        assert_eq!(bytes.first(), Some(&GTYP_V3_MAGIC));
    }

    #[test]
    fn gtyp_v3_round_trips_oneof_endpoint() {
        // BRIEF-131e: GTYP V3 magic stays at 0xB7; the appended OneOf variant
        // rides on the existing in-place V3 evolution. Encode + decode a
        // GraphTypeDef with an OneOf edge endpoint and assert structural
        // equality including OneOf payload sort order.
        let person = intern("V3OneOfPerson").unwrap();
        let company = intern("V3OneOfCompany").unwrap();
        let school = intern("V3OneOfSchool").unwrap();
        let affiliated = intern("V3_AFFILIATED").unwrap();
        let graph_type = GraphTypeDef {
            name: intern("v3.oneof.graph").unwrap(),
            node_types: vec![
                NodeTypeDef {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                },
                NodeTypeDef {
                    name: company,
                    key_labels: LabelSet::single(company),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                },
                NodeTypeDef {
                    name: school,
                    key_labels: LabelSet::single(school),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                },
            ],
            edge_types: vec![EdgeTypeDef {
                name: affiliated,
                label: affiliated,
                source_node_type: EdgeEndpointDef::NodeType(0),
                target_node_type: EdgeEndpointDef::one_of([1, 2]),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
        };
        let graph = SharedGraph::builder(GraphId::new(212))
            .bound_to(graph_type.clone())
            .unwrap()
            .build()
            .unwrap()
            .read()
            .as_ref()
            .clone();

        let bytes = encode_graph_types(&graph).unwrap();
        assert_eq!(bytes.first(), Some(&GTYP_V3_MAGIC));

        let decoded = decode_graph_types(&bytes).unwrap();
        assert_eq!(decoded.len(), 1);
        let decoded_graph_type = &decoded[0].1;
        decoded_graph_type.validate_ref().unwrap();
        let edge_type = &decoded_graph_type.edge_types[0];
        // Pin the OneOf shape explicitly so a future variant-reorder regression
        // surfaces here rather than as silent on-disk corruption.
        assert_eq!(
            edge_type.target_node_type,
            EdgeEndpointDef::OneOf(vec![1, 2])
        );
        assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
    }

    #[test]
    fn gtyp_v1_legacy_still_decodes_unchanged() {
        // Q7 grounding: legacy V1 decode produces NodeType only; OneOf is
        // structurally unreachable. This is the regression guard.
        let person = intern("V1LegacyOneOfBlind").unwrap();
        let knows = intern("V1_LEGACY_KNOWS").unwrap();
        let rows = vec![(
            0_u32,
            GraphTypeDefV1 {
                name: intern("v1.oneof.blind.graph").unwrap(),
                node_types: vec![NodeTypeDefV1 {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: Vec::new(),
                }],
                edge_types: vec![EdgeTypeDefV1 {
                    name: knows,
                    label: knows,
                    source_node_type: 0,
                    target_node_type: 0,
                    properties: Vec::new(),
                }],
            },
        )];
        let bytes = encode_rkyv(&rows, "CORE/GTYP").unwrap();
        let decoded = decode_graph_types(&bytes).unwrap();
        let edge_type = &decoded[0].1.edge_types[0];
        assert!(
            !matches!(edge_type.source_node_type, EdgeEndpointDef::OneOf(_)),
            "V1 legacy must never produce OneOf"
        );
        assert!(
            !matches!(edge_type.target_node_type, EdgeEndpointDef::OneOf(_)),
            "V1 legacy must never produce OneOf"
        );
        assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
        assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(0));
    }

    #[test]
    fn gtyp_v2_legacy_still_decodes_unchanged() {
        // Q7 grounding: legacy V2 decode produces NodeType only.
        let person = intern("V2LegacyOneOfBlind").unwrap();
        let knows = intern("V2_LEGACY_KNOWS").unwrap();
        let rows = vec![(
            0_u32,
            GraphTypeDefV2 {
                name: intern("v2.oneof.blind.graph").unwrap(),
                node_types: vec![NodeTypeDefV2 {
                    name: person,
                    key_labels: LabelSet::single(person),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                }],
                edge_types: vec![EdgeTypeDefV2 {
                    name: knows,
                    label: knows,
                    source_node_type: 0,
                    target_node_type: 0,
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                }],
            },
        )];
        let mut bytes = vec![GTYP_V2_MAGIC];
        bytes.extend(encode_rkyv(&rows, "CORE/GTYP").unwrap());
        let decoded = decode_graph_types(&bytes).unwrap();
        let edge_type = &decoded[0].1.edge_types[0];
        assert!(
            !matches!(edge_type.source_node_type, EdgeEndpointDef::OneOf(_)),
            "V2 legacy must never produce OneOf"
        );
        assert!(
            !matches!(edge_type.target_node_type, EdgeEndpointDef::OneOf(_)),
            "V2 legacy must never produce OneOf"
        );
        assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
        assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(0));
    }
}
