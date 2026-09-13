//! Identifier positions within query_specification, scoped by its delimiters.
//!
//! Grammar audit: CALL roots/qualified parts, variable scopes, TABLE/GRAPH
//! arguments and YIELD lists; MATCH/INSERT/MERGE path, node, edge and label
//! names (also direct EXISTS patterns); FOR bindings/positions; focused
//! mutation target lists and labels. QueryFrames separately owns LET/schema
//! declarations, property/record fields, AS aliases, USE/AT names and `.name`.
//! Expression quotes remain strings: function_call/var_ref overlap literal in
//! `primary`, so a quoted expression root is not an unconditional name slot.

#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    #[default]
    Expression,
    Yield,
    CallRoot,
    CallName,
    CallArguments,
    CallScope,
    Pattern,
    Element,
    For,
    ForWith,
    Targets,
    Delete,
}

/// Local grammar state. Each delimiter frame owns its own copy.
#[derive(Clone, Copy, Default)]
pub(super) struct NameSlots {
    mode: Mode,
    name: bool,
    argument_start: bool,
    assignment: bool,
    exists: bool,
}

/// How to classify this token and initialize a newly opened delimiter.
#[derive(Default)]
pub(super) struct Names {
    pub(super) identifier: bool,
    pub(super) child: NameSlots,
}

impl NameSlots {
    pub(super) fn is_pattern(self) -> bool {
        self.mode == Mode::Pattern
    }

    pub(super) fn observe(&mut self, byte: u8, word: Option<&str>, protected: bool) -> Names {
        let mut result = Names::default();
        if matches!(byte, b'(' | b'[' | b'{') {
            result.child.mode = match (self.mode, byte) {
                _ if self.exists => Mode::Pattern,
                (Mode::Pattern, b'(' | b'[') => Mode::Element,
                (Mode::CallRoot, b'(') => Mode::CallScope,
                (Mode::CallName, b'(') => Mode::CallArguments,
                _ => Mode::Expression,
            };
            result.child.argument_start = result.child.mode == Mode::CallArguments;
            self.exists = false;
            if matches!(self.mode, Mode::CallRoot | Mode::CallName) {
                *self = Self::default();
            }
            return result;
        }
        self.exists = false;
        let name_token = word.is_some() || matches!(byte, b'"' | b'`');
        if self.name && name_token {
            self.name = false;
            result.identifier = true;
            if self.mode == Mode::CallRoot {
                self.mode = Mode::CallName;
            }
            return result;
        }
        if protected {
            return result;
        }
        if let Some(word) = word {
            if self.mode == Mode::ForWith && any(word, &["ORDINALITY", "OFFSET"]) {
                self.mode = Mode::Expression;
                self.name = true;
                return result;
            }
            if self.mode == Mode::CallArguments && self.argument_start {
                self.argument_start = false;
                if any(word, &["TABLE", "GRAPH"]) {
                    self.name = true;
                    return result;
                }
            }
            if any(word, &["MATCH", "INSERT", "MERGE"]) {
                self.start(Mode::Pattern, false);
            } else if any(word, &["SET", "REMOVE"]) {
                self.start(Mode::Targets, true);
            } else if word.eq_ignore_ascii_case("DELETE") {
                self.start(Mode::Delete, true);
            } else if word.eq_ignore_ascii_case("CALL") {
                self.start(Mode::CallRoot, true);
            } else if word.eq_ignore_ascii_case("YIELD") {
                self.start(Mode::Yield, true);
            } else if word.eq_ignore_ascii_case("FOR") {
                self.start(Mode::For, true);
            } else if word.eq_ignore_ascii_case("WITH") && self.mode == Mode::For {
                self.start(Mode::ForWith, false);
            } else if any(
                word,
                &[
                    "RETURN",
                    "WHERE",
                    "LET",
                    "WITH",
                    "FILTER",
                    "ORDER",
                    "GROUP",
                    "HAVING",
                    "OFFSET",
                    "LIMIT",
                    "SKIP",
                    "NEXT",
                    "UNION",
                    "EXCEPT",
                    "INTERSECT",
                    "OTHERWISE",
                    "FINISH",
                    "USE",
                    "AT",
                ],
            ) {
                *self = Self::default();
            } else if word.eq_ignore_ascii_case("EXISTS") {
                self.exists = true;
            } else if (self.mode == Mode::Yield && word.eq_ignore_ascii_case("AS"))
                || (self.mode == Mode::Targets
                    && !self.assignment
                    && word.eq_ignore_ascii_case("IS"))
            {
                self.name = true;
            } else if matches!(self.mode, Mode::Element | Mode::CallScope) {
                result.identifier = true;
            }
        } else if matches!(byte, b'"' | b'`') {
            result.identifier =
                matches!(self.mode, Mode::Pattern | Mode::Element | Mode::CallScope);
            self.argument_start = false;
        } else if byte == b',' {
            self.name = matches!(self.mode, Mode::Yield | Mode::Targets | Mode::Delete);
            self.assignment = false;
            self.argument_start = self.mode == Mode::CallArguments;
        } else if (byte == b'.' && self.mode == Mode::CallName)
            || (byte == b':' && self.mode == Mode::Targets && !self.assignment)
        {
            self.name = true;
        } else if byte == b'=' && self.mode == Mode::Targets {
            self.assignment = true;
        } else {
            self.name = false;
            self.argument_start = false;
        }
        result
    }

    fn start(&mut self, mode: Mode, name: bool) {
        *self = Self {
            mode,
            name,
            ..Self::default()
        };
    }
}

fn any(word: &str, words: &[&str]) -> bool {
    words
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
}
