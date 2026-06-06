use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map as SerdeJsonMap, Value as SerdeJsonValue};

use crate::{
    CoreError, CoreResult, DbString, db_string::MAX_DB_STRING_BYTES, json_patch::apply_json_patch,
};

/// Selector used by JSON path-existence helpers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JsonPathSelector {
    /// Select an object member by key.
    Key(DbString),
    /// Select an array element by signed index.
    ///
    /// Negative indexes count from the end, matching `json_has_path`.
    Index(i64),
    /// Select an array element by unsigned index.
    UnsignedIndex(u64),
}

/// Native JSON payload stored as a first-class [`crate::Value`].
///
/// `JsonValue` validates once at construction and deserialization so every
/// engine layer can assume a finite JSON data-model value whose strings and
/// container cardinalities stay inside the implementation-defined GQL caps.
#[derive(Clone, Debug, PartialEq)]
pub struct JsonValue {
    value: Arc<SerdeJsonValue>,
}

impl JsonValue {
    /// Build a validated JSON value from an owned serde-json value.
    pub fn new(value: SerdeJsonValue) -> CoreResult<Self> {
        validate_json_value(&value)?;
        Ok(Self {
            value: Arc::new(value),
        })
    }

    /// Parse and validate a JSON value from text.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::JsonParse`] when `text` is not valid JSON, or the
    /// usual value-limit errors when the parsed value exceeds engine caps.
    pub fn parse_str(text: &str) -> CoreResult<Self> {
        let value = serde_json::from_str(text).map_err(|err| CoreError::JsonParse {
            message: err.to_string(),
        })?;
        Self::new(value)
    }

    /// Borrow the underlying serde-json value.
    #[must_use]
    pub fn as_serde(&self) -> &SerdeJsonValue {
        &self.value
    }

    /// Clone the shared JSON storage.
    #[must_use]
    pub fn as_arc(&self) -> Arc<SerdeJsonValue> {
        Arc::clone(&self.value)
    }

    /// Return a stable compact JSON rendering with object keys sorted.
    #[must_use]
    pub fn to_canonical_string(&self) -> String {
        let mut output = String::new();
        write_json_canonical(self.as_serde(), &mut output);
        output
    }

    /// Return the JSON data-model type name.
    #[must_use]
    pub fn json_type_name(&self) -> &'static str {
        match self.as_serde() {
            SerdeJsonValue::Null => "null",
            SerdeJsonValue::Bool(_) => "boolean",
            SerdeJsonValue::Number(_) => "number",
            SerdeJsonValue::String(_) => "string",
            SerdeJsonValue::Array(_) => "array",
            SerdeJsonValue::Object(_) => "object",
        }
    }

    /// Return true when this JSON value recursively contains `candidate`.
    ///
    /// Objects contain candidates whose keys are present and whose values are
    /// themselves contained. Arrays contain array candidates when each
    /// candidate element is contained by at least one target element. A scalar
    /// or object candidate also matches any containing target array element.
    #[must_use]
    pub fn contains(&self, candidate: &Self) -> bool {
        json_contains_value(self.as_serde(), candidate.as_serde())
    }

    /// Return true when every selector in `path` resolves inside this value.
    ///
    /// Stored-value shape mismatches return false. This makes the helper useful
    /// for heterogeneous graph scans where one malformed document should not
    /// abort the entire candidate search.
    #[must_use]
    pub fn path_exists(&self, path: &[JsonPathSelector]) -> bool {
        let mut current = self.as_serde();
        for selector in path {
            let Some(next) = select_json_child(current, selector) else {
                return false;
            };
            current = next;
        }
        true
    }

    /// Return the result of applying an RFC 7396 JSON Merge Patch document.
    ///
    /// The operation is copy-on-write: this value and `patch` are left
    /// unchanged, and the merged result is validated as a fresh [`JsonValue`].
    ///
    /// # Errors
    ///
    /// Returns the usual value-limit errors if the merged result exceeds engine
    /// caps.
    pub fn merge_patch(&self, patch: &Self) -> CoreResult<Self> {
        let mut target = self.as_serde().clone();
        merge_patch_value(&mut target, patch.as_serde());
        Self::new(target)
    }

    /// Return the result of applying an RFC 6902 JSON Patch document.
    ///
    /// The operation is atomic from the caller's perspective: this value and
    /// `patch` are left unchanged, and an invalid patch returns an error
    /// without exposing any intermediate document state.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::JsonPatch`] when the patch document is malformed or
    /// an operation fails. Returns the usual value-limit errors if the patched
    /// result exceeds engine caps.
    pub fn apply_patch(&self, patch: &Self) -> CoreResult<Self> {
        Self::new(apply_json_patch(self.as_serde(), patch.as_serde())?)
    }
}

fn select_json_child<'a>(
    current: &'a SerdeJsonValue,
    selector: &JsonPathSelector,
) -> Option<&'a SerdeJsonValue> {
    match (current, selector) {
        (SerdeJsonValue::Object(values), JsonPathSelector::Key(key)) => values.get(key.as_str()),
        (SerdeJsonValue::Array(values), JsonPathSelector::Index(index)) => {
            signed_array_index(*index, values.len()).and_then(|index| values.get(index))
        }
        (SerdeJsonValue::Array(values), JsonPathSelector::UnsignedIndex(index)) => {
            usize::try_from(*index)
                .ok()
                .and_then(|index| values.get(index))
        }
        _ => None,
    }
}

fn signed_array_index(index: i64, len: usize) -> Option<usize> {
    if index >= 0 {
        usize::try_from(index).ok().filter(|index| *index < len)
    } else {
        let offset = usize::try_from(index.unsigned_abs()).ok()?;
        (offset <= len).then_some(len - offset)
    }
}

impl TryFrom<SerdeJsonValue> for JsonValue {
    type Error = CoreError;

    fn try_from(value: SerdeJsonValue) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<JsonValue> for SerdeJsonValue {
    fn from(value: JsonValue) -> Self {
        value.as_serde().clone()
    }
}

impl Serialize for JsonValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            self.as_serde().serialize(serializer)
        } else {
            serializer.serialize_str(&self.to_canonical_string())
        }
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = if deserializer.is_human_readable() {
            SerdeJsonValue::deserialize(deserializer)?
        } else {
            let value = String::deserialize(deserializer)?;
            serde_json::from_str(&value).map_err(serde::de::Error::custom)?
        };
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn validate_json_value(value: &SerdeJsonValue) -> CoreResult<()> {
    match value {
        SerdeJsonValue::Null | SerdeJsonValue::Bool(_) | SerdeJsonValue::Number(_) => Ok(()),
        SerdeJsonValue::String(value) => {
            validate_json_string_len(value.len())?;
            Ok(())
        }
        SerdeJsonValue::Array(values) => {
            ensure_json_container_len(values.len())?;
            for value in values {
                validate_json_value(value)?;
            }
            Ok(())
        }
        SerdeJsonValue::Object(values) => {
            ensure_json_container_len(values.len())?;
            for (key, value) in values {
                validate_json_string_len(key.len())?;
                validate_json_value(value)?;
            }
            Ok(())
        }
    }
}

fn validate_json_string_len(len: usize) -> CoreResult<()> {
    if len > MAX_DB_STRING_BYTES {
        return Err(CoreError::StringTooLong {
            got: len,
            max: u32::MAX,
        });
    }
    Ok(())
}

fn ensure_json_container_len(len: usize) -> CoreResult<()> {
    if len > u32::MAX as usize {
        return Err(CoreError::ConstructedValueTooLarge {
            got: len,
            max: u32::MAX,
        });
    }
    Ok(())
}

fn json_contains_value(target: &SerdeJsonValue, candidate: &SerdeJsonValue) -> bool {
    match (target, candidate) {
        (SerdeJsonValue::Object(target), SerdeJsonValue::Object(candidate)) => {
            candidate.iter().all(|(key, value)| {
                target
                    .get(key)
                    .is_some_and(|found| json_contains_value(found, value))
            })
        }
        (SerdeJsonValue::Array(target), SerdeJsonValue::Array(candidate)) => candidate
            .iter()
            .all(|value| target.iter().any(|found| json_contains_value(found, value))),
        (SerdeJsonValue::Array(target), candidate) => target
            .iter()
            .any(|found| json_contains_value(found, candidate)),
        _ => target == candidate,
    }
}

fn merge_patch_value(target: &mut SerdeJsonValue, patch: &SerdeJsonValue) {
    let SerdeJsonValue::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };

    if !target.is_object() {
        *target = SerdeJsonValue::Object(SerdeJsonMap::new());
    }
    let target = target
        .as_object_mut()
        .expect("target was normalized to object");

    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            let entry = target.entry(key.clone()).or_insert(SerdeJsonValue::Null);
            merge_patch_value(entry, value);
        }
    }
}

fn write_json_canonical(value: &SerdeJsonValue, output: &mut String) {
    match value {
        SerdeJsonValue::Null => output.push_str("null"),
        SerdeJsonValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        SerdeJsonValue::Number(value) => output.push_str(&value.to_string()),
        SerdeJsonValue::String(value) => {
            output.push_str(&serde_json::to_string(value).expect("JSON string rendering succeeds"));
        }
        SerdeJsonValue::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_json_canonical(value, output);
            }
            output.push(']');
        }
        SerdeJsonValue::Object(values) => {
            output.push('{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|lhs, rhs| lhs.0.cmp(rhs.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).expect("JSON key rendering succeeds"));
                output.push(':');
                write_json_canonical(value, output);
            }
            output.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JsonPathSelector, JsonValue};
    use crate::db_string;

    #[test]
    fn human_readable_serde_preserves_json_shape() {
        let value = JsonValue::new(serde_json::json!({"b": [2, true], "a": null})).unwrap();
        let encoded = serde_json::to_value(&value).expect("JSON value serializes");
        assert_eq!(encoded, serde_json::json!({"a": null, "b": [2, true]}));
        let decoded: JsonValue = serde_json::from_value(encoded).expect("JSON value deserializes");
        assert_eq!(decoded, value);
    }

    #[test]
    fn contains_matches_nested_subset_and_array_membership() {
        let target = JsonValue::new(serde_json::json!({
            "memory": {"kind": "episodic", "score": 7},
            "tags": ["agent", "graph", {"scope": "current"}]
        }))
        .unwrap();

        assert!(target.contains(
            &JsonValue::new(serde_json::json!({"memory": {"kind": "episodic"}})).unwrap()
        ));
        assert!(target.contains(&JsonValue::new(serde_json::json!({"tags": "graph"})).unwrap()));
        assert!(target.contains(
            &JsonValue::new(serde_json::json!({"tags": [{"scope": "current"}, "agent"]})).unwrap()
        ));
        assert!(!target.contains(
            &JsonValue::new(serde_json::json!({"memory": {"kind": "semantic"}})).unwrap()
        ));
    }

    #[test]
    fn path_exists_matches_object_keys_and_array_indexes() {
        let target = JsonValue::new(serde_json::json!({
            "memory": {"facts": [{"title": "old"}, {"title": "current"}]}
        }))
        .unwrap();
        let path = [
            JsonPathSelector::Key(db_string("memory").unwrap()),
            JsonPathSelector::Key(db_string("facts").unwrap()),
            JsonPathSelector::Index(1),
            JsonPathSelector::Key(db_string("title").unwrap()),
        ];

        assert!(target.path_exists(&path));
        assert!(target.path_exists(&[
            JsonPathSelector::Key(db_string("memory").unwrap()),
            JsonPathSelector::Key(db_string("facts").unwrap()),
            JsonPathSelector::Index(-1),
            JsonPathSelector::Key(db_string("title").unwrap()),
        ]));
        assert!(!target.path_exists(&[
            JsonPathSelector::Key(db_string("memory").unwrap()),
            JsonPathSelector::Key(db_string("facts").unwrap()),
            JsonPathSelector::UnsignedIndex(9),
        ]));
    }

    #[test]
    fn merge_patch_matches_rfc7396_core_cases() {
        for (target, patch, expected) in [
            (r#"{"a":"b"}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
            (r#"{"a":"b"}"#, r#"{"b":"c"}"#, r#"{"a":"b","b":"c"}"#),
            (r#"{"a":"b"}"#, r#"{"a":null}"#, r#"{}"#),
            (r#"{"a":"b","b":"c"}"#, r#"{"a":null}"#, r#"{"b":"c"}"#),
            (r#"{"a":["b"]}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
            (r#"{"a":"c"}"#, r#"{"a":["b"]}"#, r#"{"a":["b"]}"#),
            (
                r#"{"a":{"b":"c"}}"#,
                r#"{"a":{"b":"d","c":null}}"#,
                r#"{"a":{"b":"d"}}"#,
            ),
            (r#"{"a":[{"b":"c"}]}"#, r#"{"a":[1]}"#, r#"{"a":[1]}"#),
            (r#"["a","b"]"#, r#"["c","d"]"#, r#"["c","d"]"#),
            (r#"{"a":"b"}"#, r#"["c"]"#, r#"["c"]"#),
            (r#"{"a":"foo"}"#, r#"null"#, r#"null"#),
            (r#"{"a":"foo"}"#, r#""bar""#, r#""bar""#),
            (r#"{"e":null}"#, r#"{"a":1}"#, r#"{"a":1,"e":null}"#),
        ] {
            let target = JsonValue::parse_str(target).expect("target JSON parses");
            let patch = JsonValue::parse_str(patch).expect("patch JSON parses");
            let expected = JsonValue::parse_str(expected).expect("expected JSON parses");

            assert_eq!(
                target.merge_patch(&patch).expect("merge patch succeeds"),
                expected
            );
        }
    }
}
