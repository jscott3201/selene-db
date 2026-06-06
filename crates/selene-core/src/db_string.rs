//! Engine-owned database strings backed by standard [`String`] storage.
//!
//! `DbString` is an owned string newtype used for GQL string values, graph
//! labels, property keys, aliases, and procedure-name segments. There is no
//! process-global string pool, specialized small-string storage, or
//! distinct-string cardinality cap: [`db_string`] simply constructs an owned
//! [`DbString`] after enforcing the per-string byte cap (`IL013`).
//!
//! The only construction guard is the `IL013` per-string byte limit
//! ([`MAX_DB_STRING_BYTES`]); a string at or below it constructs an
//! [`DbString`], a longer one raises [`CoreError::StringTooLong`] (GQLSTATUS
//! `22G03`).

use std::fmt;

use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Place, Serialize as RkyvSerialize, SerializeUnsized,
    rancor::{Fallible, Source},
    string::{ArchivedString, StringResolver},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{CoreError, CoreResult};

/// Maximum byte length of a single database string.
///
/// Per ISO Annex B `IL013` (2^32 - 1 bytes per inline string). A string at or
/// below this length may be constructed; a longer one raises
/// [`CoreError::StringTooLong`] (GQLSTATUS `22G03`), mirroring the `IL015`
/// constructed-value cardinality enforcement in `PropertyMap`.
pub const MAX_DB_STRING_BYTES: usize = u32::MAX as usize;

/// True when a string of `byte_len` bytes exceeds the `IL013` inline-string limit.
const fn string_cap_exceeded(byte_len: usize) -> bool {
    byte_len > MAX_DB_STRING_BYTES
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

/// Owned database string.
///
/// `DbString` is a standard [`String`] newtype. It is owned and `'static` (no
/// borrow), so the multi-writer committer's
/// `assert_send_static::<SealedCommit>()` proof holds for free. Ordering is
/// **lexicographic** through the inner string, so query-visible comparisons and
/// `BTreeMap`/`BTreeSet` iteration are content-ordered.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct DbString(String);

/// Construct an owned [`DbString`] from a string slice.
///
/// Construction is a plain [`String`] allocation guarded only by the `IL013`
/// per-string byte cap; there is no global pool, specialized small-string
/// storage, or distinct-string cardinality cap.
///
/// # Errors
///
/// Returns [`CoreError::StringTooLong`] if `s` exceeds
/// [`MAX_DB_STRING_BYTES`] (IL013).
pub fn db_string(s: &str) -> CoreResult<DbString> {
    ensure_within_string_cap(s)?;
    Ok(DbString(s.to_owned()))
}

impl DbString {
    /// Return this database string as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Archive for DbString {
    type Archived = ArchivedString;
    type Resolver = StringResolver;

    fn resolve(&self, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedString::resolve_from_str(self.as_str(), resolver, out);
    }
}

impl<S> RkyvSerialize<S> for DbString
where
    S: Fallible + ?Sized,
    S::Error: Source,
    str: SerializeUnsized<S>,
{
    fn serialize(&self, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        // Why: archive bytes mirror `String`/`ArchivedString` exactly so
        // snapshots stay content-addressable and cold-start portable per spec
        // 04 section 2 / D9.
        ArchivedString::serialize_from_str(self.as_str(), serializer)
    }
}

impl<D> RkyvDeserialize<DbString, D> for ArchivedString
where
    D: Fallible + ?Sized,
    D::Error: Source,
{
    fn deserialize(&self, _deserializer: &mut D) -> Result<DbString, D::Error> {
        // IL013 byte guard is retained on the decode path: an over-length
        // archived string raises StringTooLong (22G03) via `db_string`.
        match db_string(self.as_str()) {
            Ok(value) => Ok(value),
            Err(error) => {
                rkyv::rancor::fail!(error);
            }
        }
    }
}

impl fmt::Display for DbString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for DbString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Byte-identical to `String`: emit the string content via
        // `serialize_str`.
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DbString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // IL013 byte guard is retained on the decode path via `db_string`.
        let value = String::deserialize(deserializer)?;
        db_string(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_string_round_trip() {
        let key = db_string("alpha").expect("DB string construction succeeds");
        assert_eq!(key.as_str(), "alpha");
        assert_eq!(key.to_string(), "alpha");
    }

    #[test]
    fn same_string_constructs_equal_value() {
        assert_eq!(db_string("same").unwrap(), db_string("same").unwrap());
    }

    #[test]
    fn distinct_strings_construct_distinct_values() {
        assert_ne!(db_string("left").unwrap(), db_string("right").unwrap());
    }

    #[test]
    fn empty_and_unicode_strings_construct() {
        assert_eq!(db_string("").unwrap().as_str(), "");
        assert_eq!(
            db_string("\u{03bb} graph").unwrap().as_str(),
            "\u{03bb} graph"
        );
    }

    #[test]
    fn db_string_is_string_sized() {
        // DbString wraps a standard String.
        assert_eq!(std::mem::size_of::<DbString>(), 24);
    }

    #[test]
    fn db_string_ord_is_lexicographic() {
        let aaa = db_string("aaa").unwrap();
        let zzz = db_string("zzz").unwrap();
        assert!(aaa < zzz);
        assert_eq!(aaa.cmp(&zzz), aaa.as_str().cmp(zzz.as_str()));
    }

    #[test]
    fn string_cap_boundary_is_il013_byte_limit() {
        // CORE-12: IL013 enforces 2^32 - 1 bytes per inline string. A 4 GiB
        // allocation is infeasible in a test, so exercise the length predicate
        // at the exact boundary.
        assert_eq!(MAX_DB_STRING_BYTES, u32::MAX as usize);
        assert!(!string_cap_exceeded(MAX_DB_STRING_BYTES));
        assert!(!string_cap_exceeded(MAX_DB_STRING_BYTES - 1));
        assert!(string_cap_exceeded(MAX_DB_STRING_BYTES + 1));
    }

    #[test]
    fn over_length_string_raises_string_too_long_with_22g03() {
        // CORE-12: the producer maps an over-length string to StringTooLong /
        // GQLSTATUS 22G03, mirroring IL015's ConstructedValueTooLarge.
        let err = ensure_within_string_cap_for_len(MAX_DB_STRING_BYTES + 1)
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
    fn within_length_string_constructs_normally() {
        // CORE-12: a sub-cap string still constructs and round-trips.
        let key = format!("core-12-within-cap-{}", std::process::id());
        let value = db_string(&key).expect("within-cap string fits DB string cap");
        assert_eq!(value.as_str(), key);
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
        // ArchivedString.
        let key = db_string("db_string.rkyv.portable").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let archived =
            rkyv::access::<rkyv::Archived<DbString>, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(archived.as_str(), "db_string.rkyv.portable");
    }

    #[test]
    fn rkyv_round_trip_preserves_string() {
        // Wire-stability guard: round-trip through rkyv preserves content and
        // equality.
        let key = db_string("db_string.rkyv.round_trip").unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key).unwrap();
        let decoded: DbString = rkyv::from_bytes::<DbString, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(decoded.as_str(), "db_string.rkyv.round_trip");
        assert_eq!(decoded, key);
    }
}
