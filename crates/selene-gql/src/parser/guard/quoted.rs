//! Quote skipping shared by the pre-pest resource guards.

use super::next_is;

/// Skip an ident/prop_ident: only doubled delimiters escape a delimiter.
/// Unlike expression strings, an identifier's backslash is always literal.
pub(super) fn skip_identifier_quoted(bytes: &[u8], mut index: usize, delimiter: u8) -> usize {
    while index < bytes.len() {
        if bytes[index] == delimiter {
            if !next_is(bytes, index, delimiter) {
                return index;
            }
            index += 1;
        }
        index += 1;
    }
    bytes.len()
}

pub(super) fn skip_single_quoted(
    bytes: &[u8],
    mut index: usize,
    last_quote: Option<usize>,
) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            // `\'` where the `'` is the final quote in the input is a *dangling*
            // escape (pest `dangling_escape`): the `\` is literal and the `'`
            // closes the string. Return the `'` position so the scan resumes
            // after it and still counts any following brackets — matching pest,
            // which closes the string here too. Any other `\X` (including a
            // `\'` with a later quote — pest `escaped_quote`) escapes one byte.
            b'\\' if bytes.get(index + 1) == Some(&b'\'') && Some(index + 1) == last_quote => {
                return index + 1;
            }
            b'\\' => index += 2,
            b'\'' if next_is(bytes, index, b'\'') => index += 2,
            b'\'' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

pub(super) fn skip_double_quoted(
    bytes: &[u8],
    mut index: usize,
    last_quote: Option<usize>,
) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'"') && Some(index + 1) == last_quote => {
                return index + 1;
            }
            b'\\' => index += 2,
            b'"' if next_is(bytes, index, b'"') => index += 2,
            b'"' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

pub(super) fn skip_no_escape_quoted(bytes: &[u8], mut index: usize, delimiter: u8) -> usize {
    while index < bytes.len() {
        if bytes[index] == delimiter {
            return index;
        }
        index += 1;
    }
    bytes.len()
}

pub(super) fn skip_backtick_quoted(
    bytes: &[u8],
    mut index: usize,
    last_backtick: Option<usize>,
) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'`') && Some(index + 1) == last_backtick => {
                return index + 1;
            }
            b'\\' => index += 2,
            b'`' if next_is(bytes, index, b'`') => index += 2,
            b'`' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}
