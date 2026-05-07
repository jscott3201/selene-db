//! Graph, catalog, and request identifier types per spec 02 section 4.

use std::fmt;

macro_rules! identity_id {
    ($Name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// The raw value `0` is reserved as a tombstone sentinel. Allocators
        /// start at `1`, and callers maintain that invariant when constructing
        /// IDs directly.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(transparent)]
        pub struct $Name(u64);

        impl $Name {
            #[doc = concat!("Construct a `", stringify!($Name), "` from a raw `u64`.")]
            #[must_use]
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            #[doc = concat!("Return the raw `u64` value of this `", stringify!($Name), "`.")]
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            /// The reserved tombstone sentinel value.
            pub const TOMBSTONE: Self = Self(0);
        }

        impl fmt::Display for $Name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($Name), self.0)
            }
        }
    };
}

identity_id!(NodeId, "Graph-scoped node identifier.");
identity_id!(EdgeId, "Graph-scoped edge identifier.");
identity_id!(GraphId, "Catalog-scoped graph identifier.");
identity_id!(BindingTableId, "Request-scoped binding-table identifier.");
identity_id!(RecordTypeId, "Graph-type-scoped record-type identifier.");

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(NodeId::TOMBSTONE.get())]
    #[case(EdgeId::TOMBSTONE.get())]
    #[case(GraphId::TOMBSTONE.get())]
    #[case(BindingTableId::TOMBSTONE.get())]
    #[case(RecordTypeId::TOMBSTONE.get())]
    fn tombstone_is_zero(#[case] raw: u64) {
        assert_eq!(raw, 0);
    }

    #[test]
    fn identity_types_are_eight_bytes() {
        assert_eq!(std::mem::size_of::<NodeId>(), 8);
        assert_eq!(std::mem::size_of::<EdgeId>(), 8);
        assert_eq!(std::mem::size_of::<GraphId>(), 8);
        assert_eq!(std::mem::size_of::<BindingTableId>(), 8);
        assert_eq!(std::mem::size_of::<RecordTypeId>(), 8);
    }

    #[test]
    fn display_includes_type_name_and_value() {
        assert_eq!(NodeId::new(42).to_string(), "NodeId(42)");
    }

    proptest! {
        #[test]
        fn node_id_round_trips(raw in any::<u64>()) {
            prop_assert_eq!(NodeId::new(raw).get(), raw);
        }

        #[test]
        fn edge_id_order_matches_raw_values(a in any::<u64>(), b in any::<u64>()) {
            prop_assert_eq!(EdgeId::new(a).cmp(&EdgeId::new(b)), a.cmp(&b));
        }
    }
}
