//! GQL type AST nodes.

use std::hash::{Hash, Hasher};

use selene_core::DbString;

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
    /// `USMALLINT`.
    USmallInt,
    /// `UINT`.
    Uint,
    /// `UBIGINT`.
    UBigInt,
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
    /// `REAL`.
    ///
    /// ISO floating-point type-name synonym with `FLOAT32` semantics.
    Real,
    /// `DOUBLE` or `DOUBLE PRECISION`.
    ///
    /// ISO floating-point type-name synonym with `FLOAT64` semantics.
    Double,
    /// Byte-string type.
    Bytes,
    /// Bounded byte-string type.
    ByteString(ByteStringType),
    /// `UUID`.
    Uuid,
    /// Native JSON value.
    Json,
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
    /// `DURATION (YEAR TO MONTH)`.
    DurationYearToMonth,
    /// `DURATION (DAY TO SECOND)`.
    DurationDayToSecond,
    /// Native dense-vector value.
    ///
    /// This internal type is used by procedure metadata and typed parameter
    /// validation. It is not parsed as a GQL type name; vector syntax remains
    /// outside the ISO grammar surface.
    Vector,
    /// `RECORD`.
    Record(RecordType),
    /// `LIST<T>`.
    List(Box<GqlType>),
    /// Explicitly non-null value type (`<value type> NOT NULL`).
    NotNull(Box<GqlType>),
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

/// Parsed bounded byte-string type metadata.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ByteStringType {
    /// Minimum byte length accepted by the type.
    pub min_len: u64,
    /// Maximum byte length accepted by the type.
    pub max_len: u64,
    /// Parsed syntactic form used for feature stamping.
    pub form: ByteStringTypeForm,
}

/// Parsed bounded byte-string syntactic form.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum ByteStringTypeForm {
    /// `BYTES(max)`.
    BytesMax,
    /// `BYTES(min,max)`.
    BytesMinMax,
    /// `BINARY(fixed)`.
    BinaryFixed,
    /// `VARBINARY(max)`.
    VarbinaryMax,
}

impl ByteStringType {
    /// Construct a byte-string type when the length bounds are valid.
    #[must_use]
    pub const fn new(min_len: u64, max_len: u64, form: ByteStringTypeForm) -> Option<Self> {
        if max_len == 0 || min_len > max_len {
            return None;
        }
        Some(Self {
            min_len,
            max_len,
            form,
        })
    }

    /// Return true if this type is fixed-length.
    #[must_use]
    pub const fn is_fixed_length(&self) -> bool {
        self.min_len == self.max_len
    }
}

impl PartialEq for ByteStringType {
    fn eq(&self, other: &Self) -> bool {
        self.min_len == other.min_len && self.max_len == other.max_len
    }
}

impl Eq for ByteStringType {}

impl Hash for ByteStringType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.min_len.hash(state);
        self.max_len.hash(state);
    }
}

/// Parsed record type.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum RecordType {
    /// Open record.
    Open,
    /// Closed record with named fields.
    Closed(Vec<(DbString, GqlType)>),
}

impl GqlType {
    /// Return the underlying value type after removing explicit `NOT NULL` wrappers.
    #[must_use]
    pub fn strip_not_null(&self) -> &Self {
        let mut ty = self;
        while let Self::NotNull(inner) = ty {
            ty = inner;
        }
        ty
    }

    /// Return true when this type is any duration family.
    #[must_use]
    pub fn is_duration(&self) -> bool {
        matches!(
            self.strip_not_null(),
            Self::Duration | Self::DurationYearToMonth | Self::DurationDayToSecond
        )
    }
}
