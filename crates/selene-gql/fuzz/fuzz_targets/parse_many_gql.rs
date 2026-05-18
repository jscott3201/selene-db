#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_gql::{ParserError, parse_many};

fuzz_target!(|bytes: &[u8]| {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return;
    };
    match parse_many(source) {
        Ok(_) => {}
        Err(ParserError::SyntaxError { .. })
        | Err(ParserError::UnsupportedFeature { .. })
        | Err(ParserError::NotImplemented { .. })
        | Err(ParserError::InternerBudgetExceeded { .. })
        | Err(ParserError::NestingLimitExceeded { .. }) => {}
        Err(_) => {}
    }
});
