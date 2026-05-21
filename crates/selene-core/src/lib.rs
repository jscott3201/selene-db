//! Foundation types for the selene-db ISO/IEC 39075:2024 GQL property graph
//! engine.
//!
//! This crate is the dependency root: every other selene crate transitively
//! depends on it. Per D8, `selene-core` has zero dependencies on other selene
//! crates. The mandatory data types of ISO GQL minimum conformance live here
//! (`STRING`, `BOOLEAN`, `INT`, `FLOAT`); composite, temporal, and reference
//! value types are also normatively defined in Spec 02 and implemented here.
//! The crate now also carries the composite containers,
//! schema model, transient codec hooks, origin metadata, and WAL change payload
//! types needed by downstream crates.
//!
//! See Spec 02 for the full data model specification.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cancellation;
pub mod changeset;
mod changeset_variants;
pub mod codec;
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
pub mod pack_lifecycle;
pub mod property_map;
pub mod property_value_type;
pub mod schema;
pub mod value;
pub mod value_adapter;

pub use cancellation::{CancellationCause, CancellationChecker, CancellationToken};
pub use changeset::{Change, LabelDiff, PropertyDiff, SchemaChange, SchemaPropertyIndexKind};
pub use codec::{Codec, CodecError};
pub use error::{CoreError, CoreResult};
pub use extension_type_ids::{
    ExtensionTypeId, FIRST_PARTY_EXTENSION_TYPE_IDS, SELENE_FULLTEXT, SELENE_RDF,
    SELENE_TIMESERIES, SELENE_VECTOR,
};
pub use gqlstatus::{ALL_GQLSTATUS_NAMES, gqlstatus_name};
pub use hlc::HlcTimestamp;
pub use identity::{BindingTableId, EdgeId, GraphId, NodeId, RecordTypeId};
pub use istr::{
    AdmissionError, IStr, IStrAdmissionPolicy, intern, intern_atomic_admit, intern_or_external,
    intern_with_admission, lookup, resolve,
};
pub use label_set::LabelSet;
pub use origin::Origin;
pub use pack_lifecycle::PackLifecycleEvent;
pub use property_map::PropertyMap;
pub use property_value_type::PropertyValueType;
pub use schema::{
    EdgeTypeDef, GraphType, GraphTypeId, KeyLabelSetPolicy, NodeKey, NodeTypeDef, NodeTypeRef,
    PredefinedValueType, PropertyDef, RecordTypeDef, RecordTypeRef, ValidationMode, ValueType,
    ValueTypeCardinality,
};
pub use value::{EdgeDirection, Path, PathSegment, Record, RecordTyped, Value};
pub use value_adapter::{
    ValueTypeAdapter, ValueTypeRegistry, register_value_type, value_type_registry,
};

#[cfg(test)]
mod serde_tests;
