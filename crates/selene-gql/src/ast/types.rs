//! GQL type AST nodes.

use selene_core::IStr;

/// Parsed GQL type.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum GqlType {
    /// `STRING`.
    String,
    /// `BOOLEAN`.
    Boolean,
    /// `INTEGER`.
    Integer,
    /// `FLOAT`.
    ///
    /// Width-generic floating-point type. Consumers deriving storage or
    /// conversion maps from parsed GQL types must accept both `f32` and `f64`
    /// values unless a narrower `FLOAT32` or `FLOAT64` was requested.
    Float,
    /// `INT8`.
    Int8,
    /// `INT16`.
    Int16,
    /// `INT32`.
    Int32,
    /// `INT64`.
    Int64,
    /// `INT128`.
    Int128,
    /// `UINT8`.
    Uint8,
    /// `UINT16`.
    Uint16,
    /// `UINT32`.
    Uint32,
    /// `UINT64`.
    Uint64,
    /// `UINT128`.
    Uint128,
    /// `SMALLINT`.
    SmallInt,
    /// `BIGINT`.
    BigInt,
    /// `DECIMAL`.
    Decimal,
    /// `FLOAT32`.
    Float32,
    /// `FLOAT64`.
    Float64,
    /// Byte-string type.
    Bytes,
    /// `BINARY`.
    Binary,
    /// `VARBINARY`.
    VarBinary,
    /// `ZONED DATETIME`.
    ZonedDateTime,
    /// `LOCAL DATETIME`.
    LocalDateTime,
    /// `DATE`.
    Date,
    /// `ZONED TIME`.
    ZonedTime,
    /// `LOCAL TIME`.
    LocalTime,
    /// `DURATION`.
    Duration,
    /// `RECORD`.
    Record(RecordType),
    /// `LIST<T>`.
    List(Box<GqlType>),
    /// `PATH`.
    Path,
    /// Graph reference.
    GraphRef,
    /// Node reference.
    NodeRef,
    /// Edge reference.
    EdgeRef,
    /// Binding-table reference.
    TableRef,
    /// `NULL`.
    Null,
    /// `NOTHING`.
    Nothing,
}

/// Parsed record type.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum RecordType {
    /// Open record.
    Open,
    /// Closed record with named fields.
    Closed(Vec<(IStr, GqlType)>),
}
