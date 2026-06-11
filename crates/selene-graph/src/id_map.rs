//! Id-keyed immutable maps for engine-assigned graph identifiers.

use imbl::{GenericHashMap, shared_ptr::DefaultSharedPtr};
use rustc_hash::FxBuildHasher;

/// Copy-on-write HAMT for engine-assigned `NodeId`/`EdgeId` keys.
///
/// HashDoS posture: these maps are keyed only by monotonically allocated,
/// engine-assigned graph identifiers, never by attacker-chosen strings or
/// property data. Keep user-visible keys such as labels and property names on
/// their existing map types unless a separate design justifies otherwise.
pub(crate) type EngineIdMap<K, V> = GenericHashMap<K, V, FxBuildHasher, DefaultSharedPtr>;

/// Construct an empty [`EngineIdMap`] using `FxBuildHasher`.
#[must_use]
pub(crate) fn engine_id_map<K, V>() -> EngineIdMap<K, V> {
    EngineIdMap::with_hasher(FxBuildHasher)
}
