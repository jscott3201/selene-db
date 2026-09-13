//! Distinguish a VALUE query body from maps/quantifiers after an identifier.

use super::{next_is, next_sig_is_colon, scan_word_chars, skip_block_comment, skip_line_comment};

/// Whether a brace after an unqualified VALUE can introduce a query body.
///
/// VALUE is not reserved: node/edge variables and labels may have that name.
/// Their property maps start with a key and colon; a query specification never
/// does. Quoted keys cannot start a query either. Complete numeric edge
/// quantifiers are also unambiguous. Leave other (including incomplete) bodies
/// counted so malformed input is bounded before pest can retry its descent.
pub(super) fn may_start(source: &str, after_brace: usize) -> bool {
    let bytes = source.as_bytes();
    let start = skip_trivia(bytes, after_brace);
    match bytes.get(start) {
        Some(b'"' | b'`') => false,
        Some(b'0'..=b'9' | b',') => !is_complete_quantifier(bytes, start),
        Some(_) => !next_sig_is_colon(bytes, scan_word_chars(source, start)),
        None => true,
    }
}

fn is_complete_quantifier(bytes: &[u8], mut index: usize) -> bool {
    loop {
        index = skip_trivia(bytes, index);
        match bytes.get(index) {
            Some(b'0'..=b'9' | b',') => index += 1,
            Some(b'}') => return true,
            _ => return false,
        }
    }
}

pub(super) fn skip_trivia(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'/' if next_is(bytes, index, b'/') => index = skip_line_comment(bytes, index + 2),
            b'/' if next_is(bytes, index, b'*') => index = skip_block_comment(bytes, index + 2) + 1,
            _ => break,
        }
    }
    index
}
