//! Interned string handles backed by a process-global lasso interner.
//!
//! See spec 02 section 5.1. The cap of 1,000,000 distinct strings protects against
//! unbounded interner growth; exceeding the cap raises
//! [`CoreError::IStrCapExceeded`](crate::CoreError::IStrCapExceeded), mapped to
//! GQLSTATUS `54000`.

use std::fmt;
use std::sync::OnceLock;

use lasso::{Spur, ThreadedRodeo};

use crate::error::{CoreError, CoreResult};

/// Maximum number of distinct interned strings per process.
///
/// This is the spec 02 section 5.1 DoS guard for the `IL013` family of implementation
/// choices.
pub const MAX_INTERNED_STRINGS: usize = 1_000_000;

/// Interned string handle.
///
/// `IStr` is `Copy` and 32-bit sized via lasso's [`Spur`] key. Ordering is
/// interner-key order, not lexicographic order. Resolve to `&str` for
/// lexicographic comparisons at query-evaluation time.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct IStr(Spur);

static INTERNER: OnceLock<ThreadedRodeo<Spur>> = OnceLock::new();

fn interner() -> &'static ThreadedRodeo<Spur> {
    INTERNER.get_or_init(ThreadedRodeo::new)
}

const fn cap_exceeded(current_len: usize) -> bool {
    current_len >= MAX_INTERNED_STRINGS
}

/// Intern a string slice, returning a stable [`IStr`] handle.
///
/// If the string is already interned, this returns the existing handle. If
/// interning a new string would exceed [`MAX_INTERNED_STRINGS`], this returns
/// [`CoreError::IStrCapExceeded`].
pub fn intern(s: &str) -> CoreResult<IStr> {
    let rodeo = interner();
    if let Some(spur) = rodeo.get(s) {
        return Ok(IStr(spur));
    }
    if cap_exceeded(rodeo.len()) {
        return Err(CoreError::IStrCapExceeded {
            count: rodeo.len(),
            max: MAX_INTERNED_STRINGS,
        });
    }
    Ok(IStr(rodeo.get_or_intern(s)))
}

/// Resolve an [`IStr`] to its process-lifetime string representation.
#[must_use]
pub fn resolve(istr: IStr) -> &'static str {
    interner().resolve(&istr.0)
}

impl IStr {
    /// Resolve this handle to its process-lifetime string representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        resolve(self)
    }
}

impl fmt::Display for IStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn intern_and_resolve_round_trip() {
        let key = intern("alpha").expect("interning succeeds");
        assert_eq!(resolve(key), "alpha");
        assert_eq!(key.as_str(), "alpha");
        assert_eq!(key.to_string(), "alpha");
    }

    #[test]
    fn same_string_interns_to_same_key() {
        assert_eq!(intern("same").unwrap(), intern("same").unwrap());
    }

    #[test]
    fn distinct_strings_intern_to_distinct_keys() {
        assert_ne!(intern("left").unwrap(), intern("right").unwrap());
    }

    #[test]
    fn empty_and_unicode_strings_intern() {
        assert_eq!(intern("").unwrap().as_str(), "");
        assert_eq!(intern("\u{03bb} graph").unwrap().as_str(), "\u{03bb} graph");
    }

    #[test]
    fn istr_is_32_bit_sized() {
        assert_eq!(std::mem::size_of::<IStr>(), 4);
    }

    #[test]
    fn cap_check_boundary_is_tested_without_filling_global_interner() {
        assert!(!cap_exceeded(MAX_INTERNED_STRINGS - 1));
        assert!(cap_exceeded(MAX_INTERNED_STRINGS));
        assert_eq!(MAX_INTERNED_STRINGS, 1_000_000);
    }

    #[test]
    fn concurrent_interning_is_thread_safe() {
        let handles: Vec<_> = (0..8)
            .map(|idx| thread::spawn(move || intern(&format!("threaded-{idx}")).unwrap()))
            .collect();
        for handle in handles {
            assert!(handle.join().is_ok());
        }
    }
}
