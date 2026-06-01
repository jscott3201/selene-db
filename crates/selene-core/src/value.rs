//! In-memory GQL value representation per spec 02 section 3.
//!
//! The [`Value`] variant order is canonical and append-only. Reordering,
//! removing, or inserting variants in the middle is a major-version and
//! durability-format change. The serde/postcard and rkyv serialization
//! derives are part of the same durability contract.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::extension_type_ids::ExtensionTypeId;
use crate::identity::{BindingTableId, EdgeId, GraphId, NodeId, RecordTypeId};
use crate::istr::IStr;

/// In-memory representation of a GQL value.
///
/// IA001: default floating-point arithmetic is IEEE 754 binary64; `Float32`
/// remains distinct for schema storage. Rust equality preserves GQL's
/// `+0.0 == -0.0` behavior, while NaN ordering is handled by query-engine
/// `ORDER BY` logic outside this crate.
///
/// Value equality matches IEEE 754 for non-NaN AND treats all NaN bit-patterns
/// as equal for round-trip integrity. This is the internal Rust-level equality
/// used by `PropertyMap` serde round-trip and snapshot diffs. The GQL `=`
/// operator is intercepted at the runtime layer (`runtime::value_compare`)
/// and preserves ISO 3VL semantics — `NaN = NaN` returns NULL there.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Value {
    /// Boolean value.
    Bool(bool),
    /// Signed integer up to 64 bits.
    Int(i64),
    /// Unsigned integer up to 64 bits.
    Uint(u64),
    /// Signed 128-bit integer.
    Int128(#[serde(with = "serde_i128_le")] i128),
    /// Unsigned 128-bit integer.
    Uint128(#[serde(with = "serde_u128_le")] u128),
    /// Default floating-point value.
    Float(f64),
    /// Distinct 32-bit floating-point value.
    Float32(f32),
    /// Fixed-precision decimal value.
    Decimal(#[serde(with = "serde_decimal_str")] rust_decimal::Decimal),
    /// String value.
    String(IStr),
    /// Byte-string value.
    Bytes(Arc<[u8]>),
    /// List value.
    List(Vec<Value>),
    /// Open record value.
    Record(Box<Record>),
    /// Closed record value tied to a graph-type-defined record type.
    RecordTyped(Box<RecordTyped>),
    /// Path value.
    Path(Path),
    /// Node reference value.
    NodeRef(NodeId),
    /// Edge reference value.
    EdgeRef(EdgeId),
    /// Graph reference value.
    GraphRef(GraphId),
    /// Binding-table reference value.
    TableRef(BindingTableId),
    /// Zoned datetime value.
    ZonedDateTime(jiff::Zoned),
    /// Local datetime value.
    LocalDateTime(jiff::civil::DateTime),
    /// Date value.
    Date(jiff::civil::Date),
    /// Zoned time value.
    ///
    /// `jiff` 0.2 has no dedicated zoned-time type, so selene-core uses
    /// `jiff::Zoned`; date components are ignored at the GQL boundary.
    ZonedTime(jiff::Zoned),
    /// Local time value.
    LocalTime(jiff::civil::Time),
    /// Duration value.
    Duration(jiff::Span),
    /// Extension-owned opaque payload.
    Extended {
        /// Registered extension type ID.
        type_id: ExtensionTypeId,
        /// Extension-owned byte payload.
        payload: Arc<[u8]>,
    },
    /// Null value.
    Null,
    /// UUID value.
    Uuid(uuid::Uuid),
}

/// Compile-time ceiling on `size_of::<Value>` — a zero-cost re-bloat tripwire.
///
/// `Value` is moved and cloned on every property read, `PropertyMap` copy, and
/// binding-table row, so its in-memory size is a hot-path cost multiplier (see
/// the `value_clone` bench). It is currently 128 bytes on 64-bit targets,
/// dominated by the inlined `Duration(jiff::Span)` (64 B) and `jiff::Zoned`
/// time variants. This assert fails the build if a future variant grows the
/// enum; **lower the ceiling** when CORE-06 boxes the large time variants.
const _: () = assert!(core::mem::size_of::<Value>() <= 128);

impl Value {
    /// Factory table with one sample value for each [`Value`] variant.
    ///
    /// The table is used by tests as an append-only ANCHOR: adding a new
    /// variant requires adding one factory here so the source-of-truth crate
    /// owns the variant census.
    pub const ALL: &[fn() -> Self] = &[
        || Self::Bool(false),
        || Self::Int(0),
        || Self::Uint(0),
        || Self::Int128(0),
        || Self::Uint128(0),
        || Self::Float(0.0),
        || Self::Float32(0.0),
        || Self::Decimal(rust_decimal::Decimal::ZERO),
        || Self::String(value_variant_istr("value.all.string")),
        || Self::Bytes(Arc::from([0_u8])),
        || Self::List(Vec::new()),
        || Self::Record(Box::new(Record::Open(SmallVec::new()))),
        || {
            Self::RecordTyped(Box::new(RecordTyped {
                type_id: RecordTypeId::new(1),
                values: SmallVec::new(),
            }))
        },
        || {
            Self::Path(Path {
                graph: GraphId::new(1),
                start: NodeId::new(1),
                segments: SmallVec::new(),
            })
        },
        || Self::NodeRef(NodeId::new(1)),
        || Self::EdgeRef(EdgeId::new(1)),
        || Self::GraphRef(GraphId::new(1)),
        || Self::TableRef(BindingTableId::new(1)),
        || Self::ZonedDateTime(value_variant_zoned()),
        || Self::LocalDateTime("2024-01-01T00:00:00".parse().unwrap()),
        || Self::Date("2024-01-01".parse().unwrap()),
        || Self::ZonedTime(value_variant_zoned()),
        || Self::LocalTime("00:00:00".parse().unwrap()),
        || Self::Duration("PT1S".parse().unwrap()),
        || Self::Extended {
            type_id: ExtensionTypeId::FIRST_PARTY_MIN,
            payload: Arc::from([0_u8]),
        },
        || Self::Null,
        || Self::Uuid(uuid::Uuid::nil()),
    ];

    /// Number of known [`Value`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this value variant.
    ///
    /// This match is exhaustive in `selene-core`, so a future variant addition
    /// forces the defining crate to choose the new public name once.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "Bool",
            Self::Int(_) => "Int",
            Self::Uint(_) => "Uint",
            Self::Int128(_) => "Int128",
            Self::Uint128(_) => "Uint128",
            Self::Float(_) => "Float",
            Self::Float32(_) => "Float32",
            Self::Decimal(_) => "Decimal",
            Self::String(_) => "String",
            Self::Bytes(_) => "Bytes",
            Self::List(_) => "List",
            Self::Record(_) => "Record",
            Self::RecordTyped(_) => "RecordTyped",
            Self::Path(_) => "Path",
            Self::NodeRef(_) => "NodeRef",
            Self::EdgeRef(_) => "EdgeRef",
            Self::GraphRef(_) => "GraphRef",
            Self::TableRef(_) => "TableRef",
            Self::ZonedDateTime(_) => "ZonedDateTime",
            Self::LocalDateTime(_) => "LocalDateTime",
            Self::Date(_) => "Date",
            Self::ZonedTime(_) => "ZonedTime",
            Self::LocalTime(_) => "LocalTime",
            Self::Duration(_) => "Duration",
            Self::Extended { .. } => "Extended",
            Self::Null => "Null",
            Self::Uuid(_) => "Uuid",
        }
    }
}

fn value_variant_istr(name: &str) -> IStr {
    crate::intern(name).expect("Value::ALL fixture strings fit the process interner cap")
}

fn value_variant_zoned() -> jiff::Zoned {
    jiff::Timestamp::new(0, 0)
        .expect("Value::ALL timestamp fixture is in range")
        .to_zoned(jiff::tz::TimeZone::UTC)
}

impl PartialEq for Value {
    fn eq(&self, rhs: &Self) -> bool {
        match (self, rhs) {
            (Self::Bool(lhs), Self::Bool(rhs)) => lhs == rhs,
            (Self::Int(lhs), Self::Int(rhs)) => lhs == rhs,
            (Self::Uint(lhs), Self::Uint(rhs)) => lhs == rhs,
            (Self::Int128(lhs), Self::Int128(rhs)) => lhs == rhs,
            (Self::Uint128(lhs), Self::Uint128(rhs)) => lhs == rhs,
            (Self::Float(lhs), Self::Float(rhs)) => lhs == rhs || (lhs.is_nan() && rhs.is_nan()),
            (Self::Float32(lhs), Self::Float32(rhs)) => {
                lhs == rhs || (lhs.is_nan() && rhs.is_nan())
            }
            (Self::Decimal(lhs), Self::Decimal(rhs)) => lhs == rhs,
            (Self::String(lhs), Self::String(rhs)) => lhs == rhs,
            (Self::Bytes(lhs), Self::Bytes(rhs)) => lhs == rhs,
            (Self::List(lhs), Self::List(rhs)) => lhs == rhs,
            (Self::Record(lhs), Self::Record(rhs)) => lhs == rhs,
            (Self::RecordTyped(lhs), Self::RecordTyped(rhs)) => lhs == rhs,
            (Self::Path(lhs), Self::Path(rhs)) => lhs == rhs,
            (Self::NodeRef(lhs), Self::NodeRef(rhs)) => lhs == rhs,
            (Self::EdgeRef(lhs), Self::EdgeRef(rhs)) => lhs == rhs,
            (Self::GraphRef(lhs), Self::GraphRef(rhs)) => lhs == rhs,
            (Self::TableRef(lhs), Self::TableRef(rhs)) => lhs == rhs,
            (Self::ZonedDateTime(lhs), Self::ZonedDateTime(rhs)) => lhs == rhs,
            (Self::LocalDateTime(lhs), Self::LocalDateTime(rhs)) => lhs == rhs,
            (Self::Date(lhs), Self::Date(rhs)) => lhs == rhs,
            (Self::ZonedTime(lhs), Self::ZonedTime(rhs)) => lhs == rhs,
            (Self::LocalTime(lhs), Self::LocalTime(rhs)) => lhs == rhs,
            (Self::Duration(lhs), Self::Duration(rhs)) => lhs.fieldwise() == rhs.fieldwise(),
            (
                Self::Extended {
                    type_id: lhs_type_id,
                    payload: lhs_payload,
                },
                Self::Extended {
                    type_id: rhs_type_id,
                    payload: rhs_payload,
                },
            ) => lhs_type_id == rhs_type_id && lhs_payload == rhs_payload,
            (Self::Null, Self::Null) => true,
            (Self::Uuid(lhs), Self::Uuid(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

/// Open record value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Record {
    /// Open `RECORD` literal in expressions.
    Open(SmallVec<[(IStr, Value); 4]>),
}

/// Closed record value tied to a graph-type-defined record type.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecordTyped {
    /// Identifier pointing to a `RecordTypeDef` in the graph type catalog.
    pub type_id: RecordTypeId,
    /// Positional values aligned with the record type's field list.
    pub values: SmallVec<[Option<Value>; 4]>,
}

/// Path value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Path {
    /// Graph the path lives within.
    pub graph: GraphId,
    /// Starting node of the path.
    pub start: NodeId,
    /// Ordered segments traversed.
    pub segments: SmallVec<[PathSegment; 4]>,
}

/// One traversal step in a [`Path`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PathSegment {
    /// Edge traversed in this step.
    pub edge: EdgeId,
    /// Direction of edge traversal.
    pub direction: EdgeDirection,
    /// Node reached after this step.
    pub node: NodeId,
}

/// Direction of edge traversal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum EdgeDirection {
    /// Source-to-target traversal of a directed edge.
    Outgoing,
    /// Target-to-source traversal of a directed edge.
    Incoming,
    /// Undirected edge.
    Undirected,
}

mod serde_i128_le {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(value: &i128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.to_le_bytes().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<i128, D::Error>
    where
        D: Deserializer<'de>,
    {
        <[u8; 16]>::deserialize(deserializer).map(i128::from_le_bytes)
    }
}

mod serde_u128_le {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.to_le_bytes().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        <[u8; 16]>::deserialize(deserializer).map(u128::from_le_bytes)
    }
}

mod serde_decimal_str {
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(
        value: &rust_decimal::Decimal,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<rust_decimal::Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        rust_decimal::Decimal::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{PropertyMap, intern};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn value_is_send_sync() {
        assert_send_sync::<Value>();
    }

    #[test]
    fn representative_variants_clone_and_compare() {
        let values = [
            Value::Bool(true),
            Value::Int(42),
            Value::String(intern("name").unwrap()),
            Value::Bytes(Arc::from([1_u8, 2, 3])),
            Value::NodeRef(NodeId::new(1)),
            Value::Null,
            Value::Uuid(uuid::Uuid::nil()),
        ];
        for value in values {
            assert_eq!(value.clone(), value);
        }
    }

    #[test]
    fn edge_direction_variants_are_distinct() {
        assert_ne!(EdgeDirection::Outgoing, EdgeDirection::Incoming);
        assert_ne!(EdgeDirection::Outgoing, EdgeDirection::Undirected);
        assert_ne!(EdgeDirection::Incoming, EdgeDirection::Undirected);
    }

    #[test]
    fn path_clone_round_trips() {
        let mut segments = SmallVec::new();
        segments.push(PathSegment {
            edge: EdgeId::new(3),
            direction: EdgeDirection::Outgoing,
            node: NodeId::new(4),
        });
        let path = Path {
            graph: GraphId::new(1),
            start: NodeId::new(2),
            segments,
        };
        assert_eq!(path.clone(), path);
    }

    #[test]
    fn value_discriminant_size_is_stable_on_this_target() {
        assert!(std::mem::size_of::<Value>() >= std::mem::size_of::<usize>());
    }

    #[test]
    fn value_all_covers_every_variant() {
        assert_eq!(Value::VARIANT_COUNT, 27);
        let mut discriminants = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for factory in Value::ALL {
            let value = factory();
            assert!(
                discriminants.insert(std::mem::discriminant(&value)),
                "Value::ALL has duplicate variant: {}",
                value.variant_name()
            );
            let name = value.variant_name();
            assert!(!name.is_empty(), "Value::variant_name must not be empty");
            assert!(names.insert(name), "Value::variant_name collision: {name}");
        }
        assert_eq!(discriminants.len(), Value::ALL.len());
        assert_eq!(names.len(), Value::ALL.len());
    }

    #[test]
    fn value_float_nan_eq_bit_exact() {
        assert_eq!(Value::Float(f64::NAN), Value::Float(f64::NAN));
    }

    #[test]
    fn value_float32_nan_eq_bit_exact() {
        assert_eq!(Value::Float32(f32::NAN), Value::Float32(f32::NAN));
    }

    #[test]
    fn value_float_signed_zero_eq_preserved() {
        assert_eq!(Value::Float(0.0), Value::Float(-0.0));
    }

    #[test]
    fn value_property_map_round_trip_nan() {
        let original = PropertyMap::from_pairs([(intern("x").unwrap(), Value::Float(f64::NAN))])
            .expect("property map builds");
        let bytes = postcard::to_allocvec(&original).expect("property map serializes");
        let decoded: PropertyMap = postcard::from_bytes(&bytes).expect("property map deserializes");

        assert_eq!(original, decoded);
    }

    proptest! {
        #[test]
        fn random_short_path_clones(segment_count in 0_usize..=4) {
            let mut segments = SmallVec::<[PathSegment; 4]>::new();
            for idx in 0..segment_count {
                segments.push(PathSegment {
                    edge: EdgeId::new(idx as u64 + 1),
                    direction: EdgeDirection::Outgoing,
                    node: NodeId::new(idx as u64 + 2),
                });
            }
            let path = Path {
                graph: GraphId::new(1),
                start: NodeId::new(1),
                segments,
            };
            prop_assert_eq!(path.clone(), path);
        }
    }
}
