use proptest::prelude::*;

use super::*;
use crate::{Value, db_string};

proptest! {
    #[test]
    fn random_label_diff_preserves_sorted_deduped(
        raw_added in proptest::collection::vec(0_u8..32, 0..32),
        raw_removed in proptest::collection::vec(33_u8..64, 0..32),
    ) {
        let added = raw_added.into_iter().map(|value| {
            let name = format!("change.diff.{value}");
            db_string(&name).unwrap()
        });
        let removed = raw_removed.into_iter().map(|value| {
            let name = format!("change.diff.{value}");
            db_string(&name).unwrap()
        });
        let diff = LabelDiff::new(added, removed).unwrap();
        prop_assert!(diff.added.windows(2).all(|pair| pair[0] < pair[1]));
        prop_assert!(diff.removed.windows(2).all(|pair| pair[0] < pair[1]));
        prop_assert!(diff.added.iter().all(|label| !diff.removed.contains(label)));
    }

    #[test]
    fn random_property_diff_preserves_sorted_sets(
        raw_set in proptest::collection::vec(0_u8..32, 0..32),
        raw_removed in proptest::collection::vec(33_u8..64, 0..32),
    ) {
        let set = raw_set.into_iter().map(|value| {
            let name = format!("change.prop.{value}");
            (db_string(&name).unwrap(), Value::Uint(u64::from(value)))
        });
        let removed = raw_removed.into_iter().map(|value| {
            let name = format!("change.prop.{value}");
            db_string(&name).unwrap()
        });
        let diff = PropertyDiff::new(set, removed).unwrap();
        prop_assert!(diff.set.windows(2).all(|pair| pair[0].0 < pair[1].0));
        prop_assert!(diff.removed.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
