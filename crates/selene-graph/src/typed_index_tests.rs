use roaring::RoaringBitmap;
use selene_core::Value;

use super::*;

fn decimal(value: &str) -> rust_decimal::Decimal {
    value.parse().expect("test decimal parses")
}

fn row_index(index: &TypedIndex, value: &Value) -> RoaringBitmap {
    index.lookup_eq(value).expect("kind matches").into_owned()
}

#[path = "typed_index_tests/basics.rs"]
mod basics;
#[path = "typed_index_tests/lookup.rs"]
mod lookup;
#[path = "typed_index_tests/ranges.rs"]
mod ranges;
#[path = "typed_index_tests/strings.rs"]
mod strings;
