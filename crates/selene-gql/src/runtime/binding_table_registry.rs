//! Request-scoped binding-table registry.

use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use rustc_hash::FxHashMap;
use selene_core::BindingTableId;

use crate::runtime::BindingTable;

/// Per-statement registry for binding-table reference values.
///
/// The registry owns request-scoped [`BindingTableId`] allocations and maps
/// those IDs back to the tables they reference. A fresh registry is created for
/// each statement so IDs never bleed across statement boundaries.
pub struct BindingTableRegistry {
    tables: RefCell<FxHashMap<BindingTableId, Arc<BindingTable>>>,
    next_id: Cell<u64>,
}

impl BindingTableRegistry {
    /// Create an empty registry whose first allocated ID is non-tombstone.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tables: RefCell::default(),
            next_id: Cell::new(1),
        }
    }

    /// Register `table` and return its request-scoped reference ID.
    pub fn register(&self, table: Arc<BindingTable>) -> BindingTableId {
        let raw = self.next_id.get();
        self.next_id.set(
            raw.checked_add(1)
                .expect("binding table ID counter exhausted"),
        );
        let id = BindingTableId::new(raw);
        self.tables.borrow_mut().insert(id, table);
        id
    }

    /// Look up a registered table by request-scoped reference ID.
    #[must_use]
    pub fn lookup(&self, id: BindingTableId) -> Option<Arc<BindingTable>> {
        self.tables.borrow().get(&id).map(Arc::clone)
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
    fn first_allocated_id_is_not_tombstone() {
        let registry = BindingTableRegistry::new();
        let id = registry.register(empty_table());

        assert_ne!(id, BindingTableId::TOMBSTONE);
    }

    #[test]
    fn register_lookup_round_trips_tables() {
        let registry = BindingTableRegistry::new();
        let table = empty_table();
        let id = registry.register(Arc::clone(&table));

        let found = registry.lookup(id).expect("registered table found");
        assert!(Arc::ptr_eq(&found, &table));
        assert!(registry.lookup(BindingTableId::new(999)).is_none());
    }

    #[test]
    fn ids_are_monotonic_from_one() {
        let registry = BindingTableRegistry::new();

        let first = registry.register(empty_table());
        let second = registry.register(empty_table());

        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
    }
}
