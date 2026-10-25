//! A JSON parser that REFUSES a repeated object key at any depth.
//!
//! Why this exists: the policy reads the body into a `serde_json::Value`, where the last of two equal keys wins, but Docker decodes
//! the same bytes with Go's `encoding/json`, which merges a repeated object into the same field. `{"HostConfig":{"Privileged":true},
//! "HostConfig":{<compliant>}}` would pass a last-wins check and still start a privileged container. So the proxy (1) refuses any body
//! with a repeated key, after JSON string unescaping (so `"Privileged"` collides with `"Privileged"`), (2) refuses trailing data,
//! and (3) forwards the CANONICAL serialisation of what it checked, never the bytes it received (`canonical`), so the daemon sees
//! exactly the value the policy saw.

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct Strict(Value);

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value without a repeated object key")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("a number is not finite"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("repeated key {key:?}")));
            }
            let Strict(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(Strict)
    }
}

/// Parse a complete JSON document: one value, nothing after it, no repeated key anywhere.
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice::<Strict>(bytes)
        .map(|Strict(value)| value)
        .map_err(|error| error.to_string())
}

/// Same for text (the `filters` query parameter is JSON too).
pub fn parse_str(text: &str) -> Result<Value, String> {
    parse(text.as_bytes())
}

/// The canonical serialisation of a checked value: compact, keys in sorted order, no duplicates possible.
pub fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}
