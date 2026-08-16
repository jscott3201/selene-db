//! Id-keyed immutable maps for engine-assigned graph identifiers.

use immutable_chunkmap::map::MapM;

/// Copy-on-write chunked tree for engine-assigned `NodeId`/`EdgeId` keys.
pub(crate) type EngineIdMap<K, V> = MapM<K, V>;

/// Construct an empty [`EngineIdMap`].
#[must_use]
pub(crate) fn engine_id_map<K: Clone + Ord, V: Clone>() -> EngineIdMap<K, V> {
    EngineIdMap::new()
}

/// Return the value for `key`, inserting its default when absent.
pub(crate) fn get_or_insert_default<K, V>(map: &mut MapM<K, V>, key: K) -> &mut V
where
    K: Clone + Ord,
    V: Clone + Default,
{
    map.get_or_insert_cow(key, V::default)
}
