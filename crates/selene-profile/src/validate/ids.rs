//! Identifier-shape checks; identifiers remain opaque after validation.

pub(super) fn valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

pub(super) fn valid_feature_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 4
        && ((bytes[0].is_ascii_uppercase() && bytes[1..].iter().all(u8::is_ascii_digit))
            || (bytes[..2].iter().all(u8::is_ascii_uppercase)
                && bytes[2..].iter().all(u8::is_ascii_digit)))
}

pub(super) fn valid_extension_id(id: &str) -> bool {
    id.strip_prefix("IM_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

pub(super) fn valid_impl_defined_id(id: &str) -> bool {
    let valid_one = |value: &str| {
        let bytes = value.as_bytes();
        bytes.len() == 5
            && bytes[..2].iter().all(u8::is_ascii_uppercase)
            && bytes[2..].iter().all(u8::is_ascii_digit)
    };
    if let Some((start, end)) = id.split_once('-') {
        valid_one(start) && valid_one(end)
    } else {
        valid_one(id)
    }
}

pub(super) fn valid_prefixed(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.bytes().all(|byte| {
                byte.is_ascii_uppercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            })
    })
}
