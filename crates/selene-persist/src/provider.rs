//! Recovery provider trait and registry.

use std::collections::BTreeMap;
use std::sync::Arc;

use selene_core::Change;

use crate::{PersistError, PersistResult};

/// Boxed dynamic error returned by recovery providers.
pub type RecoveryError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result alias used by the [`RecoveryProvider`] surface.
pub type RecoveryResult<T> = Result<T, RecoveryError>;

/// Recovery-time consumer of snapshot section bytes and WAL change events.
///
/// Implementations own any in-memory state they materialize from section bytes
/// and changes. The trait uses only raw tags and core change payloads so
/// `selene-persist` stays graph-blind.
pub trait RecoveryProvider: Send + Sync {
    /// Stable four-byte tag identifying this provider in snapshot section tables.
    fn provider_tag(&self) -> [u8; 4];

    /// Deliver the already-decompressed bytes of one snapshot section.
    ///
    /// # Errors
    ///
    /// Returns provider-owned errors when the section cannot be applied.
    fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()>;

    /// Deliver one committed mutation from WAL replay.
    ///
    /// # Errors
    ///
    /// Returns provider-owned errors when the change cannot be applied.
    fn on_change(&self, change: &Change) -> RecoveryResult<()>;

    /// Deliver one committed WAL entry as a batch.
    ///
    /// The default implementation preserves the historical per-change contract
    /// by calling [`Self::on_change`] for every change in entry order.
    /// Providers that need commit-level ordering can override this method.
    ///
    /// # Errors
    ///
    /// Returns provider-owned errors when any change in the batch cannot be
    /// applied.
    fn on_changes(&self, changes: &[Change]) -> RecoveryResult<()> {
        for change in changes {
            self.on_change(change)?;
        }
        Ok(())
    }
}

/// Recovery-time provider lookup table.
///
/// Providers are keyed by their four-byte `provider_tag`. The internal map is
/// ordered so WAL replay fans out changes deterministically.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<[u8; 4], Arc<dyn RecoveryProvider>>,
}

impl ProviderRegistry {
    /// Construct an empty provider registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            providers: BTreeMap::new(),
        }
    }

    /// Register a provider by its stable tag.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::DuplicateProviderTag`] if another provider with
    /// the same tag is already registered.
    pub fn register(&mut self, provider: Arc<dyn RecoveryProvider>) -> PersistResult<()> {
        let tag = provider.provider_tag();
        if self.providers.contains_key(&tag) {
            return Err(PersistError::DuplicateProviderTag { tag });
        }
        self.providers.insert(tag, provider);
        Ok(())
    }

    /// Look up a provider by tag.
    #[must_use]
    pub fn lookup(&self, provider_tag: [u8; 4]) -> Option<&Arc<dyn RecoveryProvider>> {
        self.providers.get(&provider_tag)
    }

    /// Return the number of registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Return true when no providers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Iterate registered providers in provider-tag ascending order.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn RecoveryProvider>> + '_ {
        self.providers.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyProvider {
        tag: [u8; 4],
    }

    impl RecoveryProvider for DummyProvider {
        fn provider_tag(&self) -> [u8; 4] {
            self.tag
        }

        fn read_section(&self, _sub: [u8; 4], _bytes: &[u8]) -> RecoveryResult<()> {
            Ok(())
        }

        fn on_change(&self, _change: &Change) -> RecoveryResult<()> {
            Ok(())
        }
    }

    fn provider(tag: [u8; 4]) -> Arc<dyn RecoveryProvider> {
        Arc::new(DummyProvider { tag })
    }

    #[test]
    fn register_unique_tag_succeeds() {
        let mut registry = ProviderRegistry::new();
        registry.register(provider(*b"CORE")).unwrap();
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn register_duplicate_tag_is_rejected() {
        let mut registry = ProviderRegistry::new();
        registry.register(provider(*b"CORE")).unwrap();
        assert!(matches!(
            registry.register(provider(*b"CORE")),
            Err(PersistError::DuplicateProviderTag { tag }) if tag == *b"CORE"
        ));
    }

    #[test]
    fn lookup_returns_registered_provider() {
        let mut registry = ProviderRegistry::new();
        registry.register(provider(*b"CORE")).unwrap();
        assert_eq!(registry.lookup(*b"CORE").unwrap().provider_tag(), *b"CORE");
        assert!(registry.lookup(*b"MISS").is_none());
    }

    #[test]
    fn iter_yields_in_provider_tag_order() {
        let mut registry = ProviderRegistry::new();
        registry.register(provider(*b"VECT")).unwrap();
        registry.register(provider(*b"CORE")).unwrap();
        registry.register(provider(*b"META")).unwrap();
        let tags: Vec<_> = registry
            .iter()
            .map(|provider| provider.provider_tag())
            .collect();
        assert_eq!(tags, vec![*b"CORE", *b"META", *b"VECT"]);
    }
}
