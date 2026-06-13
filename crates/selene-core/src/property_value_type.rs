//! Runtime property value type tags for closed graph validation.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{DurationTypeQualifier, Value};

/// Closed-graph property value type tags.
///
/// The variant order follows [`Value`]'s canonical durable order, excluding
/// `Value::Extended`, which is not declarable as a closed-graph property type.
///
/// The [`Self::List`] and [`Self::Record`] / [`Self::RecordTyped`] tags are
/// deliberately **structure-blind**: they identify only the container kind, not
/// the element type of a `LIST<T>` or the field types of a `RECORD{..}`. The
/// closed-graph validator validates element and field types structurally via
/// `list_element_type` / `record_field_types` on the graph-type definition (not
/// through these flat tags), so a `LIST<INT>` column still rejects a
/// `LIST<STRING>` value. These tags answer "is this a list?", not "is this a
/// list of the right element type?".
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum PropertyValueType {
    /// Boolean value.
    Bool,
    /// Signed integer up to 64 bits.
    Int,
    /// Unsigned integer up to 64 bits.
    Uint,
    /// Signed 128-bit integer.
    Int128,
    /// Unsigned 128-bit integer.
    Uint128,
    /// Default floating-point value.
    Float,
    /// Distinct 32-bit floating-point value.
    Float32,
    /// Fixed-precision decimal value.
    Decimal,
    /// Database-string value.
    String,
    /// Byte-string value.
    Bytes,
    /// List value.
    List,
    /// Open record value.
    Record,
    /// Closed record value.
    RecordTyped,
    /// Path value.
    Path,
    /// Node reference value.
    NodeRef,
    /// Edge reference value.
    EdgeRef,
    /// Graph reference value.
    GraphRef,
    /// Binding-table reference value.
    TableRef,
    /// Zoned datetime value.
    ZonedDateTime,
    /// Local datetime value.
    LocalDateTime,
    /// Date value.
    Date,
    /// Zoned time value.
    ZonedTime,
    /// Local time value.
    LocalTime,
    /// Duration value.
    Duration,
    /// Year-month duration value.
    DurationYearToMonth,
    /// Day-time duration value.
    DurationDayToSecond,
    /// Null value.
    Null,
    /// UUID value.
    Uuid,
    /// Native dense vector value.
    Vector,
    /// Native JSON value.
    Json,
}

impl PropertyValueType {
    /// Return the closed-graph type tag for a value.
    ///
    /// `Value::Extended` returns `None` because extension-owned values are not
    /// declarable as closed-graph property types.
    #[must_use]
    pub fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(_) => Some(Self::Bool),
            Value::Int(_) => Some(Self::Int),
            Value::Uint(_) => Some(Self::Uint),
            Value::Int128(_) => Some(Self::Int128),
            Value::Uint128(_) => Some(Self::Uint128),
            Value::Float(_) => Some(Self::Float),
            Value::Float32(_) => Some(Self::Float32),
            Value::Decimal(_) => Some(Self::Decimal),
            Value::String(_) => Some(Self::String),
            Value::Bytes(_) => Some(Self::Bytes),
            Value::List(_) => Some(Self::List),
            Value::Record(_) => Some(Self::Record),
            Value::RecordTyped(_) => Some(Self::RecordTyped),
            Value::Path(_) => Some(Self::Path),
            Value::NodeRef(_) => Some(Self::NodeRef),
            Value::EdgeRef(_) => Some(Self::EdgeRef),
            Value::GraphRef(_) => Some(Self::GraphRef),
            Value::TableRef(_) => Some(Self::TableRef),
            Value::ZonedDateTime(_) => Some(Self::ZonedDateTime),
            Value::LocalDateTime(_) => Some(Self::LocalDateTime),
            Value::Date(_) => Some(Self::Date),
            Value::ZonedTime(_) => Some(Self::ZonedTime),
            Value::LocalTime(_) => Some(Self::LocalTime),
            Value::Duration(_) => Some(Self::Duration),
            Value::Extended { .. } => None,
            Value::Null => Some(Self::Null),
            Value::Uuid(_) => Some(Self::Uuid),
            Value::Vector(_) => Some(Self::Vector),
            Value::Json(_) => Some(Self::Json),
        }
    }

    /// Return true when `value` belongs to this type.
    #[must_use]
    pub fn matches(self, value: &Value) -> bool {
        match (self, value) {
            (Self::DurationYearToMonth, Value::Duration(value)) => {
                DurationTypeQualifier::YearToMonth.matches_span(value)
            }
            (Self::DurationDayToSecond, Value::Duration(value)) => {
                DurationTypeQualifier::DayToSecond.matches_span(value)
            }
            _ => Self::of(value) == Some(self),
        }
    }

    /// Stable diagnostic name for an observed value.
    #[must_use]
    pub fn observed_name(value: &Value) -> &'static str {
        match Self::of(value) {
            Some(kind) => kind.name(),
            None => "Extended",
        }
    }

    /// Stable diagnostic name for this type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Int => "Int",
            Self::Uint => "Uint",
            Self::Int128 => "Int128",
            Self::Uint128 => "Uint128",
            Self::Float => "Float",
            Self::Float32 => "Float32",
            Self::Decimal => "Decimal",
            Self::String => "String",
            Self::Bytes => "Bytes",
            Self::List => "List",
            Self::Record => "Record",
            Self::RecordTyped => "RecordTyped",
            Self::Path => "Path",
            Self::NodeRef => "NodeRef",
            Self::EdgeRef => "EdgeRef",
            Self::GraphRef => "GraphRef",
            Self::TableRef => "TableRef",
            Self::ZonedDateTime => "ZonedDateTime",
            Self::LocalDateTime => "LocalDateTime",
            Self::Date => "Date",
            Self::ZonedTime => "ZonedTime",
            Self::LocalTime => "LocalTime",
            Self::Duration => "Duration",
            Self::DurationYearToMonth => "DurationYearToMonth",
            Self::DurationDayToSecond => "DurationDayToSecond",
            Self::Null => "Null",
            Self::Uuid => "Uuid",
            Self::Vector => "Vector",
            Self::Json => "Json",
        }
    }
}

impl fmt::Display for PropertyValueType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use smallvec::smallvec;

    use super::*;
    use crate::{
        BindingTableId, EdgeDirection, EdgeId, ExtensionTypeId, GraphId, JsonValue, NodeId, Path,
        PathSegment, Record, RecordTypeId, RecordTyped, VectorValue, db_string,
    };

    fn sample_values() -> Vec<(PropertyValueType, Value)> {
        vec![
            (PropertyValueType::Bool, Value::Bool(true)),
            (PropertyValueType::Int, Value::Int(-1)),
            (PropertyValueType::Uint, Value::Uint(1)),
            (PropertyValueType::Int128, Value::Int128(-1)),
            (PropertyValueType::Uint128, Value::Uint128(1)),
            (PropertyValueType::Float, Value::Float(1.25)),
            (PropertyValueType::Float32, Value::Float32(1.5)),
            (
                PropertyValueType::Decimal,
                Value::Decimal("123.45".parse().unwrap()),
            ),
            (
                PropertyValueType::String,
                Value::String(db_string("property-value-type.string").unwrap()),
            ),
            (
                PropertyValueType::Bytes,
                Value::Bytes(Arc::from([1_u8, 2, 3])),
            ),
            (PropertyValueType::List, Value::List(vec![Value::Int(1)])),
            (
                PropertyValueType::Record,
                Value::Record(Box::new(Record::Open(smallvec![(
                    db_string("property-value-type.field").unwrap(),
                    Value::Bool(true),
                )]))),
            ),
            (
                PropertyValueType::RecordTyped,
                Value::RecordTyped(Box::new(RecordTyped {
                    type_id: RecordTypeId::new(1),
                    values: smallvec![Some(Value::Int(1))],
                })),
            ),
            (
                PropertyValueType::Path,
                Value::Path(Box::new(Path {
                    graph: GraphId::new(1),
                    start: NodeId::new(1),
                    segments: smallvec![PathSegment {
                        edge: EdgeId::new(1),
                        direction: EdgeDirection::Outgoing,
                        node: NodeId::new(2),
                    }],
                })),
            ),
            (PropertyValueType::NodeRef, Value::NodeRef(NodeId::new(1))),
            (PropertyValueType::EdgeRef, Value::EdgeRef(EdgeId::new(1))),
            (
                PropertyValueType::GraphRef,
                Value::GraphRef(GraphId::new(1)),
            ),
            (
                PropertyValueType::TableRef,
                Value::TableRef(BindingTableId::new(1)),
            ),
            (
                PropertyValueType::ZonedDateTime,
                Value::ZonedDateTime(Box::new(
                    "2026-05-07T12:34:56-04:00[America/New_York]"
                        .parse()
                        .unwrap(),
                )),
            ),
            (
                PropertyValueType::LocalDateTime,
                Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
            ),
            (
                PropertyValueType::Date,
                Value::Date("2026-05-07".parse().unwrap()),
            ),
            (
                PropertyValueType::ZonedTime,
                Value::ZonedTime(Box::new(
                    "2026-05-07T12:34:56-04:00[America/New_York]"
                        .parse()
                        .unwrap(),
                )),
            ),
            (
                PropertyValueType::LocalTime,
                Value::LocalTime("12:34:56".parse().unwrap()),
            ),
            (
                PropertyValueType::Duration,
                Value::Duration(Box::new("PT1H2S".parse().unwrap())),
            ),
            (PropertyValueType::Null, Value::Null),
            (
                PropertyValueType::Uuid,
                Value::Uuid(uuid::Uuid::from_u128(42)),
            ),
            (
                PropertyValueType::Vector,
                Value::Vector(VectorValue::new(vec![1.0, 2.0, 3.0]).unwrap()),
            ),
            (
                PropertyValueType::Json,
                Value::Json(JsonValue::new(serde_json::json!({"kind": "sample"})).unwrap()),
            ),
        ]
    }

    #[test]
    fn matches_canonical_value_variants() {
        for (expected, value) in sample_values() {
            assert_eq!(PropertyValueType::of(&value), Some(expected));
            assert!(expected.matches(&value));
        }
    }

    #[test]
    fn qualified_duration_types_match_field_families() {
        let year_month = Value::Duration(Box::new("P1Y2M".parse().unwrap()));
        let day_time = Value::Duration(Box::new("P3DT4H".parse().unwrap()));
        let zero = Value::Duration(Box::new("PT0S".parse().unwrap()));

        assert!(PropertyValueType::DurationYearToMonth.matches(&year_month));
        assert!(!PropertyValueType::DurationYearToMonth.matches(&day_time));
        assert!(PropertyValueType::DurationYearToMonth.matches(&zero));
        assert!(PropertyValueType::DurationDayToSecond.matches(&day_time));
        assert!(!PropertyValueType::DurationDayToSecond.matches(&year_month));
        assert!(PropertyValueType::DurationDayToSecond.matches(&zero));
    }

    #[test]
    fn rejects_extension_value_for_every_declarable_type() {
        let value = Value::Extended {
            type_id: ExtensionTypeId(0x100),
            payload: Arc::from([1_u8, 2, 3]),
        };
        assert_eq!(PropertyValueType::of(&value), None);
        for (kind, _) in sample_values() {
            assert!(!kind.matches(&value));
        }
    }

    #[test]
    fn display_uses_stable_type_name() {
        assert_eq!(PropertyValueType::RecordTyped.to_string(), "RecordTyped");
    }

    #[test]
    fn rkyv_round_trips() {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&PropertyValueType::Path).unwrap();
        let decoded = rkyv::from_bytes::<PropertyValueType, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(decoded, PropertyValueType::Path);
    }
}
