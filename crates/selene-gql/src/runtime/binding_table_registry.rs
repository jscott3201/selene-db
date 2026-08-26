//! Request-scoped binding-table registry.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use selene_core::BindingTableId;

use crate::runtime::BindingTable;

static NEXT_AUTHORITY: AtomicU32 = AtomicU32::new(1);

#[derive(Default)]
struct RegistryState {
    tables: FxHashMap<BindingTableId, Arc<BindingTable>>,
    next_local_id: u32,
}

/// Failure to resolve a request-scoped binding-table reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum BindingTableLookupError {
    /// The reserved zero sentinel is never a valid table reference.
    #[error("binding-table reference is the tombstone sentinel")]
    Tombstone,
    /// The ID was allocated by another request authority.
    #[error("binding-table reference belongs to another request")]
    ForeignRequest,
    /// The ID belongs to this request but has no registered table.
    #[error("binding-table reference is not registered in this request")]
    Unknown,
}

/// One request's sole binding-table ID allocator and lookup authority.
///
/// The high 32 ID bits identify the request authority and the low 32 bits are a
/// deterministic local sequence. Distinct registries therefore cannot both
/// allocate raw ID `1`, and an ID from an ended request cannot alias a table in
/// a later request.
pub struct BindingTableRegistry {
    authority: u32,
    state: Mutex<RegistryState>,
}

impl BindingTableRegistry {
    /// Create an empty registry whose first local allocation is non-tombstone.
    #[must_use]
    pub fn new() -> Self {
        let authority = NEXT_AUTHORITY
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                (value != u32::MAX).then_some(value + 1)
            })
            .expect("binding-table request authority exhausted");
        Self {
            authority,
            state: Mutex::new(RegistryState {
                tables: FxHashMap::default(),
                next_local_id: 1,
            }),
        }
    }

    /// Register `table` and return its request-scoped reference ID.
    pub fn register(&self, table: Arc<BindingTable>) -> BindingTableId {
        let mut state = self.state.lock();
        let local = state.next_local_id;
        state.next_local_id = local
            .checked_add(1)
            .expect("binding-table request-local ID counter exhausted");
        let id = BindingTableId::new((u64::from(self.authority) << 32) | u64::from(local));
        state.tables.insert(id, table);
        id
    }

    /// Resolve one ID, distinguishing tombstone, foreign, and unknown values.
    pub fn resolve(
        &self,
        id: BindingTableId,
    ) -> Result<Arc<BindingTable>, BindingTableLookupError> {
        if id == BindingTableId::TOMBSTONE {
            return Err(BindingTableLookupError::Tombstone);
        }
        let raw = id.get();
        if (raw >> 32) as u32 != self.authority {
            return Err(BindingTableLookupError::ForeignRequest);
        }
        self.state
            .lock()
            .tables
            .get(&id)
            .map(Arc::clone)
            .ok_or(BindingTableLookupError::Unknown)
    }

    /// Compatibility lookup returning `None` for every rejected reference.
    #[must_use]
    pub fn lookup(&self, id: BindingTableId) -> Option<Arc<BindingTable>> {
        self.resolve(id).ok()
    }

    /// Return the number of registered tables.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().tables.len()
    }

    /// Return whether no table is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for BindingTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BindingTableSchema;

    fn empty_table() -> Arc<BindingTable> {
        Arc::new(BindingTable::empty(BindingTableSchema {
            columns: Vec::new(),
        }))
    }

    #[test]
    fn same_request_round_trip_is_deterministic_and_non_tombstone() {
        let registry = BindingTableRegistry::new();
        let table = empty_table();
        let first = registry.register(Arc::clone(&table));
        let second = registry.register(empty_table());

        assert_ne!(first, BindingTableId::TOMBSTONE);
        assert_eq!(first.get() & u64::from(u32::MAX), 1);
        assert_eq!(second.get() & u64::from(u32::MAX), 2);
        assert!(Arc::ptr_eq(&registry.resolve(first).unwrap(), &table));
    }

    #[test]
    fn foreign_stale_tombstone_and_unknown_ids_fail_precisely() {
        let ended_request = BindingTableRegistry::new();
        let stale = ended_request.register(empty_table());
        let current_request = BindingTableRegistry::new();
        let current = current_request.register(empty_table());

        assert_ne!(stale, current);
        assert_eq!(
            current_request.resolve(stale),
            Err(BindingTableLookupError::ForeignRequest)
        );
        assert_eq!(
            current_request.resolve(BindingTableId::TOMBSTONE),
            Err(BindingTableLookupError::Tombstone)
        );
        let unknown = BindingTableId::new((current.get() & !u64::from(u32::MAX)) | 999);
        assert_eq!(
            current_request.resolve(unknown),
            Err(BindingTableLookupError::Unknown)
        );
    }

    #[test]
    fn dropping_request_registry_reclaims_registered_tables() {
        let table = empty_table();
        let weak = Arc::downgrade(&table);

        {
            let registry = BindingTableRegistry::new();
            registry.register(Arc::clone(&table));
            drop(table);
            assert!(weak.upgrade().is_some());
        }

        assert!(weak.upgrade().is_none());
    }
}
