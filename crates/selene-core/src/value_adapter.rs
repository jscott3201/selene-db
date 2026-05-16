//! Extension value-type adapter trait and process-wide registry.
//!
//! Per spec 02 section 3.2, extension-owned value types use `Value::Extended` with an
//! opaque `Arc<[u8]>` payload. Typed behavior is contributed by extension
//! crates via [`ValueTypeAdapter`] registrations at process start.
//!
//! Locking policy: this module uses `parking_lot::RwLock` with no poison
//! semantics, so a panic inside any adapter callback executed under the guard
//! cannot brick all subsequent `Value::Extended` evaluations. selene-core uses
//! parking_lot for `Mutex`/`RwLock` and std for `OnceLock`/atomics.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;

use crate::error::{CoreError, CoreResult};
use crate::extension_type_ids::ExtensionTypeId;

/// Behavior contract for an extension-owned value type.
pub trait ValueTypeAdapter: Send + Sync + 'static {
    /// Stable human-readable name, such as `selene-vector.vector`.
    fn name(&self) -> &'static str;

    /// Encode an extension-owned in-memory representation to payload bytes.
    fn encode(&self, value: &dyn Any) -> CoreResult<Arc<[u8]>>;

    /// Decode payload bytes to an extension-owned in-memory representation.
    fn decode(&self, payload: &Arc<[u8]>) -> CoreResult<Box<dyn Any>>;

    /// Compare two payloads for equality.
    fn equals(&self, lhs: &Arc<[u8]>, rhs: &Arc<[u8]>) -> CoreResult<bool>;

    /// Compare two payloads for ordering.
    fn compare(&self, lhs: &Arc<[u8]>, rhs: &Arc<[u8]>) -> CoreResult<Option<Ordering>>;

    /// Validate a payload against the extension-owned schema rules.
    fn schema_validate(&self, payload: &Arc<[u8]>) -> CoreResult<()>;

    /// Render a payload for diagnostics.
    fn display(&self, payload: &Arc<[u8]>, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

/// Process-wide registry of [`ValueTypeAdapter`] implementations.
pub struct ValueTypeRegistry {
    inner: RwLock<HashMap<ExtensionTypeId, Arc<dyn ValueTypeAdapter>>>,
}

impl ValueTypeRegistry {
    fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Look up the adapter registered for the given extension type ID.
    #[must_use]
    pub fn lookup(&self, type_id: ExtensionTypeId) -> Option<Arc<dyn ValueTypeAdapter>> {
        self.inner.read().get(&type_id).cloned()
    }
}

static REGISTRY: OnceLock<ValueTypeRegistry> = OnceLock::new();

/// Return the process-global value-type registry.
#[must_use]
pub fn value_type_registry() -> &'static ValueTypeRegistry {
    REGISTRY.get_or_init(ValueTypeRegistry::new)
}

/// Register an extension's adapter for the given type ID.
///
/// Re-registering the same `Arc` for the same ID is idempotent. Registering a
/// different adapter for an already claimed ID returns
/// [`CoreError::ExtensionTypeIdConflict`].
pub fn register_value_type(
    type_id: ExtensionTypeId,
    adapter: Arc<dyn ValueTypeAdapter>,
) -> CoreResult<()> {
    let registry = value_type_registry();
    let mut guard = registry.inner.write();
    if let Some(existing) = guard.get(&type_id) {
        if Arc::ptr_eq(existing, &adapter) {
            return Ok(());
        }
        return Err(CoreError::ExtensionTypeIdConflict { type_id });
    }
    guard.insert(type_id, adapter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::thread;

    use super::*;

    struct TestAdapter {
        name: &'static str,
    }

    impl ValueTypeAdapter for TestAdapter {
        fn name(&self) -> &'static str {
            self.name
        }

        fn encode(&self, _value: &dyn Any) -> CoreResult<Arc<[u8]>> {
            Ok(Arc::from([1_u8]))
        }

        fn decode(&self, payload: &Arc<[u8]>) -> CoreResult<Box<dyn Any>> {
            Ok(Box::new(payload.len()))
        }

        fn equals(&self, lhs: &Arc<[u8]>, rhs: &Arc<[u8]>) -> CoreResult<bool> {
            Ok(lhs == rhs)
        }

        fn compare(&self, lhs: &Arc<[u8]>, rhs: &Arc<[u8]>) -> CoreResult<Option<Ordering>> {
            Ok(Some(lhs.cmp(rhs)))
        }

        fn schema_validate(&self, _payload: &Arc<[u8]>) -> CoreResult<()> {
            Ok(())
        }

        fn display(&self, payload: &Arc<[u8]>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} bytes", payload.len())
        }
    }

    #[test]
    fn register_and_lookup_adapter() {
        let type_id = ExtensionTypeId(0x0001_1000);
        let adapter: Arc<dyn ValueTypeAdapter> = Arc::new(TestAdapter { name: "test.one" });
        register_value_type(type_id, adapter.clone()).unwrap();
        assert_eq!(
            value_type_registry().lookup(type_id).unwrap().name(),
            "test.one"
        );
        register_value_type(type_id, adapter).unwrap();
    }

    #[test]
    fn conflicting_adapter_returns_error() {
        let type_id = ExtensionTypeId(0x0001_1001);
        let first: Arc<dyn ValueTypeAdapter> = Arc::new(TestAdapter { name: "first" });
        let second: Arc<dyn ValueTypeAdapter> = Arc::new(TestAdapter { name: "second" });
        register_value_type(type_id, first).unwrap();
        let error = register_value_type(type_id, second).unwrap_err();
        assert!(matches!(error, CoreError::ExtensionTypeIdConflict { .. }));
    }

    #[test]
    fn registry_survives_panic_under_guard() {
        let registry = ValueTypeRegistry::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = registry.inner.write();
            panic!("simulated adapter panic");
        }));

        assert!(result.is_err());
        assert!(registry.lookup(ExtensionTypeId(0x0001_1100)).is_none());
    }

    #[test]
    fn concurrent_distinct_registrations_complete() {
        let handles: Vec<_> = (0..4)
            .map(|idx| {
                thread::spawn(move || {
                    let type_id = ExtensionTypeId(0x0001_2000 + idx);
                    let adapter: Arc<dyn ValueTypeAdapter> =
                        Arc::new(TestAdapter { name: "threaded" });
                    register_value_type(type_id, adapter)
                })
            })
            .collect();
        for handle in handles {
            assert!(handle.join().unwrap().is_ok());
        }
    }
}
