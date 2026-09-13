//! Track query entry and field-name positions without parsing expressions/types.
//!
//! A field-types specification and a record/property map both begin each field
//! with a name. Treat that position as an identifier regardless of its spelling
//! or of the field's type (including `VALUE {RETURN LIST}`). Query bodies have
//! no such field-name slots. All state is bounded by the existing delimiter cap.

use super::{
    MAX_NESTING_DEPTH, name_slots::NameSlots, next_is, next_sig_is_colon, scan_word_chars,
    type_names, value_queries,
};

#[derive(Clone, Copy, Default)]
struct Frame {
    delimiter: u8,
    fields: bool,
    field_name: bool,
    call_scope: bool,
    let_bindings: bool,
    declarations: bool,
    default_expr: bool,
    names: NameSlots,
}

#[derive(Clone, Copy, Default)]
enum Pending {
    #[default]
    None,
    Value,
    Call,
    UseName,
    UseVariable,
    UseReady,
    AtName,
    AtReady,
    BareQuery,
    Let,
    BindingName,
    SchemaStart,
    SchemaElement,
    SchemaFields,
}

/// Grammar context needed by the caller before it skips the current token.
#[derive(Default)]
pub(super) struct Observation {
    /// An opening brace that repeats query parsing.
    pub(super) query_brace: bool,
    /// A quoted name uses doubled delimiters and treats backslashes literally.
    pub(super) quoted_identifier: bool,
}

pub(super) struct QueryFrames {
    frames: [Frame; MAX_NESTING_DEPTH as usize],
    depth: usize,
    pending: Pending,
    root_let_bindings: bool,
    type_end: usize,
    root_names: NameSlots,
}

impl Default for QueryFrames {
    fn default() -> Self {
        Self {
            frames: [Frame::default(); MAX_NESTING_DEPTH as usize],
            depth: 0,
            pending: Pending::None,
            root_let_bindings: false,
            type_end: 0,
            root_names: NameSlots::default(),
        }
    }
}

impl QueryFrames {
    /// Observe one significant token and report its query/identifier context.
    /// The caller skips whole words, numbers and quoted tokens between calls.
    pub(super) fn observe(
        &mut self,
        source: &str,
        index: usize,
        previous: Option<u8>,
        after_alias: bool,
    ) -> Result<Observation, type_names::CapacityExceeded> {
        let bytes = source.as_bytes();
        let byte = bytes[index];
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
            || (byte == b'/' && (next_is(bytes, index, b'/') || next_is(bytes, index, b'*')))
        {
            return Ok(Observation::default());
        }

        let parent = self.depth.checked_sub(1).map(|depth| self.frames[depth]);
        let type_token = index < self.type_end;
        let field_name = parent.is_some_and(|frame| {
            (frame.fields && frame.field_name) || (frame.declarations && !frame.default_expr)
        });
        let let_bindings = parent.map_or(self.root_let_bindings, |frame| frame.let_bindings);
        let word =
            super::is_word_byte_start(byte).then(|| &source[index..scan_word_chars(source, index)]);
        let protected =
            type_token || field_name || after_alias || matches!(previous, Some(b'.' | b'$'));
        let names = if let Some(depth) = self.depth.checked_sub(1) {
            &mut self.frames[depth].names
        } else {
            &mut self.root_names
        }
        .observe(byte, word, protected);
        if let Some(depth) = self.depth.checked_sub(1) {
            self.frames[depth].field_name = self.frames[depth].fields && byte == b',';
            if byte == b',' {
                self.frames[depth].default_expr = false;
            }
        }

        let pending = if type_token {
            Pending::None
        } else {
            std::mem::take(&mut self.pending)
        };
        let quoted_identifier = matches!(byte, b'"' | b'`')
            && (names.identifier
                || type_token
                || field_name
                || previous == Some(b'.')
                || after_alias
                || parent.is_some_and(|frame| frame.call_scope)
                || matches!(
                    pending,
                    Pending::UseName
                        | Pending::UseVariable
                        | Pending::AtName
                        | Pending::SchemaFields
                        | Pending::Let
                        | Pending::BindingName
                ));
        match byte {
            b'{' => {
                let retries = matches!(
                    pending,
                    Pending::Call | Pending::UseReady | Pending::AtReady
                ) || (matches!(pending, Pending::Value)
                    && value_queries::may_start(source, index + 1));
                let query = retries
                    || names.child.is_pattern()
                    || previous.is_none()
                    || matches!(pending, Pending::BareQuery)
                    || (previous == Some(b'{') && parent.is_some_and(|frame| !frame.fields));
                self.push(Frame {
                    delimiter: byte,
                    fields: !query,
                    field_name: !query,
                    names: names.child,
                    ..Frame::default()
                });
                self.pending = if type_token {
                    Pending::UseReady
                } else {
                    Pending::None
                };
                return Ok(Observation {
                    query_brace: retries,
                    ..Observation::default()
                });
            }
            b'(' | b'[' => self.push(Frame {
                delimiter: byte,
                call_scope: byte == b'(' && matches!(pending, Pending::Call),
                fields: matches!(pending, Pending::SchemaFields),
                field_name: matches!(pending, Pending::SchemaFields),
                declarations: matches!(pending, Pending::SchemaFields),
                names: names.child,
                ..Frame::default()
            }),
            b')' | b']' | b'}' => {
                let opener = match byte {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if let Some(frame) = parent
                    && frame.delimiter == opener
                {
                    self.depth -= 1;
                    if frame.call_scope {
                        self.pending = Pending::Call;
                    }
                }
            }
            b'"' | b'`' => {
                self.pending = match pending {
                    Pending::UseName | Pending::UseVariable => Pending::UseReady,
                    Pending::AtName => Pending::AtReady,
                    Pending::SchemaFields => Pending::SchemaFields,
                    _ => Pending::None,
                };
            }
            b'.' | b'/' | b'$' => {
                self.pending = match pending {
                    Pending::UseName | Pending::UseReady => Pending::UseName,
                    Pending::AtName | Pending::AtReady => Pending::AtName,
                    Pending::SchemaFields => Pending::SchemaFields,
                    _ => Pending::None,
                };
            }
            byte if super::is_word_byte_start(byte) => {
                let end = scan_word_chars(source, index);
                let word = &source[index..end];
                let identifier = names.identifier
                    || type_token
                    || field_name
                    || matches!(previous, Some(b'.' | b'$'))
                    || after_alias
                    || next_sig_is_colon(bytes, end);
                if !identifier && word.eq_ignore_ascii_case("LET") {
                    self.set_let_bindings(true);
                } else if !identifier && is_statement_boundary(word) {
                    self.set_let_bindings(false);
                }
                if word.eq_ignore_ascii_case("DEFAULT")
                    && parent.is_some_and(|frame| !frame.field_name)
                    && let Some(depth) = self.depth.checked_sub(1)
                    && self.frames[depth].declarations
                {
                    self.frames[depth].default_expr = true;
                }
                self.pending = match pending {
                    Pending::UseName if word.eq_ignore_ascii_case("VARIABLE") => {
                        Pending::UseVariable
                    }
                    Pending::UseName | Pending::UseVariable => Pending::UseReady,
                    Pending::AtName => Pending::AtReady,
                    Pending::Let if word.eq_ignore_ascii_case("VALUE") => Pending::BindingName,
                    Pending::BindingName => Pending::None,
                    Pending::SchemaStart
                        if word.eq_ignore_ascii_case("NODE")
                            || word.eq_ignore_ascii_case("EDGE") =>
                    {
                        Pending::SchemaElement
                    }
                    Pending::SchemaStart
                        if word.eq_ignore_ascii_case("OR")
                            || word.eq_ignore_ascii_case("REPLACE") =>
                    {
                        Pending::SchemaStart
                    }
                    Pending::SchemaElement if word.eq_ignore_ascii_case("TYPE") => {
                        Pending::SchemaFields
                    }
                    Pending::SchemaFields => Pending::SchemaFields,
                    _ if identifier => Pending::None,
                    _ if word.eq_ignore_ascii_case("VALUE") => Pending::Value,
                    _ if word.eq_ignore_ascii_case("CALL") => Pending::Call,
                    _ if word.eq_ignore_ascii_case("USE") => Pending::UseName,
                    _ if word.eq_ignore_ascii_case("AT") => Pending::AtName,
                    _ if word.eq_ignore_ascii_case("LET") => Pending::Let,
                    _ if self.depth == 0
                        && (word.eq_ignore_ascii_case("CREATE")
                            || word.eq_ignore_ascii_case("ALTER")) =>
                    {
                        Pending::SchemaStart
                    }
                    _ if is_query_boundary(word) => Pending::BareQuery,
                    _ => Pending::None,
                };
            }
            b':' if matches!(pending, Pending::UseReady) && next_is(bytes, index, b':') => {
                self.type_end = type_names::end(source, index + 2)?.unwrap_or(index + 2);
                self.pending = Pending::UseReady;
            }
            b',' if let_bindings => self.pending = Pending::Let,
            _ if matches!(pending, Pending::SchemaFields) => self.pending = Pending::SchemaFields,
            _ => {}
        }
        if type_token {
            self.pending = Pending::UseReady;
        }
        Ok(Observation {
            quoted_identifier,
            ..Observation::default()
        })
    }

    fn set_let_bindings(&mut self, active: bool) {
        if let Some(depth) = self.depth.checked_sub(1) {
            self.frames[depth].let_bindings = active;
        } else {
            self.root_let_bindings = active;
        }
    }

    fn push(&mut self, frame: Frame) {
        // The main delimiter guard rejects the next token past this capacity.
        if let Some(slot) = self.frames.get_mut(self.depth) {
            *slot = frame;
            self.depth += 1;
        }
    }
}

fn is_statement_boundary(word: &str) -> bool {
    [
        "MATCH", "FOR", "WITH", "FILTER", "ORDER", "OFFSET", "LIMIT", "RETURN", "CALL", "NEXT",
    ]
    .iter()
    .any(|keyword| word.eq_ignore_ascii_case(keyword))
}

fn is_query_boundary(word: &str) -> bool {
    [
        "EXPLAIN",
        "NEXT",
        "UNION",
        "EXCEPT",
        "INTERSECT",
        "OTHERWISE",
        "ALL",
        "DISTINCT",
    ]
    .iter()
    .any(|keyword| word.eq_ignore_ascii_case(keyword))
}
