use smallvec::SmallVec;

use crate::{
    Change, EdgeId, EdgeTypeDef, EdgeTypeDefV1, GraphId, GraphType, GraphTypeId, IStr,
    IvfIndexConfig, LabelDiff, LabelSet, NodeId, NodeTypeDef, NodeTypeDefV1, NodeTypeRef,
    PropertyDiff, PropertyMap, RecordTypeDef, RecordTypeId, SchemaChange, SchemaPropertyIndexKind,
    SchemaVectorIndexKind,
};

impl Change {
    /// Factory table with one sample change for each [`Change`] variant.
    ///
    /// Tests use this as an append-only ANCHOR so new WAL variants require a
    /// source-of-truth census update in `selene-core`.
    pub const ALL: &[fn() -> Self] = &[
        || Self::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::new(),
            properties: PropertyMap::new(),
        },
        || Self::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([], []).unwrap(),
            properties_diff: PropertyDiff::new([], []).unwrap(),
        },
        || Self::NodeDeleted { id: NodeId::new(1) },
        || Self::EdgeCreated {
            id: EdgeId::new(1),
            label: changeset_variant_istr("change.all.edge"),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
        || Self::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([], []).unwrap(),
        },
        || Self::EdgeDeleted { id: EdgeId::new(1) },
        || Self::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphDropped {
                id: GraphId::new(2),
            },
        },
        || Self::NodePropertyRemoved {
            id: NodeId::new(1),
            property: changeset_variant_istr("change.all.node_property_removed"),
        },
        || Self::EdgePropertyRemoved {
            id: EdgeId::new(1),
            property: changeset_variant_istr("change.all.edge_property_removed"),
        },
        || Self::NodeLabelRemoved {
            id: NodeId::new(1),
            label: changeset_variant_istr("change.all.node_label_removed"),
        },
        || Self::NodesOfTypeTruncated {
            label: changeset_variant_istr("change.all.nodes_of_type_truncated"),
        },
        || Self::EdgesOfTypeTruncated {
            label: changeset_variant_istr("change.all.edges_of_type_truncated"),
        },
        || Self::GraphReset {},
    ];

    /// Number of known [`Change`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this change variant.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::NodeCreated { .. } => "NodeCreated",
            Self::NodeUpdated { .. } => "NodeUpdated",
            Self::NodeDeleted { .. } => "NodeDeleted",
            Self::EdgeCreated { .. } => "EdgeCreated",
            Self::EdgeUpdated { .. } => "EdgeUpdated",
            Self::EdgeDeleted { .. } => "EdgeDeleted",
            Self::SchemaChanged { .. } => "SchemaChanged",
            Self::NodePropertyRemoved { .. } => "NodePropertyRemoved",
            Self::EdgePropertyRemoved { .. } => "EdgePropertyRemoved",
            Self::NodeLabelRemoved { .. } => "NodeLabelRemoved",
            Self::NodesOfTypeTruncated { .. } => "NodesOfTypeTruncated",
            Self::EdgesOfTypeTruncated { .. } => "EdgesOfTypeTruncated",
            Self::GraphReset {} => "GraphReset",
        }
    }
}

impl SchemaChange {
    /// Factory table with one sample change for each [`SchemaChange`] variant.
    ///
    /// Hidden legacy variants are included because their postcard
    /// discriminants are reserved for WAL compatibility.
    pub const ALL: &[fn() -> Self] = &[
        || Self::GraphCreated {
            id: GraphId::new(1),
            name: changeset_variant_istr("schema.all.graph"),
            graph_type: Some(changeset_graph_type_id()),
        },
        || Self::GraphDropped {
            id: GraphId::new(1),
        },
        || Self::GraphTypeCreated {
            graph_type: changeset_graph_type(),
        },
        || Self::GraphTypeDropped {
            id: changeset_graph_type_id(),
        },
        || Self::NodeTypeAdded {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.node"),
            def: NodeTypeDefV1::new(LabelSet::single(changeset_variant_istr("schema.all.node"))),
        },
        || Self::EdgeTypeAdded {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.edge"),
            def: EdgeTypeDefV1::new(
                changeset_variant_istr("schema.all.edge"),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
            ),
        },
        || Self::NodeTypeDropped {
            graph_type: changeset_graph_type_id(),
            name: changeset_variant_istr("schema.all.node"),
        },
        || Self::EdgeTypeDropped {
            graph_type: changeset_graph_type_id(),
            name: changeset_variant_istr("schema.all.edge"),
        },
        || Self::RecordTypeAdded {
            graph_type: changeset_graph_type_id(),
            def: RecordTypeDef {
                id: RecordTypeId::new(1),
                name: changeset_variant_istr("schema.all.record"),
                fields: SmallVec::new(),
            },
        },
        || Self::PropertyIndexCreated {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.property"),
            kind: SchemaPropertyIndexKind::I64,
        },
        || Self::PropertyIndexDropped {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.property"),
        },
        || Self::PropertyIndexCreatedNamed {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.property"),
            kind: SchemaPropertyIndexKind::I64,
            name: Some(changeset_variant_istr("schema.all.index")),
        },
        || Self::NodeTypeAddedV2 {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.node.v2"),
            def: NodeTypeDef::new(LabelSet::single(changeset_variant_istr(
                "schema.all.node.v2",
            ))),
        },
        || Self::EdgeTypeAddedV2 {
            graph_type: changeset_graph_type_id(),
            label: changeset_variant_istr("schema.all.edge.v2"),
            def: EdgeTypeDef::new(
                changeset_variant_istr("schema.all.edge.v2"),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
                NodeTypeRef(changeset_variant_istr("schema.all.node")),
            ),
        },
        || Self::CompositePropertyIndexCreated {
            label: changeset_variant_istr("schema.all.node"),
            properties: SmallVec::from_vec(vec![
                changeset_variant_istr("schema.all.property.a"),
                changeset_variant_istr("schema.all.property.b"),
            ]),
            kinds: SmallVec::from_vec(vec![
                SchemaPropertyIndexKind::I64,
                SchemaPropertyIndexKind::String,
            ]),
            name: Some(changeset_variant_istr("schema.all.composite.index")),
        },
        || Self::CompositePropertyIndexDropped {
            label: changeset_variant_istr("schema.all.node"),
            properties: SmallVec::from_vec(vec![
                changeset_variant_istr("schema.all.property.a"),
                changeset_variant_istr("schema.all.property.b"),
            ]),
        },
        || Self::VectorIndexCreated {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.vector"),
            kind: SchemaVectorIndexKind::IvfCosine,
            dimension: 128,
            name: Some(changeset_variant_istr("schema.all.vector.index")),
            hnsw_config: None,
            ivf_config: Some(IvfIndexConfig::new(256)),
        },
        || Self::VectorIndexDropped {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.vector"),
        },
        || Self::TextIndexCreated {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.text"),
            name: Some(changeset_variant_istr("schema.all.text.index")),
        },
        || Self::TextIndexDropped {
            label: changeset_variant_istr("schema.all.node"),
            property: changeset_variant_istr("schema.all.text"),
        },
    ];

    /// Number of known [`SchemaChange`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this schema-change variant.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::GraphCreated { .. } => "GraphCreated",
            Self::GraphDropped { .. } => "GraphDropped",
            Self::GraphTypeCreated { .. } => "GraphTypeCreated",
            Self::GraphTypeDropped { .. } => "GraphTypeDropped",
            Self::NodeTypeAdded { .. } => "NodeTypeAdded",
            Self::EdgeTypeAdded { .. } => "EdgeTypeAdded",
            Self::NodeTypeDropped { .. } => "NodeTypeDropped",
            Self::EdgeTypeDropped { .. } => "EdgeTypeDropped",
            Self::RecordTypeAdded { .. } => "RecordTypeAdded",
            Self::PropertyIndexCreated { .. } => "PropertyIndexCreated",
            Self::PropertyIndexDropped { .. } => "PropertyIndexDropped",
            Self::PropertyIndexCreatedNamed { .. } => "PropertyIndexCreatedNamed",
            Self::NodeTypeAddedV2 { .. } => "NodeTypeAddedV2",
            Self::EdgeTypeAddedV2 { .. } => "EdgeTypeAddedV2",
            Self::CompositePropertyIndexCreated { .. } => "CompositePropertyIndexCreated",
            Self::CompositePropertyIndexDropped { .. } => "CompositePropertyIndexDropped",
            Self::VectorIndexCreated { .. } => "VectorIndexCreated",
            Self::VectorIndexDropped { .. } => "VectorIndexDropped",
            Self::TextIndexCreated { .. } => "TextIndexCreated",
            Self::TextIndexDropped { .. } => "TextIndexDropped",
        }
    }
}

fn changeset_variant_istr(name: &str) -> IStr {
    crate::intern(name).expect("Change::ALL fixture strings fit the process interner cap")
}

fn changeset_graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).expect("Change::ALL graph type fixture is non-zero")
}

fn changeset_graph_type() -> GraphType {
    GraphType::new(
        changeset_graph_type_id(),
        changeset_variant_istr("schema.all.graph_type"),
    )
}
