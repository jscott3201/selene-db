//! Exact nonrecursive tails from grammar.pest's type_name_base alternatives.

use super::{Scanner, skip_trivia};

impl Scanner<'_> {
    pub(super) fn scalar(&mut self, word: &str) -> bool {
        if any(word, &["SIGNED", "UNSIGNED"]) {
            if self.take_any(&["SMALL", "BIG"]) {
                return self.take("INTEGER");
            }
            let Some(integer) = self.next() else {
                return false;
            };
            if integer.eq_ignore_ascii_case("INTEGER") {
                self.parameters(true, false);
                return true;
            }
            return sized(integer, "INTEGER", true);
        }
        if any(word, &["SMALL", "BIG"]) {
            return self.take("INTEGER");
        }
        if any(word, &["INT", "INTEGER", "UINT", "FLOAT", "DECIMAL", "DEC"]) {
            self.parameters(true, any(word, &["FLOAT", "DECIMAL", "DEC"]));
        } else if any(
            word,
            &["STRING", "CHAR", "VARCHAR", "BYTES", "BINARY", "VARBINARY"],
        ) {
            self.parameters(false, any(word, &["STRING", "BYTES"]));
        } else if word.eq_ignore_ascii_case("DOUBLE") {
            self.take("PRECISION");
        } else if any(word, &["TIMESTAMP", "TIME"]) {
            let zone = self.sequence(&["WITH", "TIME", "ZONE"])
                || self.sequence(&["WITHOUT", "TIME", "ZONE"]);
            return zone || word.eq_ignore_ascii_case("TIMESTAMP");
        } else if any(word, &["ZONED", "LOCAL"]) {
            return self.take_any(&["DATETIME", "TIME"]);
        } else if word.eq_ignore_ascii_case("DURATION") {
            return self.take("(")
                && (self.sequence(&["YEAR", "TO", "MONTH"])
                    || self.sequence(&["DAY", "TO", "SECOND"]))
                && self.take(")");
        } else {
            return any(
                word,
                &[
                    "BOOLEAN",
                    "BOOL",
                    "BIGINT",
                    "SMALLINT",
                    "USMALLINT",
                    "UBIGINT",
                    "REAL",
                    "UUID",
                    "JSON",
                    "VECTOR",
                    "BYTEA",
                    "DATE",
                    "GRAPH",
                    "NODE",
                    "VERTEX",
                    "EDGE",
                    "RELATIONSHIP",
                    "PATH",
                    "NOTHING",
                    "NULL",
                ],
            ) || ["INTEGER", "INT", "UINT"]
                .iter()
                .any(|prefix| sized(word, prefix, true))
                || sized(word, "FLOAT", false);
        }
        true
    }

    fn parameters(&mut self, precision: bool, pair: bool) {
        let start = self.cursor;
        if self.take("(")
            && self.parameter(precision)
            && (!pair || !self.take(",") || self.parameter(precision))
            && self.take(")")
        {
            return;
        }
        self.cursor = start;
    }

    fn parameter(&mut self, precision: bool) -> bool {
        if precision {
            self.next().is_some_and(|token| {
                let mut digits = token.bytes();
                digits.next().is_some_and(|byte| byte.is_ascii_digit())
                    && token.split('_').all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
        } else {
            self.unsigned_integer(true)
        }
    }

    pub(super) fn cardinality(&mut self) {
        let start = self.cursor;
        if !(self.take("[") && self.unsigned_integer(false) && self.take("]")) {
            self.cursor = start;
        }
    }

    fn unsigned_integer(&mut self, atomic: bool) -> bool {
        self.cursor = skip_trivia(self.source.as_bytes(), self.cursor);
        let start = self.cursor;
        // unsigned_integer's radix prefixes are case-sensitive. Decimal
        // literals allow trailing/repeated underscores, radix literals do not.
        for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
            if self
                .source
                .get(start..)
                .is_some_and(|tail| tail.starts_with(prefix))
            {
                self.cursor = start + prefix.len();
                if self.radix_digit(radix, atomic) {
                    while self.radix_digit(radix, atomic) {}
                    return true;
                }
                self.cursor = start;
            }
        }
        if !self.digit(10, atomic) {
            return false;
        }
        while self.digit(10, atomic) || self.number_byte(b'_', atomic) {}
        true
    }

    fn radix_digit(&mut self, radix: u32, atomic: bool) -> bool {
        let start = self.cursor;
        self.number_byte(b'_', atomic);
        if self.digit(radix, atomic) {
            true
        } else {
            self.cursor = start;
            false
        }
    }

    fn digit(&mut self, radix: u32, atomic: bool) -> bool {
        let index = self.number_index(atomic);
        if self
            .source
            .as_bytes()
            .get(index)
            .is_some_and(|byte| byte.is_ascii_hexdigit() && char::from(*byte).is_digit(radix))
        {
            self.cursor = index + 1;
            true
        } else {
            false
        }
    }

    fn number_byte(&mut self, byte: u8, atomic: bool) -> bool {
        let index = self.number_index(atomic);
        if self.source.as_bytes().get(index) == Some(&byte) {
            self.cursor = index + 1;
            true
        } else {
            false
        }
    }

    fn number_index(&self, atomic: bool) -> usize {
        // Lengths are atomic; cardinality's unsigned_integer is non-atomic
        // and admits grammar trivia between its digits and underscores.
        if atomic {
            self.cursor
        } else {
            skip_trivia(self.source.as_bytes(), self.cursor)
        }
    }
}

pub(super) fn is_field_name(token: &str) -> bool {
    if token.starts_with(['"', '`']) {
        return token.len() > 2;
    }
    let mut chars = token.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || pest::unicode::LETTER(ch))
        && chars.all(|ch| ch == '_' || pest::unicode::LETTER(ch) || pest::unicode::NUMBER(ch))
}

fn any(word: &str, names: &[&str]) -> bool {
    names.iter().any(|name| word.eq_ignore_ascii_case(name))
}

fn sized(word: &str, prefix: &str, eight: bool) -> bool {
    word.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        && (matches!(
            word.get(prefix.len()..),
            Some("16" | "32" | "64" | "128" | "256")
        ) || (eight && word.get(prefix.len()..) == Some("8")))
}
