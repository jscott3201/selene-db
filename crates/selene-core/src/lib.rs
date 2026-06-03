//! Foundation types for the selene-db ISO/IEC 39075:2024 GQL property graph
//! engine.
//!
//! This crate is the dependency root: every other selene crate transitively
//! depends on it. Per D8, `selene-core` has zero dependencies on other selene
//! crates. The mandatory data types of ISO GQL minimum conformance live here
//! (`STRING`, `BOOLEAN`, `INT`, `FLOAT`); composite, temporal, and reference
//! value types are also normatively defined in Spec 02 and implemented here.
//! The crate now also carries the composite containers,
//! schema model, origin metadata, and WAL change payload
//! types needed by downstream crates.
//!
//! See Spec 02 for the full data model specification.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cancellation;
pub mod change_kind;
pub mod changeset;
mod changeset_variants;
pub mod error;
pub mod extension_type_ids;
pub mod feature_register;
pub mod gqlstatus;
pub mod hlc;
pub mod identity;
pub mod istr;
pub mod label_set;
pub mod metrics;
pub mod origin;
pub mod property_map;
pub mod property_value_type;
pub mod reserved;
pub mod schema;
pub mod value;
pub mod vector;

pub use cancellation::{CancellationCause, CancellationChecker, CancellationToken};
pub use change_kind::ChangeKind;
pub use changeset::{Change, LabelDiff, PropertyDiff, SchemaChange, SchemaPropertyIndexKind};
pub use error::{CoreError, CoreResult};
pub use extension_type_ids::{
    ExtensionTypeId, FIRST_PARTY_EXTENSION_TYPE_IDS, SELENE_RDF, SELENE_TIMESERIES,
};
pub use gqlstatus::{ALL_GQLSTATUS_NAMES, gqlstatus_name};
pub use hlc::HlcTimestamp;
pub use identity::{BindingTableId, EdgeId, GraphId, NodeId, RecordTypeId};
pub use istr::{IStr, intern, resolve};
pub use label_set::LabelSet;
pub use origin::Origin;
pub use property_map::{PropertyMap, PropertyMapIter, PropertyMapKeys, PropertyMapValues};
pub use property_value_type::PropertyValueType;
pub use reserved::RESERVED_LABEL_PREFIX;
pub use schema::{
    EdgeEndpointDef, EdgeTypeDef, EdgeTypeDefV1, GraphType, GraphTypeId, KeyLabelSetPolicy,
    NodeKey, NodeTypeDef, NodeTypeDefV1, NodeTypeRef, PredefinedValueType, PropertyDef,
    PropertyDefV1, RecordFieldStructure, RecordFieldStructureDef, RecordFieldStructureType,
    RecordTypeDef, RecordTypeRef, ValidationMode, ValueType, ValueTypeCardinality,
};
pub use value::{
    EdgeDirection, MAX_VECTOR_DIMENSION, Path, PathSegment, Record, RecordTyped, Value, VectorValue,
};
pub use vector::{VectorMetric, VectorSearchHit, exact_vector_top_k};

#[cfg(test)]
mod serde_tests;
