#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_gql::{
    ast::{format_read_statement, structurally_eq},
    parse,
};

fuzz_target!(|bytes: &[u8]| {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(first) = parse(source) else {
        return;
    };
    let Ok(formatted) = format_read_statement(&first) else {
        return;
    };
    let Ok(second) = parse(&formatted) else {
        panic!("formatted source did not parse: {formatted:?}");
    };
    assert!(
        structurally_eq(&first, &second),
        "formatted source was {formatted:?}"
    );
});
