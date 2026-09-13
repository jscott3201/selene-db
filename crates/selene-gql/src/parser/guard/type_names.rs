//! Locate a typed USE parameter's type boundary without entering pest.
//!
//! This recognizes grammar syntax, not semantic type validity/precision. In particular,
//! optional RECORD fields must finish before they can hide a following query
//! brace: `RECORD {RETURN LIST}` is a type, but `RECORD {RETURN 0}` is not.
//! Explicit work/fallback stacks keep this lookahead off the native call stack.

use super::{MAX_NESTING_DEPTH, scan_word_chars, value_queries::skip_trivia};

mod scalar;
#[cfg(test)]
mod tests;

// These are storage-slot limits, not claimed type-depth limits. Each recursive
// type level needs at most eight work slots and two fallback slots; one extra
// level allows the depth-zero root plus the existing 64 nested type levels.
const WORK_CAPACITY: usize = (MAX_NESTING_DEPTH as usize + 1) * 8;
const FALLBACK_CAPACITY: usize = (MAX_NESTING_DEPTH as usize + 1) * 2;

/// A full lookahead array must reject before pest, never become a syntax miss.
#[derive(Debug)]
pub(super) struct CapacityExceeded {
    /// The exhausted array's capacity in slots, not syntactic nesting levels.
    pub(super) limit: u32,
}

#[derive(Clone, Copy, Default)]
enum Task {
    #[default]
    Type,
    Infix,
    Primary,
    Suffix,
    Union,
    Fields,
    Field,
    FieldTail,
    Cardinality,
    Close(&'static str),
    Commit(usize),
}

#[derive(Clone, Copy, Default)]
struct Fallback {
    cursor: usize,
    tasks: usize,
    resume: Option<Task>,
}

struct Scanner<'a> {
    source: &'a str,
    cursor: usize,
    tasks: [Task; WORK_CAPACITY],
    len: usize,
    fallbacks: [Fallback; FALLBACK_CAPACITY],
    alternatives: usize,
}

/// Return the type boundary, a syntax miss, or a bounded-storage failure.
pub(super) fn end(source: &str, cursor: usize) -> Result<Option<usize>, CapacityExceeded> {
    let mut scan = Scanner {
        source,
        cursor,
        tasks: [Task::Type; WORK_CAPACITY],
        len: 1,
        fallbacks: [Fallback::default(); FALLBACK_CAPACITY],
        alternatives: 0,
    };
    while scan.len > 0 {
        scan.len -= 1;
        let task = scan.tasks[scan.len];
        if !scan.step(task)? {
            let Some(alternative) = scan.alternatives.checked_sub(1) else {
                return Ok(None);
            };
            scan.alternatives = alternative;
            let fallback = scan.fallbacks[alternative];
            scan.cursor = fallback.cursor;
            scan.len = fallback.tasks;
            if let Some(task) = fallback.resume {
                scan.push(task)?;
            }
        }
    }
    Ok(Some(scan.cursor))
}

impl<'a> Scanner<'a> {
    fn push(&mut self, task: Task) -> Result<(), CapacityExceeded> {
        *self.tasks.get_mut(self.len).ok_or(CapacityExceeded {
            limit: WORK_CAPACITY as u32,
        })? = task;
        self.len += 1;
        Ok(())
    }

    fn attempt(
        &mut self,
        cursor: usize,
        resume: Option<Task>,
        continuation: Option<Task>,
    ) -> Result<(), CapacityExceeded> {
        *self
            .fallbacks
            .get_mut(self.alternatives)
            .ok_or(CapacityExceeded {
                limit: FALLBACK_CAPACITY as u32,
            })? = Fallback {
            cursor,
            tasks: self.len,
            resume,
        };
        if let Some(task) = continuation {
            self.push(task)?;
        }
        self.push(Task::Commit(self.alternatives))?;
        self.alternatives += 1;
        Ok(())
    }

    fn step(&mut self, task: Task) -> Result<bool, CapacityExceeded> {
        match task {
            Task::Type => {
                let start = self.cursor;
                let any = self.take("ANY");
                if any {
                    self.take("VALUE");
                }
                if any && self.take("<") {
                    // A prefixed union contains primaries, and is itself only a
                    // type_name alternative, never a primary or suffix base.
                    self.attempt(start, Some(Task::Infix), None)?;
                    self.push(Task::Close(">"))?;
                } else {
                    self.cursor = start;
                }
                self.push(Task::Infix)?;
            }
            Task::Infix => {
                self.push(Task::Union)?;
                self.push(Task::Primary)?;
            }
            Task::Primary => {
                self.push(Task::Suffix)?;
                return self.base();
            }
            Task::Union => {
                let start = self.cursor;
                if self.take("|") {
                    self.attempt(start, None, Some(Task::Union))?;
                    self.push(Task::Primary)?;
                }
            }
            Task::Suffix => {
                let start = self.cursor;
                self.sequence(&["NOT", "NULL"]);
                if self.take("LIST") || self.take("ARRAY") {
                    self.cardinality();
                    self.push(Task::Suffix)?;
                } else {
                    self.cursor = start;
                    self.sequence(&["NOT", "NULL"]);
                }
            }
            Task::Fields => {
                if !self.take("}") {
                    self.push(Task::Field)?;
                }
            }
            Task::Field => {
                if !self.next().is_some_and(scalar::is_field_name) {
                    return Ok(false);
                }
                // The :: marker is atomic: trivia between its colons is invalid.
                if !self.take("::") {
                    self.take("TYPED");
                }
                self.push(Task::FieldTail)?;
                self.push(Task::Type)?;
            }
            Task::FieldTail => {
                if self.take(",") {
                    self.push(Task::Field)?;
                } else if !self.take("}") {
                    return Ok(false);
                }
            }
            Task::Close(token) => return Ok(self.take(token)),
            Task::Cardinality => self.cardinality(),
            Task::Commit(alternatives) => self.alternatives = alternatives,
        }
        Ok(true)
    }

    fn base(&mut self) -> Result<bool, CapacityExceeded> {
        if self.take("{") {
            self.push(Task::Fields)?;
            return Ok(true);
        }
        let Some(word) = self.next() else {
            return Ok(false);
        };
        if word.eq_ignore_ascii_case("RECORD") {
            let start = self.cursor;
            if self.take("{") {
                self.attempt(start, None, None)?;
                self.push(Task::Fields)?;
            }
        } else if word.eq_ignore_ascii_case("ANY") {
            if !(self.sequence(&["PROPERTY", "GRAPH"])
                || self.take_any(&["GRAPH", "NODE", "VERTEX", "EDGE", "RELATIONSHIP", "RECORD"])
                || self.sequence(&["PROPERTY", "VALUE"]))
            {
                self.take("VALUE");
            }
        } else if word.eq_ignore_ascii_case("PROPERTY") {
            return Ok(self.take("VALUE") || self.take("GRAPH"));
        } else if word.eq_ignore_ascii_case("TABLE") || word.eq_ignore_ascii_case("BINDING") {
            if word.eq_ignore_ascii_case("BINDING") && !self.take("TABLE") {
                return Ok(false);
            }
            if !self.take("{") {
                return Ok(false);
            }
            self.push(Task::Fields)?;
        } else if word.eq_ignore_ascii_case("LIST") || word.eq_ignore_ascii_case("ARRAY") {
            let start = self.cursor;
            if self.take("<") {
                self.attempt(start, Some(Task::Cardinality), None)?;
                self.push(Task::Cardinality)?;
                self.push(Task::Close(">"))?;
                self.push(Task::Type)?;
            } else {
                self.cardinality();
            }
        } else {
            return Ok(self.scalar(word));
        }
        Ok(true)
    }

    fn take_any(&mut self, words: &[&str]) -> bool {
        words.iter().any(|word| self.take(word))
    }

    fn sequence(&mut self, words: &[&str]) -> bool {
        let start = self.cursor;
        if words.iter().all(|word| self.take(word)) {
            true
        } else {
            self.cursor = start;
            false
        }
    }

    fn take(&mut self, token: &str) -> bool {
        let start = self.cursor;
        if self
            .next()
            .is_some_and(|next| next.eq_ignore_ascii_case(token))
        {
            true
        } else {
            self.cursor = start;
            false
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        let bytes = self.source.as_bytes();
        let start = skip_trivia(bytes, self.cursor);
        let byte = *bytes.get(start)?;
        self.cursor = start + 1;
        if byte == b':' && bytes.get(self.cursor) == Some(&b':') {
            self.cursor += 1;
        } else if matches!(byte, b'"' | b'`') {
            loop {
                let next = *bytes.get(self.cursor)?;
                self.cursor += 1;
                if next == byte {
                    if bytes.get(self.cursor) != Some(&byte) {
                        break;
                    }
                    self.cursor += 1;
                }
            }
        } else if super::is_word_byte_start(byte) || byte.is_ascii_digit() {
            self.cursor = scan_word_chars(self.source, start);
            if self.cursor == start {
                // As in the main guard, a non-identifier Unicode character is
                // one whole token, never an empty token or a partial UTF-8 byte.
                self.cursor += super::char_len_at(self.source, start);
            }
        }
        self.source.get(start..self.cursor)
    }
}
