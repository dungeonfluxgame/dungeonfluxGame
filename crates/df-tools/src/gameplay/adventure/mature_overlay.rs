//! The mature variant of the adventure tables; governing design: adventure-generator §21.
//!
//! The shipped tables are the restrained text. `mature-overlay.json` replaces named tables of
//! three of those files (the fight, victory and defeat prose, the speaking antagonists' arrival
//! and warning lines, and the NPC flirting and threat reactions) with mature ones. The overlay is
//! applied to the parsed JSON before the usual typed loading, so every replacement goes through
//! exactly the same validation, face tiling, tag agreement and length bounds as the table it
//! replaces. A replacement is never an addition: it names an existing table or antagonist, so a
//! typo is a load error instead of a silently ignored entry.
//!
//! The overlay file is deliberately not listed in `tables.json`, and the restrained variant never
//! reads it into any revision, so every restrained adventure keeps its recorded bytes. The mature
//! variant's revisions add the overlay's bytes.
use super::TablesError;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, fs, io::Read, path::Path};

pub(super) const OVERLAY_FILE: &str = "mature-overlay.json";
const SCHEMA: &str = "df-adventure-overlay/1";
const MAX_FILE: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    file: String,
    description: String,
    replaces: Replacements,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacements {
    /// Pattern tables of `prose-patterns.json`, by table name.
    #[serde(rename = "prose-patterns.json")]
    prose: BTreeMap<String, Value>,
    /// The `arrivals` and `warnings` tables of speaking antagonists, by antagonist id.
    #[serde(rename = "antagonists.json")]
    antagonists: BTreeMap<String, AntagonistLines>,
    /// Temperament tables of `npc-social.json`, by table name.
    #[serde(rename = "npc-social.json")]
    social: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AntagonistLines {
    arrivals: Option<Value>,
    warnings: Option<Value>,
}

/// The parsed overlay and its exact bytes (which the mature revisions hash).
pub(super) struct Overlay {
    bytes: Vec<u8>,
    document: Document,
}

fn invalid(path: &str, reason: &str) -> TablesError {
    TablesError::Invalid {
        file: OVERLAY_FILE.into(),
        path: path.into(),
        reason: reason.into(),
    }
}

impl Overlay {
    /// Reads and checks the overlay in `directory`. It is required: mature is the default rating.
    pub(super) fn read(directory: &Path) -> Result<Self, TablesError> {
        let mut bytes = Vec::new();
        fs::File::open(directory.join(OVERLAY_FILE))
            .and_then(|file| file.take(MAX_FILE + 1).read_to_end(&mut bytes))
            .map_err(|error| TablesError::Io {
                file: OVERLAY_FILE.into(),
                kind: error.kind(),
            })?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(invalid(".", "file exceeds one MiB"));
        }
        let document: Document =
            serde_json::from_slice(&bytes).map_err(|error| invalid(".", &error.to_string()))?;
        if document.schema != SCHEMA
            || document.file != OVERLAY_FILE.trim_end_matches(".json")
            || document.description.trim().is_empty()
        {
            return Err(invalid("schema", "unsupported schema or file marker"));
        }
        Ok(Self { bytes, document })
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// `original` with this overlay's replacements for `file` applied; files the overlay does
    /// not name come back unchanged.
    pub(super) fn merged(&self, file: &str, original: &[u8]) -> Result<Vec<u8>, TablesError> {
        let replaced = match file {
            "prose-patterns.json" => {
                self.replace_tables(file, original, &self.document.replaces.prose)
            }
            "npc-social.json" => {
                self.replace_tables(file, original, &self.document.replaces.social)
            }
            "antagonists.json" => self.replace_antagonist_lines(file, original),
            _ => return Ok(original.to_vec()),
        }?;
        serde_json::to_vec(&replaced).map_err(|_| invalid(file, "serialization"))
    }

    fn replace_tables(
        &self,
        file: &str,
        original: &[u8],
        replacements: &BTreeMap<String, Value>,
    ) -> Result<Value, TablesError> {
        let mut document = parse(file, original)?;
        let tables = document
            .get_mut("tables")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid(file, "tables"))?;
        for (name, replacement) in replacements {
            let slot = tables.get_mut(name).ok_or_else(|| {
                invalid(
                    &format!("{file}.{name}"),
                    "replaces a table that does not exist",
                )
            })?;
            *slot = replacement.clone();
        }
        Ok(document)
    }

    fn replace_antagonist_lines(&self, file: &str, original: &[u8]) -> Result<Value, TablesError> {
        let mut document = parse(file, original)?;
        let entries = document
            .pointer_mut("/tables/antagonists/entries")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid(file, "tables.antagonists.entries"))?;
        for (id, lines) in &self.document.replaces.antagonists {
            let path = format!("{file}.{id}");
            let entry = entries
                .iter_mut()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id.as_str()))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| invalid(&path, "replaces an antagonist that does not exist"))?;
            for (field, replacement) in
                [("arrivals", &lines.arrivals), ("warnings", &lines.warnings)]
            {
                let Some(replacement) = replacement else {
                    continue;
                };
                if entry.get(field).is_none_or(Value::is_null) {
                    return Err(invalid(
                        &path,
                        "replaces lines the antagonist does not have",
                    ));
                }
                entry.insert(field.into(), replacement.clone());
            }
        }
        Ok(document)
    }
}

fn parse(file: &str, bytes: &[u8]) -> Result<Value, TablesError> {
    serde_json::from_slice(bytes).map_err(|error| TablesError::Invalid {
        file: file.into(),
        path: ".".into(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn overlay(replaces: Value) -> Overlay {
        let document = json!({
            "schema": SCHEMA,
            "file": "mature-overlay",
            "description": "test",
            "replaces": replaces,
        });
        let bytes = serde_json::to_vec(&document).unwrap();
        Overlay {
            document: serde_json::from_slice(&bytes).unwrap(),
            bytes,
        }
    }

    fn none() -> Value {
        json!({"prose-patterns.json": {}, "antagonists.json": {}, "npc-social.json": {}})
    }

    #[test]
    fn a_replacement_names_an_existing_table_or_antagonist_or_the_load_is_refused() {
        let prose = br#"{"tables": {"clue": {"die": 1, "variants": []}}}"#;
        let mut replaces = none();
        replaces["prose-patterns.json"] = json!({"no_such_table": {"die": 1}});
        let error = overlay(replaces)
            .merged("prose-patterns.json", prose)
            .unwrap_err();
        assert!(error.to_string().contains("does not exist"), "{error}");

        let antagonists =
            br#"{"tables": {"antagonists": {"entries": [{"id": "wolf", "warnings": null}]}}}"#;
        let mut replaces = none();
        replaces["antagonists.json"] = json!({"no-such-creature": {"arrivals": {"die": 4}}});
        assert!(
            overlay(replaces)
                .merged("antagonists.json", antagonists)
                .is_err()
        );
        let mut replaces = none();
        replaces["antagonists.json"] = json!({"wolf": {"warnings": {"die": 4}}});
        let error = overlay(replaces)
            .merged("antagonists.json", antagonists)
            .unwrap_err();
        assert!(error.to_string().contains("does not have"), "{error}");
    }

    #[test]
    fn a_replacement_swaps_exactly_the_named_table_and_other_files_pass_through() {
        let prose = br#"{"tables": {"clue": {"die": 4}, "keep": {"die": 6}}}"#;
        let mut replaces = none();
        replaces["prose-patterns.json"] = json!({"clue": {"die": 8}});
        let overlay = overlay(replaces);

        let merged: Value =
            serde_json::from_slice(&overlay.merged("prose-patterns.json", prose).unwrap()).unwrap();

        assert_eq!(merged["tables"]["clue"], json!({"die": 8}));
        assert_eq!(merged["tables"]["keep"], json!({"die": 6}));
        assert_eq!(
            overlay.merged("hooks.json", b"not json").unwrap(),
            b"not json"
        );
    }

    #[test]
    fn the_shipped_overlay_loads() {
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables");
        let overlay = Overlay::read(&directory).unwrap();
        assert!(!overlay.bytes().is_empty());
    }
}
