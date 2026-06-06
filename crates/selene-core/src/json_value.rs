use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as SerdeJsonValue;

use crate::{CoreError, CoreResult, db_string::MAX_DB_STRING_BYTES};

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
    use super::JsonValue;

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
}
