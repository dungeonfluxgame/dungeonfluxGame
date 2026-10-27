//! Bounded strict output schemas and duplicate-free JSON for the Responses profile.

use std::fmt;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// Caller-selected retention bounds, with finite implementation ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextSchemaLimits {
    pub maximum_schema_bytes: usize,
    pub maximum_depth: usize,
    pub maximum_nodes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSchemaError {
    InvalidLimits,
    InvalidName,
    InvalidJson,
    UnsupportedSchema,
    OutputMismatch,
}

/// Checked caller-owned schema. Supported forms are closed objects with every
/// property required, arrays, string/integer/number/boolean/null, and typed enums.
/// Unsupported JSON Schema keywords are refused before network dispatch.
#[derive(Clone, Eq, PartialEq)]
pub struct TextOutputSchema {
    name: String,
    schema: Value,
    limits: TextSchemaLimits,
}

impl fmt::Debug for TextOutputSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextOutputSchema")
            .finish_non_exhaustive()
    }
}

impl TextOutputSchema {
    pub fn new(name: &str, json: &[u8], limits: TextSchemaLimits) -> Result<Self, TextSchemaError> {
        if limits.maximum_schema_bytes == 0
            || limits.maximum_schema_bytes > 1_048_576
            || limits.maximum_depth == 0
            || limits.maximum_depth > 64
            || limits.maximum_nodes == 0
            || limits.maximum_nodes > 65_536
            || json.len() > limits.maximum_schema_bytes
        {
            return Err(TextSchemaError::InvalidLimits);
        }
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(TextSchemaError::InvalidName);
        }
        let schema = parse_unique_json(json, limits)?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(TextSchemaError::UnsupportedSchema);
        }
        validate_schema(&schema)?;
        Ok(Self {
            name: name.to_owned(),
            schema,
            limits,
        })
    }

    pub(crate) fn format(&self) -> Value {
        serde_json::json!({"type":"json_schema","name":self.name,"strict":true,"schema":self.schema})
    }

    /// The validated schema, for adapters that provide their own transport.
    pub fn json_schema(&self) -> &Value {
        &self.schema
    }

    /// Parse a bounded, duplicate-free output and validate it against this schema.
    pub fn parse_output(&self, bytes: &[u8]) -> Result<Value, TextSchemaError> {
        let value = parse_unique_json(bytes, self.limits)?;
        self.validate_output(&value)?;
        Ok(value)
    }

    pub(crate) fn limits(&self) -> TextSchemaLimits {
        self.limits
    }

    pub(crate) fn validate_output(&self, value: &Value) -> Result<(), TextSchemaError> {
        if matches_schema(&self.schema, value) {
            Ok(())
        } else {
            Err(TextSchemaError::OutputMismatch)
        }
    }
}

fn validate_schema(schema: &Value) -> Result<(), TextSchemaError> {
    let object = schema
        .as_object()
        .ok_or(TextSchemaError::UnsupportedSchema)?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(TextSchemaError::UnsupportedSchema)?;
    let allowed: &[&str] = match kind {
        "object" => &[
            "type",
            "properties",
            "required",
            "additionalProperties",
            "description",
        ],
        "array" => &["type", "items", "description"],
        "string" | "integer" | "number" | "boolean" | "null" => &["type", "enum", "description"],
        _ => return Err(TextSchemaError::UnsupportedSchema),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || object
            .get("description")
            .is_some_and(|value| !value.is_string())
    {
        return Err(TextSchemaError::UnsupportedSchema);
    }
    match kind {
        "object" => {
            let properties = object
                .get("properties")
                .and_then(Value::as_object)
                .ok_or(TextSchemaError::UnsupportedSchema)?;
            let required = object
                .get("required")
                .and_then(Value::as_array)
                .ok_or(TextSchemaError::UnsupportedSchema)?;
            if object.get("additionalProperties") != Some(&Value::Bool(false))
                || required.len() != properties.len()
            {
                return Err(TextSchemaError::UnsupportedSchema);
            }
            let mut keys = std::collections::BTreeSet::new();
            for key in required {
                let key = key.as_str().ok_or(TextSchemaError::UnsupportedSchema)?;
                if !properties.contains_key(key) || !keys.insert(key) {
                    return Err(TextSchemaError::UnsupportedSchema);
                }
            }
            for child in properties.values() {
                validate_schema(child)?;
            }
        }
        "array" => validate_schema(
            object
                .get("items")
                .ok_or(TextSchemaError::UnsupportedSchema)?,
        )?,
        _ => {
            if let Some(values) = object.get("enum") {
                let values = values
                    .as_array()
                    .ok_or(TextSchemaError::UnsupportedSchema)?;
                if values.is_empty() || values.iter().any(|value| !matches_kind(kind, value)) {
                    return Err(TextSchemaError::UnsupportedSchema);
                }
            }
        }
    }
    Ok(())
}

fn matches_kind(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn matches_schema(schema: &Value, value: &Value) -> bool {
    let Some(kind) = schema.get("type").and_then(Value::as_str) else {
        return false;
    };
    if !matches_kind(kind, value) {
        return false;
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array)
        && !choices.contains(value)
    {
        return false;
    }
    match kind {
        "object" => {
            let (Some(properties), Some(output)) = (
                schema.get("properties").and_then(Value::as_object),
                value.as_object(),
            ) else {
                return false;
            };
            properties.len() == output.len()
                && properties.iter().all(|(key, child)| {
                    output
                        .get(key)
                        .is_some_and(|value| matches_schema(child, value))
                })
        }
        "array" => {
            let (Some(items), Some(output)) = (schema.get("items"), value.as_array()) else {
                return false;
            };
            output.iter().all(|value| matches_schema(items, value))
        }
        _ => true,
    }
}

pub(crate) fn parse_unique_json(
    bytes: &[u8],
    limits: TextSchemaLimits,
) -> Result<Value, TextSchemaError> {
    let mut remaining = limits.maximum_nodes;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = JsonSeed {
        remaining: &mut remaining,
        depth: 0,
        maximum_depth: limits.maximum_depth,
    }
    .deserialize(&mut deserializer)
    .map_err(|_| TextSchemaError::InvalidJson)?;
    deserializer
        .end()
        .map_err(|_| TextSchemaError::InvalidJson)?;
    Ok(value)
}

struct JsonSeed<'a> {
    remaining: &'a mut usize,
    depth: usize,
    maximum_depth: usize,
}

impl<'de> DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth >= self.maximum_depth || *self.remaining == 0 {
            return Err(serde::de::Error::custom("JSON bounds"));
        }
        *self.remaining -= 1;
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for JsonSeed<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded duplicate-free JSON")
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite JSON"))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(JsonSeed {
            remaining: self.remaining,
            depth: self.depth + 1,
            maximum_depth: self.maximum_depth,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate JSON key"));
            }
            let value = map.next_value_seed(JsonSeed {
                remaining: self.remaining,
                depth: self.depth + 1,
                maximum_depth: self.maximum_depth,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
