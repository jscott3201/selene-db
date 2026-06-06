use serde_json::{Map as SerdeJsonMap, Number as SerdeJsonNumber, Value as SerdeJsonValue};

use crate::{CoreError, CoreResult};

#[derive(Clone, Debug, Eq, PartialEq)]
struct JsonPointer {
    tokens: Vec<String>,
}

impl JsonPointer {
    fn parse(source: &str) -> CoreResult<Self> {
        if source.is_empty() {
            return Ok(Self { tokens: Vec::new() });
        }
        if !source.starts_with('/') {
            return json_patch_error("JSON Pointer must be empty or start with '/'");
        }
        let mut tokens = Vec::new();
        for raw in source[1..].split('/') {
            tokens.push(decode_pointer_token(raw)?);
        }
        Ok(Self { tokens })
    }

    fn is_root(&self) -> bool {
        self.tokens.is_empty()
    }

    fn is_proper_prefix_of(&self, other: &Self) -> bool {
        self.tokens.len() < other.tokens.len()
            && self
                .tokens
                .iter()
                .zip(&other.tokens)
                .all(|(lhs, rhs)| lhs == rhs)
    }
}

#[derive(Clone, Copy)]
enum ArrayIndexMode {
    Existing,
    Add,
}

/// Apply an RFC 6902 JSON Patch document to `target`.
pub(crate) fn apply_json_patch(
    target: &SerdeJsonValue,
    patch: &SerdeJsonValue,
) -> CoreResult<SerdeJsonValue> {
    let SerdeJsonValue::Array(operations) = patch else {
        return json_patch_error("JSON Patch document must be an array");
    };

    let mut document = target.clone();
    for (index, operation) in operations.iter().enumerate() {
        apply_operation(&mut document, operation, index)?;
    }
    Ok(document)
}

fn apply_operation(
    document: &mut SerdeJsonValue,
    operation: &SerdeJsonValue,
    index: usize,
) -> CoreResult<()> {
    let SerdeJsonValue::Object(operation) = operation else {
        return json_patch_error(format!("operation {index} is not an object"));
    };
    let op = required_string_member(operation, "op", index)?;
    let path = JsonPointer::parse(required_string_member(operation, "path", index)?)?;
    match op {
        "add" => {
            let value = required_member(operation, "value", index)?.clone();
            add_value(document, &path, value)
        }
        "remove" => remove_value(document, &path).map(drop),
        "replace" => {
            let value = required_member(operation, "value", index)?.clone();
            replace_value(document, &path, value)
        }
        "move" => {
            let from = JsonPointer::parse(required_string_member(operation, "from", index)?)?;
            move_value(document, &from, &path)
        }
        "copy" => {
            let from = JsonPointer::parse(required_string_member(operation, "from", index)?)?;
            let value = get_value(document, &from)?.clone();
            add_value(document, &path, value)
        }
        "test" => {
            let expected = required_member(operation, "value", index)?;
            let actual = get_value(document, &path)?;
            if json_equal(actual, expected) {
                Ok(())
            } else {
                json_patch_error(format!("operation {index} test failed"))
            }
        }
        other => json_patch_error(format!("operation {index} has unsupported op '{other}'")),
    }
}

fn required_member<'a>(
    operation: &'a SerdeJsonMap<String, SerdeJsonValue>,
    name: &'static str,
    index: usize,
) -> CoreResult<&'a SerdeJsonValue> {
    operation
        .get(name)
        .ok_or_else(|| json_patch_core_error(format!("operation {index} is missing '{name}'")))
}

fn required_string_member<'a>(
    operation: &'a SerdeJsonMap<String, SerdeJsonValue>,
    name: &'static str,
    index: usize,
) -> CoreResult<&'a str> {
    let value = required_member(operation, name, index)?;
    let Some(value) = value.as_str() else {
        return json_patch_error(format!("operation {index} member '{name}' is not a string"));
    };
    Ok(value)
}

fn add_value(
    document: &mut SerdeJsonValue,
    pointer: &JsonPointer,
    value: SerdeJsonValue,
) -> CoreResult<()> {
    if pointer.is_root() {
        *document = value;
        return Ok(());
    }
    let (parent, token) = parent_mut(document, pointer)?;
    match parent {
        SerdeJsonValue::Object(values) => {
            values.insert(token.to_owned(), value);
            Ok(())
        }
        SerdeJsonValue::Array(values) => {
            let index = parse_array_index(token, values.len(), ArrayIndexMode::Add)?;
            values.insert(index, value);
            Ok(())
        }
        _ => json_patch_error("add target parent is not an object or array"),
    }
}

fn remove_value(
    document: &mut SerdeJsonValue,
    pointer: &JsonPointer,
) -> CoreResult<SerdeJsonValue> {
    if pointer.is_root() {
        return json_patch_error("remove at the document root is not supported");
    }
    let (parent, token) = parent_mut(document, pointer)?;
    match parent {
        SerdeJsonValue::Object(values) => values
            .remove(token)
            .ok_or_else(|| json_patch_core_error("remove target object member does not exist")),
        SerdeJsonValue::Array(values) => {
            let index = parse_array_index(token, values.len(), ArrayIndexMode::Existing)?;
            Ok(values.remove(index))
        }
        _ => json_patch_error("remove target parent is not an object or array"),
    }
}

fn replace_value(
    document: &mut SerdeJsonValue,
    pointer: &JsonPointer,
    value: SerdeJsonValue,
) -> CoreResult<()> {
    if pointer.is_root() {
        *document = value;
        return Ok(());
    }
    let (parent, token) = parent_mut(document, pointer)?;
    match parent {
        SerdeJsonValue::Object(values) => {
            let Some(slot) = values.get_mut(token) else {
                return json_patch_error("replace target object member does not exist");
            };
            *slot = value;
            Ok(())
        }
        SerdeJsonValue::Array(values) => {
            let index = parse_array_index(token, values.len(), ArrayIndexMode::Existing)?;
            values[index] = value;
            Ok(())
        }
        _ => json_patch_error("replace target parent is not an object or array"),
    }
}

fn move_value(
    document: &mut SerdeJsonValue,
    from: &JsonPointer,
    path: &JsonPointer,
) -> CoreResult<()> {
    if from == path {
        get_value(document, from)?;
        return Ok(());
    }
    if from.is_proper_prefix_of(path) {
        return json_patch_error("move source must not be a proper prefix of target path");
    }
    let value = remove_value(document, from)?;
    add_value(document, path, value)
}

fn parent_mut<'a>(
    document: &'a mut SerdeJsonValue,
    pointer: &'a JsonPointer,
) -> CoreResult<(&'a mut SerdeJsonValue, &'a str)> {
    let (last, parents) = pointer
        .tokens
        .split_last()
        .expect("root pointer handled by caller");
    let parent = get_mut(document, parents)?;
    Ok((parent, last.as_str()))
}

fn get_mut<'a>(
    mut current: &'a mut SerdeJsonValue,
    tokens: &[String],
) -> CoreResult<&'a mut SerdeJsonValue> {
    for token in tokens {
        current = match current {
            SerdeJsonValue::Object(values) => values
                .get_mut(token)
                .ok_or_else(|| json_patch_core_error("JSON Pointer object member not found"))?,
            SerdeJsonValue::Array(values) => {
                let index = parse_array_index(token, values.len(), ArrayIndexMode::Existing)?;
                &mut values[index]
            }
            _ => return json_patch_error("JSON Pointer traversed a non-container value"),
        };
    }
    Ok(current)
}

fn get_value<'a>(
    mut current: &'a SerdeJsonValue,
    pointer: &JsonPointer,
) -> CoreResult<&'a SerdeJsonValue> {
    for token in &pointer.tokens {
        current = match current {
            SerdeJsonValue::Object(values) => values
                .get(token)
                .ok_or_else(|| json_patch_core_error("JSON Pointer object member not found"))?,
            SerdeJsonValue::Array(values) => {
                let index = parse_array_index(token, values.len(), ArrayIndexMode::Existing)?;
                &values[index]
            }
            _ => return json_patch_error("JSON Pointer traversed a non-container value"),
        };
    }
    Ok(current)
}

fn parse_array_index(token: &str, len: usize, mode: ArrayIndexMode) -> CoreResult<usize> {
    if token == "-" {
        return match mode {
            ArrayIndexMode::Add => Ok(len),
            ArrayIndexMode::Existing => json_patch_error("'-' is only valid for add array paths"),
        };
    }
    if token.is_empty() {
        return json_patch_error("array index is empty");
    }
    if token.len() > 1 && token.starts_with('0') {
        return json_patch_error("array index has a leading zero");
    }
    if !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return json_patch_error("array index is not a non-negative integer");
    }
    let index = token
        .parse::<usize>()
        .map_err(|_| json_patch_core_error("array index is too large"))?;
    match mode {
        ArrayIndexMode::Add if index <= len => Ok(index),
        ArrayIndexMode::Existing if index < len => Ok(index),
        ArrayIndexMode::Add => json_patch_error("add array index is out of bounds"),
        ArrayIndexMode::Existing => json_patch_error("array index is out of bounds"),
    }
}

fn decode_pointer_token(raw: &str) -> CoreResult<String> {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            decoded.push(ch);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            Some(other) => {
                return json_patch_error(format!("invalid JSON Pointer escape '~{other}'"));
            }
            None => return json_patch_error("invalid trailing '~' in JSON Pointer"),
        }
    }
    Ok(decoded)
}

fn json_equal(lhs: &SerdeJsonValue, rhs: &SerdeJsonValue) -> bool {
    match (lhs, rhs) {
        (SerdeJsonValue::Number(lhs), SerdeJsonValue::Number(rhs)) => numbers_equal(lhs, rhs),
        (SerdeJsonValue::Array(lhs), SerdeJsonValue::Array(rhs)) => {
            lhs.len() == rhs.len() && lhs.iter().zip(rhs).all(|(lhs, rhs)| json_equal(lhs, rhs))
        }
        (SerdeJsonValue::Object(lhs), SerdeJsonValue::Object(rhs)) => {
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .all(|(key, lhs)| rhs.get(key).is_some_and(|rhs| json_equal(lhs, rhs)))
        }
        _ => lhs == rhs,
    }
}

fn numbers_equal(lhs: &SerdeJsonNumber, rhs: &SerdeJsonNumber) -> bool {
    if lhs == rhs {
        return true;
    }
    match (lhs.as_i64(), lhs.as_u64(), rhs.as_i64(), rhs.as_u64()) {
        (Some(lhs), _, Some(rhs), _) => lhs == rhs,
        (_, Some(lhs), _, Some(rhs)) => lhs == rhs,
        (Some(lhs), _, _, Some(rhs)) => lhs >= 0 && u64::try_from(lhs).is_ok_and(|lhs| lhs == rhs),
        (_, Some(lhs), Some(rhs), _) => rhs >= 0 && u64::try_from(rhs).is_ok_and(|rhs| lhs == rhs),
        _ => lhs.as_f64() == rhs.as_f64(),
    }
}

fn json_patch_error<T>(message: impl Into<String>) -> CoreResult<T> {
    Err(json_patch_core_error(message))
}

fn json_patch_core_error(message: impl Into<String>) -> CoreError {
    CoreError::JsonPatch {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{CoreError, JsonValue};

    fn json(source: &str) -> JsonValue {
        JsonValue::parse_str(source).expect("JSON parses")
    }

    #[test]
    fn apply_patch_covers_rfc6902_appendix_examples() {
        for (target, patch, expected) in [
            (
                r#"{"foo":"bar"}"#,
                r#"[{"op":"add","path":"/baz","value":"qux"}]"#,
                r#"{"baz":"qux","foo":"bar"}"#,
            ),
            (
                r#"{"foo":["bar","baz"]}"#,
                r#"[{"op":"add","path":"/foo/1","value":"qux"}]"#,
                r#"{"foo":["bar","qux","baz"]}"#,
            ),
            (
                r#"{"baz":"qux","foo":"bar"}"#,
                r#"[{"op":"remove","path":"/baz"}]"#,
                r#"{"foo":"bar"}"#,
            ),
            (
                r#"{"foo":["bar","qux","baz"]}"#,
                r#"[{"op":"remove","path":"/foo/1"}]"#,
                r#"{"foo":["bar","baz"]}"#,
            ),
            (
                r#"{"baz":"qux","foo":"bar"}"#,
                r#"[{"op":"replace","path":"/baz","value":"boo"}]"#,
                r#"{"baz":"boo","foo":"bar"}"#,
            ),
            (
                r#"{"foo":{"bar":"baz","waldo":"fred"},"qux":{"corge":"grault"}}"#,
                r#"[{"op":"move","from":"/foo/waldo","path":"/qux/thud"}]"#,
                r#"{"foo":{"bar":"baz"},"qux":{"corge":"grault","thud":"fred"}}"#,
            ),
            (
                r#"{"foo":["all","grass","cows","eat"]}"#,
                r#"[{"op":"move","from":"/foo/1","path":"/foo/3"}]"#,
                r#"{"foo":["all","cows","eat","grass"]}"#,
            ),
            (
                r#"{"foo":"bar"}"#,
                r#"[{"op":"add","path":"/child","value":{"grandchild":{}}}]"#,
                r#"{"child":{"grandchild":{}},"foo":"bar"}"#,
            ),
            (
                r#"{"foo":"bar"}"#,
                r#"[{"op":"add","path":"/baz","value":"qux","xyz":123}]"#,
                r#"{"baz":"qux","foo":"bar"}"#,
            ),
            (
                r#"{"foo":["bar"]}"#,
                r#"[{"op":"add","path":"/foo/-","value":["abc","def"]}]"#,
                r#"{"foo":["bar",["abc","def"]]}"#,
            ),
        ] {
            assert_eq!(
                json(target)
                    .apply_patch(&json(patch))
                    .expect("patch succeeds")
                    .to_canonical_string(),
                json(expected).to_canonical_string()
            );
        }
    }

    #[test]
    fn apply_patch_supports_root_add_replace_copy_and_test() {
        assert_eq!(
            json(r#"{"a":1}"#)
                .apply_patch(&json(r#"[{"op":"add","path":"","value":{"root":true}}]"#))
                .expect("root add succeeds")
                .to_canonical_string(),
            r#"{"root":true}"#
        );
        assert_eq!(
            json(r#"{"a":1}"#)
                .apply_patch(&json(r#"[{"op":"replace","path":"","value":[1]}]"#))
                .expect("root replace succeeds")
                .to_canonical_string(),
            "[1]"
        );
        assert_eq!(
            json(r#"{"a":1}"#)
                .apply_patch(&json(r#"[{"op":"copy","from":"","path":"/copy"}]"#))
                .expect("root copy succeeds")
                .to_canonical_string(),
            r#"{"a":1,"copy":{"a":1}}"#
        );
        json(r#"{"a":1.00}"#)
            .apply_patch(&json(r#"[{"op":"test","path":"/a","value":1}]"#))
            .expect("numeric JSON Patch test succeeds");
        json(r#"{"a":1}"#)
            .apply_patch(&json(r#"[{"op":"move","from":"/a","path":"/a"}]"#))
            .expect("self move succeeds when source exists");
    }

    #[test]
    fn apply_patch_rejects_invalid_documents_without_mutating_source() {
        for patch in [
            r#"{"op":"add","path":"/a","value":1}"#,
            r#"[7]"#,
            r#"[{"path":"/a","value":1}]"#,
            r#"[{"op":"add","path":"/a"}]"#,
            r#"[{"op":"bad","path":"/a"}]"#,
            r#"[{"op":"remove","path":"/missing"}]"#,
            r#"[{"op":"replace","path":"/missing","value":1}]"#,
            r#"[{"op":"add","path":"/missing/child","value":1}]"#,
            r#"[{"op":"add","path":"/items/01","value":1}]"#,
            r#"[{"op":"remove","path":""}]"#,
            r#"[{"op":"move","from":"/a","path":"/a/b"}]"#,
            r#"[{"op":"move","from":"/missing","path":"/missing"}]"#,
            r#"[{"op":"test","path":"/a","value":2}]"#,
            r#"[{"op":"copy","from":"/missing","path":"/a"}]"#,
            r#"[{"op":"add","path":"bad","value":1}]"#,
            r#"[{"op":"add","path":"/bad~2escape","value":1}]"#,
        ] {
            let target = json(r#"{"a":{"b":1},"items":[0]}"#);
            let err = target
                .apply_patch(&json(patch))
                .expect_err("patch should fail");
            assert!(matches!(err, CoreError::JsonPatch { .. }), "{patch}: {err}");
            assert_eq!(target.to_canonical_string(), r#"{"a":{"b":1},"items":[0]}"#);
        }
    }
}
