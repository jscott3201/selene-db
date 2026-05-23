//! Schema model types per spec 02 section 6.
//!
//! These are structural data carriers. Runtime validation of graph mutations
//! against a [`GraphType`] belongs to `selene-graph`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smallvec::SmallVec;

use crate::{CoreError, CoreResult, ExtensionTypeId, IStr, LabelSet, RecordTypeId, Value};

/// Graph-type-scoped schema identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(transparent)]
pub struct GraphTypeId(pub u64);

impl<'de> Deserialize<'de> for GraphTypeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = u64::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

impl GraphTypeId {
    /// Construct a graph type ID, rejecting the reserved zero sentinel.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ZeroIdentifier`] when `value` is `0`.
    pub const fn new(value: u64) -> CoreResult<Self> {
        if value == 0 {
            Err(CoreError::ZeroIdentifier)
        } else {
            Ok(Self(value))
        }
    }

    /// Return the raw `u64` value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GraphTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GraphTypeId({})", self.0)
    }
}

/// Closed-graph schema definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphType {
    /// Stable graph type ID.
    pub id: GraphTypeId,
    /// Interned graph type name.
    pub name: IStr,
    /// Node types keyed by node label.
    pub node_types: BTreeMap<IStr, NodeTypeDef>,
    /// Edge types keyed by edge label.
    pub edge_types: BTreeMap<IStr, EdgeTypeDef>,
    /// Record types keyed by record type ID.
    pub record_types: BTreeMap<RecordTypeId, RecordTypeDef>,
    /// Policy for overlap between key label sets.
    pub key_label_set_policy: KeyLabelSetPolicy,
}

impl GraphType {
    /// Construct an empty graph type.
    #[must_use]
    pub fn new(id: GraphTypeId, name: IStr) -> Self {
        Self {
            id,
            name,
            node_types: BTreeMap::new(),
            edge_types: BTreeMap::new(),
            record_types: BTreeMap::new(),
            key_label_set_policy: KeyLabelSetPolicy::default(),
        }
    }
}

/// Node type definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NodeTypeDef {
    /// Label set required by this node type.
    pub labels: LabelSet,
    /// Property definitions in schema order.
    pub properties: SmallVec<[PropertyDef; 8]>,
    /// Optional property-name key.
    pub key: Option<NodeKey>,
    /// Closed-graph validation mode for this node type.
    #[serde(default)]
    pub validation_mode: ValidationMode,
}

impl NodeTypeDef {
    /// Construct a node type definition with no properties.
    #[must_use]
    pub fn new(labels: LabelSet) -> Self {
        Self {
            labels,
            properties: SmallVec::new(),
            key: None,
            validation_mode: ValidationMode::Strict,
        }
    }
}

/// Legacy WAL node type definition carried by [`SchemaChange::NodeTypeAdded`](crate::SchemaChange::NodeTypeAdded).
///
/// This freezes the pre-v1.1 catalog-DDL payload shape. New WAL entries use
/// [`SchemaChange::NodeTypeAddedV2`](crate::SchemaChange::NodeTypeAddedV2)
/// with [`NodeTypeDef`]; recovery upgrades this shape with
/// [`ValidationMode::Strict`] and non-immutable properties.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NodeTypeDefV1 {
    /// Label set required by this node type.
    pub labels: LabelSet,
    /// Property definitions in schema order.
    pub properties: SmallVec<[PropertyDefV1; 8]>,
    /// Optional property-name key.
    pub key: Option<NodeKey>,
}

impl NodeTypeDefV1 {
    /// Construct a legacy node type definition with no properties.
    #[must_use]
    pub fn new(labels: LabelSet) -> Self {
        Self {
            labels,
            properties: SmallVec::new(),
            key: None,
        }
    }
}

impl From<NodeTypeDefV1> for NodeTypeDef {
    fn from(value: NodeTypeDefV1) -> Self {
        Self {
            labels: value.labels,
            properties: value.properties.into_iter().map(Into::into).collect(),
            key: value.key,
            validation_mode: ValidationMode::Strict,
        }
    }
}

/// Property-name list that forms a node key.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct NodeKey {
    /// Property names participating in the key.
    pub property_names: SmallVec<[IStr; 2]>,
}

/// Edge endpoint definition.
///
/// `OneOf` carries a sorted, deduplicated, length-≥-2 set of distinct
/// [`NodeTypeRef`]s. Construct it via [`EdgeEndpointDef::one_of`] so the
/// invariants are enforced (singleton inputs collapse to
/// [`EdgeEndpointDef::NodeType`]). The WAL is permissive — recovery re-applies
/// the constructor through the storage-side resolver, so direct struct
/// construction in WAL paths is acceptable and replay canonicalizes.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum EdgeEndpointDef {
    /// Accept any declared node type at this endpoint.
    Any,
    /// Reference one concrete node type.
    NodeType(NodeTypeRef),
    /// Reference any node type drawn from a sorted, deduplicated, length-≥-2
    /// set of distinct node types.
    OneOf(SmallVec<[NodeTypeRef; 4]>),
}

impl EdgeEndpointDef {
    /// Construct an endpoint accepting `refs`, canonicalized.
    ///
    /// References are sorted by interned-name identity and deduplicated. A
    /// single resulting reference collapses to [`EdgeEndpointDef::NodeType`].
    ///
    /// # Panics
    ///
    /// Panics when the resulting set is empty; zero-label endpoints are a
    /// caller bug and the upstream resolver must reject them before reaching
    /// this constructor.
    #[must_use]
    pub fn one_of(refs: impl IntoIterator<Item = NodeTypeRef>) -> Self {
        let mut buf: SmallVec<[NodeTypeRef; 4]> = refs.into_iter().collect();
        buf.sort_unstable_by_key(|node| node.0);
        buf.dedup();
        assert!(
            !buf.is_empty(),
            "EdgeEndpointDef::one_of called with empty NodeTypeRef set"
        );
        match buf.len() {
            1 => Self::NodeType(buf[0]),
            _ => Self::OneOf(buf),
        }
    }
}

/// Edge type definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EdgeTypeDef {
    /// Single edge label.
    pub label: IStr,
    /// Source endpoint definition.
    pub source_node_type: EdgeEndpointDef,
    /// Target endpoint definition.
    pub target_node_type: EdgeEndpointDef,
    /// Property definitions in schema order.
    pub properties: SmallVec<[PropertyDef; 4]>,
    /// Closed-graph validation mode for this edge type.
    #[serde(default)]
    pub validation_mode: ValidationMode,
}

impl EdgeTypeDef {
    /// Construct an edge type definition with no properties.
    #[must_use]
    pub fn new(label: IStr, source: NodeTypeRef, target: NodeTypeRef) -> Self {
        Self::new_with_endpoints(
            label,
            EdgeEndpointDef::NodeType(source),
            EdgeEndpointDef::NodeType(target),
        )
    }

    /// Construct an edge type definition with explicit endpoints and no properties.
    #[must_use]
    pub fn new_with_endpoints(
        label: IStr,
        source: EdgeEndpointDef,
        target: EdgeEndpointDef,
    ) -> Self {
        Self {
            label,
            source_node_type: source,
            target_node_type: target,
            properties: SmallVec::new(),
            validation_mode: ValidationMode::Strict,
        }
    }
}

/// Legacy WAL edge type definition carried by [`SchemaChange::EdgeTypeAdded`](crate::SchemaChange::EdgeTypeAdded).
///
/// This freezes the pre-v1.1 catalog-DDL payload shape. New WAL entries use
/// [`SchemaChange::EdgeTypeAddedV2`](crate::SchemaChange::EdgeTypeAddedV2)
/// with [`EdgeTypeDef`]; recovery upgrades this shape with
/// [`ValidationMode::Strict`] and non-immutable properties.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EdgeTypeDefV1 {
    /// Single edge label.
    pub label: IStr,
    /// Source node type reference.
    pub source_node_type: NodeTypeRef,
    /// Target node type reference.
    pub target_node_type: NodeTypeRef,
    /// Property definitions in schema order.
    pub properties: SmallVec<[PropertyDefV1; 4]>,
}

impl EdgeTypeDefV1 {
    /// Construct a legacy edge type definition with no properties.
    #[must_use]
    pub fn new(label: IStr, source: NodeTypeRef, target: NodeTypeRef) -> Self {
        Self {
            label,
            source_node_type: source,
            target_node_type: target,
            properties: SmallVec::new(),
        }
    }
}

impl From<EdgeTypeDefV1> for EdgeTypeDef {
    fn from(value: EdgeTypeDefV1) -> Self {
        Self {
            label: value.label,
            source_node_type: EdgeEndpointDef::NodeType(value.source_node_type),
            target_node_type: EdgeEndpointDef::NodeType(value.target_node_type),
            properties: value.properties.into_iter().map(Into::into).collect(),
            validation_mode: ValidationMode::Strict,
        }
    }
}

/// Closed-graph validation mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ValidationMode {
    /// Reject type-model violations.
    #[default]
    Strict,
    /// Allow relaxed property-shape writes and report warnings.
    Warn,
}

/// Node type reference by label.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[repr(transparent)]
pub struct NodeTypeRef(pub IStr);

/// Record type reference by ID.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[repr(transparent)]
pub struct RecordTypeRef(pub RecordTypeId);

/// Property schema definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PropertyDef {
    /// Property name.
    pub name: IStr,
    /// Property value type.
    pub value_type: ValueType,
    /// Whether `Value::Null` is allowed.
    pub nullable: bool,
    /// Optional default value.
    pub default: Option<Value>,
    /// Whether updates to this property are forbidden after creation.
    #[serde(default)]
    pub immutable: bool,
}

/// Legacy WAL property definition carried by v1 catalog-DDL schema changes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PropertyDefV1 {
    /// Property name.
    pub name: IStr,
    /// Property value type.
    pub value_type: ValueType,
    /// Whether `Value::Null` is allowed.
    pub nullable: bool,
    /// Optional default value.
    pub default: Option<Value>,
}

impl From<PropertyDefV1> for PropertyDef {
    fn from(value: PropertyDefV1) -> Self {
        Self {
            name: value.name,
            value_type: value.value_type,
            nullable: value.nullable,
            default: value.default,
            immutable: false,
        }
    }
}

/// Structural value type definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ValueType {
    /// Scalar predefined type.
    pub predefined: Option<PredefinedValueType>,
    /// Union member types.
    pub union: Option<Vec<ValueType>>,
    /// List element type. When present, this takes precedence over scalar
    /// fields for callers interpreting the type.
    pub list_of: Option<Box<ValueType>>,
    /// Closed record type reference.
    pub record: Option<RecordTypeRef>,
    /// Whether null is forbidden at this level.
    pub not_null: bool,
    /// Minimal v1.0 scalar cardinality.
    pub cardinality: ValueTypeCardinality,
}

impl ValueType {
    /// Construct a predefined scalar value type.
    #[must_use]
    pub const fn predefined(predefined: PredefinedValueType) -> Self {
        Self {
            predefined: Some(predefined),
            union: None,
            list_of: None,
            record: None,
            not_null: false,
            cardinality: ValueTypeCardinality::ExactlyOne,
        }
    }

    /// Construct a list value type.
    #[must_use]
    pub fn list_of(item: Self) -> Self {
        Self {
            predefined: None,
            union: None,
            list_of: Some(Box::new(item)),
            record: None,
            not_null: false,
            cardinality: ValueTypeCardinality::ExactlyOne,
        }
    }
}

/// Predefined value types claimed by the v1.0 surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PredefinedValueType {
    /// Boolean.
    Bool,
    /// Default signed integer.
    Int,
    /// 8-bit signed integer.
    Int8,
    /// 16-bit signed integer.
    Int16,
    /// 32-bit signed integer.
    Int32,
    /// 64-bit signed integer.
    Int64,
    /// 128-bit signed integer.
    Int128,
    /// Default unsigned integer.
    Uint,
    /// 8-bit unsigned integer.
    Uint8,
    /// 16-bit unsigned integer.
    Uint16,
    /// 32-bit unsigned integer.
    Uint32,
    /// 64-bit unsigned integer.
    Uint64,
    /// 128-bit unsigned integer.
    Uint128,
    /// Default floating-point number.
    Float,
    /// 32-bit floating-point number.
    Float32,
    /// 64-bit floating-point number.
    Float64,
    /// Fixed-precision decimal.
    Decimal,
    /// Interned string.
    String,
    /// Byte string.
    Bytes,
    /// Date.
    Date,
    /// Local time.
    LocalTime,
    /// Zoned time.
    ZonedTime,
    /// Local datetime.
    LocalDateTime,
    /// Zoned datetime.
    ZonedDateTime,
    /// Duration.
    Duration,
    /// Node reference.
    NodeRef,
    /// Edge reference.
    EdgeRef,
    /// Graph reference.
    GraphRef,
    /// Binding-table reference.
    TableRef,
    /// Path.
    Path,
    /// UUID.
    Uuid,
    /// Extension-owned value type.
    Extended(ExtensionTypeId),
}

/// Minimal v1.0 value cardinality.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ValueTypeCardinality {
    /// Exactly one value.
    ExactlyOne,
    /// Zero or one value.
    ZeroOrOne,
}

/// Closed record type definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecordTypeDef {
    /// Stable record type ID.
    pub id: RecordTypeId,
    /// Interned record type name.
    pub name: IStr,
    /// Field definitions in schema order.
    pub fields: SmallVec<[PropertyDef; 4]>,
}

/// Policy for relationships between key label sets.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum KeyLabelSetPolicy {
    /// Key label sets may not overlap.
    NoOverlap,
    /// Key label sets may be contained by one another. This is the v1.0
    /// default from spec 02 section 6.1.
    #[default]
    Containment,
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
