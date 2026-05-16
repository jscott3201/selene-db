//! In-memory GQL value representation per spec 02 section 3.
//!
//! The [`Value`] variant order is canonical and append-only. Reordering,
//! removing, or inserting variants in the middle is a major-version and
//! durability-format change. Serialization derives intentionally land in
//! BRIEF-06 alongside the `Codec` trait.

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
    /// Interned string value.
    String(IStr),
    /// Non-interned string sourced from outside the global interner namespace.
    ///
    /// Use this variant when surfacing high-cardinality or free-form text from a
    /// source the engine cannot admit to the global [`IStr`] pool without
    /// risking cap exhaustion: validation diagnostic strings, per-row content
    /// hashes, external system payloads.
    ///
    /// `Value::ExternalString` and `Value::String` are not equal under
    /// [`PartialEq`] even if their underlying `&str` content matches. `Value`
    /// equality is variant-strict; GQL string-content equality is handled by
    /// executor comparison logic.
    ExternalString(#[serde(with = "serde_arc_str")] Arc<str>),
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

impl PartialEq for Value {
    fn eq(&self, rhs: &Self) -> bool {
        // Value::Hash deferred to BRIEF-104; ensure NaN bit-patterns hash equal there.
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
            (Self::ExternalString(lhs), Self::ExternalString(rhs)) => lhs == rhs,
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

mod serde_arc_str {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(value: &Arc<str>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_ref().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Arc<str>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let owned = String::deserialize(deserializer)?;
        Ok(Arc::from(owned.into_boxed_str()))
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
            Value::ExternalString(Arc::from("external name")),
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
    fn external_string_equality_is_variant_strict() {
        let interned = Value::String(intern("same text").unwrap());
        let external = Value::ExternalString(Arc::from("same text"));

        assert_eq!(
            Value::ExternalString(Arc::from("same text")),
            Value::ExternalString(Arc::from("same text"))
        );
        assert_ne!(interned, external);
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
