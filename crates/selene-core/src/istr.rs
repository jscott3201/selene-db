//! Owned interned-style string handles backed by `compact_str::CompactString`.
//!
//! See spec 02 section 5.1. After the interner removal (stages A–C), `IStr` is
//! an owned `CompactString` newtype rather than a `lasso::Spur` handle into a
//! process-global pool. There is no longer a global pool, no distinct-string
//! cardinality cap, and no admission policy: [`intern`] simply constructs an
//! owned [`IStr`] after enforcing the per-string byte cap (`IL013`).
//!
//! The only construction guard is the `IL013` per-string byte limit
//! ([`MAX_INTERNED_STRING_BYTES`]); a string at or below it constructs an
//! [`IStr`], a longer one raises [`CoreError::StringTooLong`] (GQLSTATUS
//! `22G03`).

use std::fmt;

use compact_str::CompactString;
use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Place, Serialize as RkyvSerialize, SerializeUnsized,
    rancor::{Fallible, Source},
    string::{ArchivedString, StringResolver},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{CoreError, CoreResult};

/// Maximum byte length of a single interned string.
///
/// Per ISO Annex B `IL013` (2^32 - 1 bytes per inline string). A string at or
/// below this length may be interned; a longer one raises
/// [`CoreError::StringTooLong`] (GQLSTATUS `22G03`), mirroring the `IL015`
/// constructed-value cardinality enforcement in `PropertyMap`.
pub const MAX_INTERNED_STRING_BYTES: usize = u32::MAX as usize;

/// True when a string of `byte_len` bytes exceeds the `IL013` inline-string limit.
const fn string_cap_exceeded(byte_len: usize) -> bool {
    byte_len > MAX_INTERNED_STRING_BYTES
}

/// Reject strings whose byte length exceeds the `IL013` inline-string limit.
fn ensure_within_string_cap(s: &str) -> CoreResult<()> {
    if string_cap_exceeded(s.len()) {
        return Err(CoreError::StringTooLong {
            got: s.len(),
            max: u32::MAX,
        });
    }
    Ok(())
}

/// Owned interned-style string handle.
///
/// `IStr` is a `CompactString` newtype. It is owned and `'static` (no borrow),
/// so the multi-writer committer's `assert_send_static::<SealedCommit>()` proof
/// holds for free. `Clone` is a memcpy (≤24 bytes inline). Ordering is
/// **lexicographic** through the inner `CompactString` — so query-visible
/// comparisons and `BTreeMap`/`BTreeSet` iteration are content-ordered.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct IStr(CompactString);

/// Intern a string slice, returning an owned [`IStr`].
///
/// Construction is a plain `CompactString::from(s)` guarded only by the
/// `IL013` per-string byte cap; there is no global pool and no distinct-string
/// cardinality cap.
///
/// # Errors
///
/// Returns [`CoreError::StringTooLong`] if `s` exceeds
/// [`MAX_INTERNED_STRING_BYTES`] (IL013).
pub fn intern(s: &str) -> CoreResult<IStr> {
    ensure_within_string_cap(s)?;
    Ok(IStr(CompactString::from(s)))
}

/// Resolve an [`IStr`] to its string representation.
#[must_use]
pub fn resolve(istr: &IStr) -> &str {
    istr.0.as_str()
}

impl IStr {
    /// Resolve this handle to its owned string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
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
        // Why: archive bytes mirror `String`/`ArchivedString` exactly so the
        // newtype is wire-byte-identical to the pre-removal handle and
        // cold-start portable per spec 04 section 2 / D9.
        ArchivedString::serialize_from_str(self.as_str(), serializer)
    }
}

impl<D> RkyvDeserialize<IStr, D> for ArchivedString
where
    D: Fallible + ?Sized,
    D::Error: Source,
{
    fn deserialize(&self, _deserializer: &mut D) -> Result<IStr, D::Error> {
        // IL013 byte guard is retained on the decode path: an over-length
        // archived string raises StringTooLong (22G03) via `intern`.
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
        // Byte-identical to `String`: emit the string content via
        // `serialize_str`.
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IStr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // IL013 byte guard is retained on the decode path via `intern`.
        let value = String::deserialize(deserializer)?;
        intern(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_and_resolve_round_trip() {
        let key = intern("alpha").expect("interning succeeds");
        assert_eq!(resolve(&key), "alpha");
        assert_eq!(key.as_str(), "alpha");
        assert_eq!(key.to_string(), "alpha");
    }

    #[test]
    fn same_string_interns_to_equal_value() {
        assert_eq!(intern("same").unwrap(), intern("same").unwrap());
    }

    #[test]
    fn distinct_strings_intern_to_distinct_values() {
        assert_ne!(intern("left").unwrap(), intern("right").unwrap());
    }

    #[test]
    fn empty_and_unicode_strings_intern() {
        assert_eq!(intern("").unwrap().as_str(), "");
        assert_eq!(intern("\u{03bb} graph").unwrap().as_str(), "\u{03bb} graph");
    }

    #[test]
    fn istr_is_compactstring_sized() {
        // IStr wraps a CompactString (24 bytes inline).
        assert_eq!(std::mem::size_of::<IStr>(), 24);
    }

    #[test]
    fn istr_ord_is_lexicographic() {
        let aaa = intern("aaa").unwrap();
        let zzz = intern("zzz").unwrap();
        assert!(aaa < zzz);
        assert_eq!(aaa.cmp(&zzz), aaa.as_str().cmp(zzz.as_str()));
    }

    #[test]
    fn string_cap_boundary_is_il013_byte_limit() {
        // CORE-12: IL013 enforces 2^32 - 1 bytes per inline string. A 4 GiB
        // allocation is infeasible in a test, so exercise the length predicate
        // at the exact boundary.
        assert_eq!(MAX_INTERNED_STRING_BYTES, u32::MAX as usize);
        assert!(!string_cap_exceeded(MAX_INTERNED_STRING_BYTES));
        assert!(!string_cap_exceeded(MAX_INTERNED_STRING_BYTES - 1));
        assert!(string_cap_exceeded(MAX_INTERNED_STRING_BYTES + 1));
    }

    #[test]
    fn over_length_string_raises_string_too_long_with_22g03() {
        // CORE-12: the producer maps an over-length string to StringTooLong /
        // GQLSTATUS 22G03, mirroring IL015's ConstructedValueTooLarge.
        let err = ensure_within_string_cap_for_len(MAX_INTERNED_STRING_BYTES + 1)
            .expect_err("over-length string is rejected");
        assert!(matches!(
            err,
            CoreError::StringTooLong {
                max,
                ..
            } if max == u32::MAX
        ));
        assert_eq!(err.gqlstatus(), "22G03");
    }

    #[test]
    fn within_length_string_interns_normally() {
        // CORE-12: a sub-cap string still interns and round-trips.
        let key = format!("core-12-within-cap-{}", std::process::id());
        let interned = intern(&key).expect("within-cap string interns");
        assert_eq!(interned.as_str(), key);
    }

    /// Test-only shim exercising the byte-cap producer at a synthetic length
    /// without allocating the multi-gigabyte string the real boundary needs.
    fn ensure_within_string_cap_for_len(byte_len: usize) -> CoreResult<()> {
        if string_cap_exceeded(byte_len) {
            Err(CoreError::StringTooLong {
                got: byte_len,
                max: u32::MAX,
            })
        } else {
            Ok(())
        }
    }

    #[test]
    fn rkyv_archives_resolved_string() {
        // Wire-stability guard: the newtype archives its string content as an
        // ArchivedString, byte-identical to the pre-removal handle.
        let key = intern("istr.rkyv.portable").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let archived = rkyv::access::<rkyv::Archived<IStr>, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(archived.as_str(), "istr.rkyv.portable");
    }

    #[test]
    fn rkyv_round_trip_preserves_string() {
        // Wire-stability guard: round-trip through rkyv preserves content and
        // equality (CompactString content-Eq).
        let key = intern("istr.rkyv.reintern").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let decoded: IStr = rkyv::from_bytes::<IStr, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(decoded.as_str(), "istr.rkyv.reintern");
        assert_eq!(decoded, key);
    }
}
