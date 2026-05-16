//! Interned string handles backed by a process-global lasso interner.
//!
//! See spec 02 section 5.1. The cap of 1,000,000 distinct strings protects against
//! unbounded interner growth; exceeding the cap raises
//! [`CoreError::IStrCapExceeded`](crate::CoreError::IStrCapExceeded), mapped to
//! GQLSTATUS `54000`.

use std::fmt;
use std::sync::OnceLock;

use lasso::{Spur, ThreadedRodeo};
use parking_lot::Mutex;
use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Place, Serialize as RkyvSerialize, SerializeUnsized,
    rancor::{Fallible, Source},
    string::{ArchivedString, StringResolver},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

/// Admission lock for cap-check + insert atomicity.
///
/// Held only on the slow path for strings that are not yet interned.
/// Already-interned strings hit the lock-free fast path via `rodeo.get(s)`.
/// Without this lock, concurrent callers could both observe capacity and insert
/// distinct strings, breaking the spec 02 section 5.1 GQLSTATUS 54000 contract.
///
/// Uses `parking_lot::Mutex` with no poison semantics so a panic in the
/// admission predicate path cannot brick all future intern calls.
static ADMISSION_LOCK: Mutex<()> = Mutex::new(());

fn interner() -> &'static ThreadedRodeo<Spur> {
    INTERNER.get_or_init(ThreadedRodeo::new)
}

const fn cap_exceeded(current_len: usize) -> bool {
    current_len >= MAX_INTERNED_STRINGS
}

/// Intern a string slice, returning a stable [`IStr`] handle.
///
/// If the string is already interned, this returns the existing handle from a
/// lock-free fast path. Otherwise, the admission lock serializes the second
/// lookup, cap check, and insert so [`MAX_INTERNED_STRINGS`] remains a hard
/// process cap under concurrency.
///
/// # Errors
///
/// Returns [`CoreError::IStrCapExceeded`] if the interner is at cap and the
/// string is not already present.
pub fn intern(s: &str) -> CoreResult<IStr> {
    intern_with_admission(s).map(|(value, _was_new)| value)
}

/// Intern a string slice and report whether it was newly admitted.
///
/// This has the same cap and concurrency behavior as [`intern`], but returns
/// `true` in the second tuple slot only when the call inserted a new string
/// into the process-global interner. Parser-side DoS guards use that signal to
/// enforce per-request admission budgets without charging repeated names.
///
/// # Errors
///
/// Returns [`CoreError::IStrCapExceeded`] if the interner is at cap and the
/// string is not already present.
pub fn intern_with_admission(s: &str) -> CoreResult<(IStr, bool)> {
    let rodeo = interner();

    // Fast path: already-interned strings do not need admission.
    if let Some(spur) = rodeo.get(s) {
        return Ok((IStr(spur), false));
    }

    // Slow path: serialize admission so cap-check + insert are atomic.
    let _admission = ADMISSION_LOCK.lock();

    // Re-check inside the lock; another thread may have interned `s` between
    // the fast-path miss and lock acquisition.
    if let Some(spur) = rodeo.get(s) {
        return Ok((IStr(spur), false));
    }

    let count = rodeo.len();
    if cap_exceeded(count) {
        return Err(CoreError::IStrCapExceeded {
            count,
            max: MAX_INTERNED_STRINGS,
        });
    }
    Ok((IStr(rodeo.get_or_intern(s)), true))
}

/// Resolve an [`IStr`] to its process-lifetime string representation.
#[must_use]
pub fn resolve(istr: IStr) -> &'static str {
    interner().resolve(&istr.0)
}

/// Look up an existing interned handle without admitting a new one.
///
/// Returns the interned handle if `s` is already present and `None` otherwise.
#[must_use]
pub fn lookup(s: &str) -> Option<IStr> {
    interner().get(s).map(IStr)
}

/// Outcome of an atomic admission attempt via [`intern_atomic_admit`].
#[derive(Debug)]
#[non_exhaustive]
pub enum AdmissionError {
    /// The global process-wide interner is at [`MAX_INTERNED_STRINGS`].
    Cap(CoreError),
    /// The caller-supplied admission predicate rejected this admission.
    ///
    /// Only raised when `s` would be a *new* admission and the caller's
    /// predicate returned `Err`. Already-interned strings never reach the
    /// predicate.
    Rejected,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cap(error) => fmt::Display::fmt(error, f),
            Self::Rejected => f.write_str("admission rejected by caller predicate"),
        }
    }
}

impl std::error::Error for AdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cap(error) => Some(error),
            Self::Rejected => None,
        }
    }
}

/// Intern `s`, but route every *new* admission through a caller predicate.
///
/// Already-interned strings hit the lock-free fast path; the predicate is not
/// called. New admissions take the admission lock, then the predicate is run
/// **under** the same lock that gates the global insertion. The predicate's
/// `Ok`/`Err` decision is therefore atomic with the global cap check and
/// insertion: a rejected admission cannot grow the global pool, and the
/// predicate is never invoked redundantly because of a race with another
/// thread admitting the same string.
///
/// Use this from parser-side admission budgets that must charge a slot
/// before — and only when — the admission is genuinely new from this
/// thread's vantage. Compare with [`intern_with_admission`], which only
/// reports `was_new` after the global insertion has already happened.
///
/// # Errors
///
/// - [`AdmissionError::Cap`] if the global pool is at capacity.
/// - [`AdmissionError::Rejected`] if the caller's predicate returned `Err`
///   for what would have been a new admission.
pub fn intern_atomic_admit<F>(s: &str, on_new: F) -> Result<IStr, AdmissionError>
where
    F: FnOnce() -> Result<(), ()>,
{
    let rodeo = interner();

    // Fast path: already-interned strings do not need admission.
    if let Some(spur) = rodeo.get(s) {
        return Ok(IStr(spur));
    }

    // Slow path: serialize admission with the cap check and the predicate
    // so callers see a single atomic accept-or-reject.
    let _admission = ADMISSION_LOCK.lock();

    // Re-check inside the lock: another thread may have interned `s`
    // between the fast-path miss and the lock acquisition. If so, the
    // caller predicate is NOT called — admission was not consumed by this
    // call.
    if let Some(spur) = rodeo.get(s) {
        return Ok(IStr(spur));
    }

    // This call would be a genuine new admission. Cap check first so a
    // global-cap failure never perturbs caller state via the predicate.
    let count = rodeo.len();
    if cap_exceeded(count) {
        return Err(AdmissionError::Cap(CoreError::IStrCapExceeded {
            count,
            max: MAX_INTERNED_STRINGS,
        }));
    }

    // Predicate runs under the lock; if it accepts, the global insertion
    // happens immediately and the lock is released; if it rejects, no
    // global insertion occurs at all.
    if on_new().is_err() {
        return Err(AdmissionError::Rejected);
    }

    Ok(IStr(rodeo.get_or_intern(s)))
}

impl IStr {
    /// Resolve this handle to its process-lifetime string representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        resolve(self)
    }
}

impl Archive for IStr {
    type Archived = ArchivedString;
    type Resolver = StringResolver;

    fn resolve(&self, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedString::resolve_from_str(self.as_str(), resolver, out);
    }
}

impl<S> RkyvSerialize<S> for IStr
where
    S: Fallible + ?Sized,
    S::Error: Source,
    str: SerializeUnsized<S>,
{
    fn serialize(&self, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        // Why: interner keys are process-local; archive bytes ensure
        // cold-start portability per spec 04 section 2 / D9.
        ArchivedString::serialize_from_str(self.as_str(), serializer)
    }
}

impl<D> RkyvDeserialize<IStr, D> for ArchivedString
where
    D: Fallible + ?Sized,
    D::Error: Source,
{
    fn deserialize(&self, _deserializer: &mut D) -> Result<IStr, D::Error> {
        match intern(self.as_str()) {
            Ok(value) => Ok(value),
            Err(error) => {
                rkyv::rancor::fail!(error);
            }
        }
    }
}

impl fmt::Display for IStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for IStr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IStr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        intern(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::panic::{AssertUnwindSafe, catch_unwind};
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
    fn admission_lock_survives_panic() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _admission = ADMISSION_LOCK.lock();
            panic!("simulated admission panic");
        }));

        assert!(result.is_err());
        let key = format!("brief-98-post-panic-string-{}", std::process::id());
        let _ = intern(&key).expect("intern after panic");
    }

    #[test]
    fn cap_is_monotonic_under_concurrent_admission() {
        let prefix = format!("brief-05.1-mono-{}", std::process::id());
        let n_threads = 16;
        let strings_per_thread = 64;

        let handles: Vec<Vec<IStr>> = thread::scope(|scope| {
            let mut joiners = Vec::new();
            for t in 0..n_threads {
                let prefix = prefix.clone();
                joiners.push(scope.spawn(move || {
                    let mut keys = Vec::new();
                    for i in 0..strings_per_thread {
                        let key = format!("{prefix}-t{t}-{i}");
                        keys.push(intern(&key).expect("under cap"));
                    }
                    keys
                }));
            }
            joiners
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        });

        let admitted: HashSet<IStr> = handles.into_iter().flatten().collect();
        assert_eq!(admitted.len(), n_threads * strings_per_thread);
        assert!(interner().len() <= MAX_INTERNED_STRINGS);
    }

    #[test]
    fn same_string_race_returns_identical_handle() {
        let key = format!("brief-05.1-same-{}", std::process::id());
        let n_threads = 32;

        let handles: Vec<IStr> = thread::scope(|scope| {
            let mut joiners = Vec::new();
            for _ in 0..n_threads {
                let key = key.clone();
                joiners
                    .push(scope.spawn(move || intern(&key).expect("contended duplicate intern")));
            }
            joiners
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        });

        let first = handles[0];
        for handle in &handles[1..] {
            assert_eq!(
                *handle, first,
                "concurrent intern of same string must return same handle"
            );
        }
    }

    #[test]
    fn cap_error_carries_current_count_and_max() {
        let err = CoreError::IStrCapExceeded {
            count: MAX_INTERNED_STRINGS,
            max: MAX_INTERNED_STRINGS,
        };
        assert_eq!(err.gqlstatus(), "54000");
        assert!(err.to_string().contains(&MAX_INTERNED_STRINGS.to_string()));
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

    #[test]
    fn rkyv_archives_resolved_string_not_interner_key() {
        let key = intern("istr.rkyv.portable").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let archived = rkyv::access::<rkyv::Archived<IStr>, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(archived.as_str(), "istr.rkyv.portable");
    }

    #[test]
    fn intern_atomic_admit_skips_predicate_for_existing_strings() {
        let key = format!("brief-19.atomic.exists-{}", std::process::id());
        let _admitted = intern(&key).expect("seed admission");

        let mut predicate_calls = 0;
        let result = intern_atomic_admit(&key, || {
            predicate_calls += 1;
            Ok(())
        });
        assert!(result.is_ok(), "already-interned string returns Ok");
        assert_eq!(
            predicate_calls, 0,
            "predicate must not run for already-interned strings"
        );
    }

    #[test]
    fn intern_atomic_admit_rejects_without_global_growth() {
        let key = format!("brief-19.atomic.reject-{}", std::process::id());
        assert!(lookup(&key).is_none(), "test setup: key not yet interned");

        let result = intern_atomic_admit(&key, || Err(()));
        assert!(matches!(result, Err(AdmissionError::Rejected)));
        assert!(
            lookup(&key).is_none(),
            "rejected admission must not have grown the global pool"
        );
    }

    #[test]
    fn intern_atomic_admit_accepts_and_admits() {
        let key = format!("brief-19.atomic.accept-{}", std::process::id());
        assert!(lookup(&key).is_none());

        let mut admit_count = 0;
        let result = intern_atomic_admit(&key, || {
            admit_count += 1;
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(
            admit_count, 1,
            "predicate runs exactly once on new admission"
        );
        assert!(lookup(&key).is_some(), "accepted admission grew the pool");
    }

    #[test]
    fn lookup_returns_some_for_already_interned_strings() {
        let key = format!("brief-19.lookup-some-{}", std::process::id());
        let admitted = intern(&key).expect("intern succeeds");
        assert_eq!(lookup(&key), Some(admitted));
    }

    #[test]
    fn lookup_returns_none_for_unseen_string_without_admitting() {
        let key = format!("brief-19.lookup-none-{}", std::process::id());
        assert!(lookup(&key).is_none());
        // The lookup must not have admitted the string.
        assert!(lookup(&key).is_none());
    }

    #[test]
    fn rkyv_round_trip_reinterns_string() {
        let key = intern("istr.rkyv.reintern").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let decoded: IStr = rkyv::from_bytes::<IStr, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(decoded.as_str(), "istr.rkyv.reintern");
        assert_eq!(decoded, key);
    }
}
