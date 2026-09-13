//! Scan numeric tokens so adjacent keywords retain their real token boundary.

/// Return the byte immediately after the numeric token beginning at `start`.
///
/// The caller guarantees that `bytes[start]` is an ASCII digit. This mirrors
/// the numeric shapes in `grammar.pest`; in particular, a decimal point,
/// exponent, or one-letter numeric suffix belongs to the number rather than to
/// the identifier-like word that follows it.
pub(super) fn scan(bytes: &[u8], start: usize) -> usize {
    debug_assert!(bytes[start].is_ascii_digit());

    if bytes[start] == b'0'
        && let Some(end) = scan_prefixed_integer(bytes, start)
    {
        return end;
    }

    let mut end = scan_decimal_digits(bytes, start);
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        end = scan_decimal_digits(bytes, end);
    }

    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent_end = end + 1;
        if matches!(bytes.get(exponent_end), Some(b'+' | b'-')) {
            exponent_end += 1;
        }
        if bytes.get(exponent_end).is_some_and(u8::is_ascii_digit) {
            end = scan_decimal_digits(bytes, exponent_end);
        }
    }

    if matches!(
        bytes.get(end),
        Some(b'M' | b'm' | b'F' | b'f' | b'D' | b'd')
    ) {
        end += 1;
    }
    end
}

fn scan_prefixed_integer(bytes: &[u8], start: usize) -> Option<usize> {
    let radix = match bytes.get(start + 1) {
        Some(b'x') => 16,
        Some(b'o') => 8,
        Some(b'b') => 2,
        _ => return None,
    };
    let mut end = start + 2;
    let mut saw_digit = false;
    loop {
        if bytes.get(end) == Some(&b'_')
            && bytes
                .get(end + 1)
                .is_some_and(|byte| is_radix_digit(*byte, radix))
        {
            end += 1;
        }
        if bytes
            .get(end)
            .is_some_and(|byte| is_radix_digit(*byte, radix))
        {
            saw_digit = true;
            end += 1;
        } else {
            break;
        }
    }
    saw_digit.then_some(end)
}

fn scan_decimal_digits(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while matches!(bytes.get(end), Some(b'0'..=b'9' | b'_')) {
        end += 1;
    }
    end
}

fn is_radix_digit(byte: u8, radix: u32) -> bool {
    match radix {
        2 => matches!(byte, b'0' | b'1'),
        8 => matches!(byte, b'0'..=b'7'),
        16 => byte.is_ascii_hexdigit(),
        _ => false,
    }
}
