//! The canonical field listing of an effective configuration: every setting as a sorted
//! `dotted.key = value` line, derived from the serialized structs so a new field appears in it
//! without anyone remembering to list it. Tests compare the listing of a file that sets every
//! key with that file's own keys, so a field read from the environment or silently defaulted
//! shows as a difference. Sections another owner validates are listed with every string value
//! that is not a `_file` path redacted, since this module cannot know what is secret there.

use toml::{Table, Value};

use super::{ConfigError, ServerConfig};

const REDACTED: &str = "<redacted>";
const OWNED_SECTIONS: [&str; 3] = ["objects", "identity", "payments"];

impl ServerConfig {
    pub fn listing(&self) -> Result<Vec<String>, ConfigError> {
        let value = Value::try_from(self).map_err(|error| ConfigError::ListingFailed {
            message: error.to_string(),
        })?;
        let Value::Table(mut table) = value else {
            return Err(ConfigError::ListingFailed {
                message: "the configuration is not a table".to_string(),
            });
        };
        redact_owned_sections(&mut table);
        Ok(flatten(&table))
    }
}

/// Replaces every string that is not a `_file` path with a marker, in place.
fn redact(table: &mut Table) {
    for (key, value) in table.iter_mut() {
        redact_value(key, value);
    }
}

fn redact_value(key: &str, value: &mut Value) {
    match value {
        Value::String(text) if !key.ends_with("_file") => *text = REDACTED.to_string(),
        Value::Table(nested) => redact(nested),
        Value::Array(items) => items.iter_mut().for_each(|item| redact_value(key, item)),
        _ => {}
    }
}

/// Flattens nested tables into sorted `dotted.key = value` lines.
pub fn flatten(table: &Table) -> Vec<String> {
    let mut lines = Vec::new();
    collect(table, "", &mut lines);
    lines.sort();
    lines
}

fn collect(table: &Table, prefix: &str, lines: &mut Vec<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Table(nested) => collect(nested, &path, lines),
            leaf => lines.push(format!("{path} = {}", render(leaf))),
        }
    }
}

/// A value on one line; tables inside arrays render inline with their keys in order.
fn render(value: &Value) -> String {
    match value {
        Value::String(text) => format!("{text:?}"),
        Value::Integer(number) => number.to_string(),
        Value::Float(number) => number.to_string(),
        Value::Boolean(flag) => flag.to_string(),
        Value::Datetime(moment) => moment.to_string(),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(render).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Table(table) => {
            let rendered: Vec<String> = table
                .iter()
                .map(|(key, value)| format!("{key} = {}", render(value)))
                .collect();
            format!("{{ {} }}", rendered.join(", "))
        }
    }
}

/// Redacts the sections other owners validate; the listing and its tests share this.
pub(crate) fn redact_owned_sections(table: &mut Table) {
    for section in OWNED_SECTIONS {
        if let Some(Value::Table(owned)) = table.get_mut(section) {
            redact(owned);
        }
    }
}
