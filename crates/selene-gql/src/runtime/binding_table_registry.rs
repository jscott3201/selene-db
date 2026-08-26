//! Request-scoped binding-table registry.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use selene_core::BindingTableId;

use crate::runtime::BindingTable;

static NEXT_AUTHORITY: AtomicU64 = AtomicU64::new(1);

enum AuthoritySource {
    Global,
    #[cfg(test)]
    Injected(Arc<AtomicU64>),
}

impl AuthoritySource {
    fn allocate(&self) -> Result<u32, BindingTableAllocationError> {
        let counter = match self {
            Self::Global => &NEXT_AUTHORITY,
            #[cfg(test)]
            Self::Injected(counter) => counter,
        };
        let authority = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next >= 1 && next <= u64::from(u32::MAX)).then_some(next + 1)
            })
            .map_err(|_| BindingTableAllocationError::AuthorityExhausted)?;
        u32::try_from(authority).map_err(|_| BindingTableAllocationError::AuthorityExhausted)
    }
}

struct RegistryState {
    tables: FxHashMap<BindingTableId, Arc<BindingTable>>,
    authority: Option<u32>,
    next_local_id: u64,
}

impl RegistryState {
    fn new(next_local_id: u64) -> Self {
        Self {
            tables: FxHashMap::default(),
            authority: None,
            next_local_id,
        }
    }
}

/// Failure to allocate a finite request-scoped binding-table ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum BindingTableAllocationError {
    /// The process has assigned every non-reusable request authority.
    #[error("binding-table request authority exhausted")]
    AuthorityExhausted,
    /// One request has assigned every non-tombstone local table ID.
    #[error("binding-table request-local ID space exhausted")]
    RequestLocalExhausted,
}

impl BindingTableAllocationError {
    pub(crate) const fn program_limit_detail(self) -> &'static str {
        match self {
            Self::AuthorityExhausted => "binding-table request authority exhausted",
            Self::RequestLocalExhausted => "binding-table request-local ID space exhausted",
        }
    }
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
    authority_source: AuthoritySource,
    state: Mutex<RegistryState>,
}

impl BindingTableRegistry {
    /// Create an empty registry that acquires no authority until its first table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            authority_source: AuthoritySource::Global,
            state: Mutex::new(RegistryState::new(1)),
        }
    }

    #[cfg(test)]
    fn with_allocator(allocator: Arc<AtomicU64>, next_local_id: u64) -> Self {
        Self {
            authority_source: AuthoritySource::Injected(allocator),
            state: Mutex::new(RegistryState::new(next_local_id)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limits_for_test(next_authority: u64, next_local_id: u64) -> Self {
        Self::with_allocator(Arc::new(AtomicU64::new(next_authority)), next_local_id)
    }

    /// Register `table` and return its request-scoped reference ID.
    ///
    /// # Errors
    ///
    /// Returns a controlled allocation error after either finite ID component is
    /// exhausted. IDs never wrap or reuse an ended request's authority.
    pub fn register(
        &self,
        table: Arc<BindingTable>,
    ) -> Result<BindingTableId, BindingTableAllocationError> {
        let mut state = self.state.lock();
        let local = u32::try_from(state.next_local_id)
            .ok()
            .filter(|local| *local != 0)
            .ok_or(BindingTableAllocationError::RequestLocalExhausted)?;
        let authority = match state.authority {
            Some(authority) => authority,
            None => {
                let authority = self.authority_source.allocate()?;
                state.authority = Some(authority);
                authority
            }
        };
        let id = BindingTableId::new((u64::from(authority) << 32) | u64::from(local));
        state.next_local_id += 1;
        state.tables.insert(id, table);
        Ok(id)
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
        let state = self.state.lock();
        if state.authority != Some((raw >> 32) as u32) {
            return Err(BindingTableLookupError::ForeignRequest);
        }
        state
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
        let first = registry.register(Arc::clone(&table)).unwrap();
        let second = registry.register(empty_table()).unwrap();

        assert_ne!(first, BindingTableId::TOMBSTONE);
        assert_eq!(first.get() & u64::from(u32::MAX), 1);
        assert_eq!(second.get() & u64::from(u32::MAX), 2);
        assert!(Arc::ptr_eq(&registry.resolve(first).unwrap(), &table));
    }

    #[test]
    fn foreign_stale_tombstone_and_unknown_ids_fail_precisely() {
        let ended_request = BindingTableRegistry::new();
        let stale = ended_request.register(empty_table()).unwrap();
        let current_request = BindingTableRegistry::new();
        let current = current_request.register(empty_table()).unwrap();

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
            registry.register(Arc::clone(&table)).unwrap();
            drop(table);
            assert!(weak.upgrade().is_some());
        }

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn tableless_and_rejected_requests_do_not_consume_an_authority() {
        let allocator = Arc::new(AtomicU64::new(17));
        let tableless = BindingTableRegistry::with_allocator(Arc::clone(&allocator), 1);
        let rejected = BindingTableRegistry::with_allocator(Arc::clone(&allocator), 1);

        assert!(tableless.is_empty());
        assert_eq!(
            rejected.resolve(BindingTableId::new(1)),
            Err(BindingTableLookupError::ForeignRequest)
        );
        assert_eq!(allocator.load(Ordering::Relaxed), 17);

        let id = tableless.register(empty_table()).unwrap();
        assert_eq!(id.get() >> 32, 17);
        assert_eq!(allocator.load(Ordering::Relaxed), 18);
    }

    #[test]
    fn authority_exhaustion_is_controlled_and_never_reuses_a_stale_id() {
        let allocator = Arc::new(AtomicU64::new(u64::from(u32::MAX)));
        let ended_request = BindingTableRegistry::with_allocator(Arc::clone(&allocator), 1);
        let stale = ended_request.register(empty_table()).unwrap();
        let exhausted_request = BindingTableRegistry::with_allocator(allocator, 1);
        let table = empty_table();
        let weak = Arc::downgrade(&table);

        assert_eq!(
            exhausted_request.register(table),
            Err(BindingTableAllocationError::AuthorityExhausted)
        );
        assert!(
            weak.upgrade().is_none(),
            "failed table must not be retained"
        );
        assert!(exhausted_request.is_empty());
        assert_eq!(
            exhausted_request.resolve(stale),
            Err(BindingTableLookupError::ForeignRequest)
        );
        assert_eq!(ended_request.resolve(stale).unwrap().row_count(), 0);
    }

    #[test]
    fn request_local_exhaustion_is_controlled_without_wrap_or_registry_leak() {
        let allocator = Arc::new(AtomicU64::new(23));
        let registry = BindingTableRegistry::with_allocator(allocator, u64::from(u32::MAX));
        let last = registry.register(empty_table()).unwrap();
        let table = empty_table();
        let weak = Arc::downgrade(&table);

        assert_eq!(last.get() & u64::from(u32::MAX), u64::from(u32::MAX));
        assert_eq!(
            registry.register(table),
            Err(BindingTableAllocationError::RequestLocalExhausted)
        );
        assert!(
            weak.upgrade().is_none(),
            "failed table must not be retained"
        );
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.resolve(last).unwrap().row_count(), 0);
        let wrapped = BindingTableId::new(last.get() & !u64::from(u32::MAX) | 1);
        assert_eq!(
            registry.resolve(wrapped),
            Err(BindingTableLookupError::Unknown)
        );
    }
}
