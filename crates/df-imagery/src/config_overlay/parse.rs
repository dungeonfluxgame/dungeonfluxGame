//! Supplied file-text parsing for the imagery config overlay.
//!
//! The file layer reads one operator-supplied TOML text through the pinned
//! `toml` crate, then validates only imagery-owned sections. Unrelated
//! top-level sections are ignored. Bounds below are allocation guards, not
//! product ceilings: they exceed every valid configuration shape, so a larger
//! input is refused before any map or string grows.

use std::collections::BTreeMap;

use crate::config::BackendId;
use crate::request::Purpose;

use super::OverlayError;
use super::shared::invalid;

/// A file larger than this is refused before parsing.
const MAXIMUM_FILE_TEXT_BYTES: usize = 1_048_576;
/// More distinct entries than this cannot be valid: the overlay tracks about
/// eighty canonical fields, so anything larger holds duplicates or unknown
/// paths.
const MAXIMUM_FILE_ENTRIES: usize = 512;
/// Longer dotted paths than this cannot name a recognized field.
const MAXIMUM_FILE_KEY_CHARS: usize = 128;
/// Longer quoted strings than this cannot be a valid mode, backend, or handle.
const MAXIMUM_TEXT_VALUE_BYTES: usize = 4096;

pub(super) fn parse_global_mode(text: &str) -> Option<crate::config::GlobalMode> {
    match text {
        "live" => Some(crate::config::GlobalMode::Live),
        "local-only" => Some(crate::config::GlobalMode::LocalOnly),
        "cache-only" => Some(crate::config::GlobalMode::CacheOnly),
        "off" => Some(crate::config::GlobalMode::Off),
        _ => None,
    }
}

pub(super) fn parse_backend_id(text: &str, legacy_flux: bool) -> Option<BackendId> {
    match text {
        "openai" => Some(BackendId::OpenAi),
        "fal" => Some(BackendId::Fal),
        "bfl" => Some(BackendId::Bfl),
        "codex-cli" | "codex_cli" => Some(BackendId::CodexCli),
        "flux" if legacy_flux => Some(BackendId::Fal),
        _ => None,
    }
}

pub(super) fn parse_purpose(text: &str) -> Option<Purpose> {
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index)?;
        if purpose.label() == text {
            return Some(purpose);
        }
    }
    None
}

pub(super) fn parse_backend_section(text: &str) -> Option<BackendId> {
    parse_backend_id(text, false)
}

pub(super) fn parse_legacy_switch(value: &str) -> Result<Option<bool>, ()> {
    match value {
        "" => Ok(None),
        "on" => Ok(Some(true)),
        "off" => Ok(Some(false)),
        _ => Err(()),
    }
}

pub(super) fn parse_overlay_bool(value: &str) -> Result<Option<bool>, ()> {
    match value {
        "" => Ok(None),
        "on" | "true" => Ok(Some(true)),
        "off" | "false" => Ok(Some(false)),
        _ => Err(()),
    }
}

pub(super) fn parse_count(value: &str) -> Result<Option<u64>, ()> {
    if value.is_empty() {
        return Ok(None);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    value.parse::<u64>().map(Some).map_err(|_| ())
}

pub(super) fn purpose_env_tag(purpose: Purpose) -> String {
    purpose.label().to_ascii_uppercase()
}

pub(super) fn backend_env_tag(backend: BackendId) -> String {
    backend.label().replace('-', "_").to_ascii_uppercase()
}

/// Environment tag for a profile name: uppercase with hyphens as
/// underscores, reversed back to the file spelling. Profile names keep
/// to lowercase letters, digits and hyphens, so the mapping round-trips.
/// Returns `None` when the tag cannot be a profile name.
pub(super) fn profile_name_from_tag(tag: &str) -> Option<String> {
    if tag.is_empty() || tag.len() > 64 {
        return None;
    }
    if !tag
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    Some(tag.replace('_', "-").to_ascii_lowercase())
}

pub(super) fn parse_accounting_mode(text: &str) -> Option<crate::config::AccountingMode> {
    match text {
        "record-only" => Some(crate::config::AccountingMode::RecordOnly),
        "hosted" => Some(crate::config::AccountingMode::Hosted),
        _ => None,
    }
}

pub(super) fn parse_output_format(text: &str) -> Option<crate::output::OutputFormat> {
    match text {
        "png" => Some(crate::output::OutputFormat::Png),
        "jpeg" => Some(crate::output::OutputFormat::Jpeg),
        "webp" => Some(crate::output::OutputFormat::WebP),
        _ => None,
    }
}

/// Alpha selection for a profile body: opaque, preserved alpha, or a matte
/// flattened with the separate `matte` hex color.
pub(super) enum ParsedAlpha {
    Opaque,
    Preserve,
    Matte,
}

pub(super) fn parse_alpha(text: &str) -> Option<ParsedAlpha> {
    match text {
        "opaque" => Some(ParsedAlpha::Opaque),
        "preserve-alpha" => Some(ParsedAlpha::Preserve),
        "matte" => Some(ParsedAlpha::Matte),
        _ => None,
    }
}

/// Parse a `#rrggbb` matte color; values never echo back in errors.
pub(super) fn parse_matte_hex(text: &str) -> Option<crate::output::Rgb> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| -> Option<u8> { u8::from_str_radix(hex.get(at..at + 2)?, 16).ok() };
    Some(crate::output::Rgb {
        red: channel(0)?,
        green: channel(2)?,
        blue: channel(4)?,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RawValue {
    Text(String),
    Flag(bool),
    Count(u64),
}

pub(super) fn take_flag(path: &str, value: &RawValue) -> Result<bool, OverlayError> {
    match value {
        RawValue::Flag(flag) => Ok(*flag),
        _ => Err(invalid(path, "expected true or false")),
    }
}

pub(super) fn take_text<'a>(path: &str, value: &'a RawValue) -> Result<&'a str, OverlayError> {
    match value {
        RawValue::Text(text) => Ok(text),
        _ => Err(invalid(path, "expected a quoted string")),
    }
}

pub(super) fn take_count(path: &str, value: &RawValue) -> Result<u64, OverlayError> {
    match value {
        RawValue::Count(count) => Ok(*count),
        _ => Err(invalid(path, "expected an integer")),
    }
}

pub(super) fn parse_file_values(text: &str) -> Result<BTreeMap<String, RawValue>, OverlayError> {
    if text.len() > MAXIMUM_FILE_TEXT_BYTES {
        return Err(invalid("imagery", "file text exceeds bound"));
    }
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let table: toml::Table = text
        .parse()
        .map_err(|error| map_toml_parse_error(text, &error))?;
    let mut values = BTreeMap::new();
    for (top_key, top_value) in &table {
        if top_key != "imagery" && top_key != "pictures" {
            match top_value {
                toml::Value::Table(_) | toml::Value::Array(_) => continue,
                _ => return Err(invalid("imagery", "expected a section header")),
            }
        }
        match top_value {
            toml::Value::Array(_) => {
                return Err(invalid(top_key, "invalid schema"));
            }
            toml::Value::Table(nested) => {
                flatten_owned(top_key, nested, &mut values)?;
            }
            _ => return Err(invalid(top_key, "expected a section")),
        }
    }
    if values.len() > MAXIMUM_FILE_ENTRIES {
        return Err(invalid("imagery", "too many entries"));
    }
    Ok(values)
}

fn map_toml_parse_error(text: &str, error: &toml::de::Error) -> OverlayError {
    if message_is_duplicate(error.message()) {
        if let Some(field) = duplicate_field_from_span(text, error.span()) {
            return OverlayError::DuplicateField { field };
        }
        return OverlayError::DuplicateField {
            field: "imagery".to_owned(),
        };
    }
    invalid("imagery", "invalid TOML")
}

fn message_is_duplicate(message: &str) -> bool {
    // Borrowed case-insensitive search; never copies the rendered diagnostic.
    const NEEDLE: &[u8] = b"duplicate";
    let bytes = message.as_bytes();
    if bytes.len() < NEEDLE.len() {
        return false;
    }
    bytes.windows(NEEDLE.len()).any(|window| {
        window
            .iter()
            .zip(NEEDLE.iter())
            .all(|(byte, want)| byte.to_ascii_lowercase() == *want)
    })
}

fn duplicate_field_from_span(text: &str, span: Option<std::ops::Range<usize>>) -> Option<String> {
    // Bounded borrow of the source around the reported span. Only a short
    // section/key path is ever allocated; oversized or invalid labels fall
    // back to the static `imagery` field in the caller, with no values.
    let start = span?.start;
    if start > text.len() || !text.is_char_boundary(start) {
        return None;
    }
    let line_start = text
        .get(..start)?
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = text
        .get(line_start..)?
        .find('\n')
        .map(|index| line_start + index)
        .unwrap_or(text.len());
    let line = text.get(line_start..line_end)?.trim();
    if line.is_empty() {
        return None;
    }
    if line.starts_with('[') {
        if line.starts_with("[[") {
            return None;
        }
        let close = line.find(']')?;
        let inner = line
            .get(1..close)?
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if inner.is_empty()
            || inner.len() > MAXIMUM_FILE_KEY_CHARS
            || inner.starts_with('[')
            || inner.contains('=')
        {
            return None;
        }
        return Some(inner.to_owned());
    }
    let key = line.split('=').next()?.trim();
    let key = key.trim_matches('"').trim_matches('\'').trim();
    if key.is_empty()
        || key.len() > MAXIMUM_FILE_KEY_CHARS
        || key.contains('[')
        || key.contains(']')
    {
        return None;
    }
    let prefix = text.get(..line_start)?;
    let mut section: Option<&str> = None;
    for raw in prefix.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix('[') else {
            continue;
        };
        if rest.starts_with('[') {
            continue;
        }
        let Some(close) = trimmed.find(']') else {
            continue;
        };
        let inner = trimmed.get(1..close)?.trim();
        if inner.is_empty() || inner.starts_with('[') {
            continue;
        }
        section = Some(inner);
    }
    let section = section?;
    if section.len() > MAXIMUM_FILE_KEY_CHARS
        || section.contains('=')
        || section.len() + 1 + key.len() > MAXIMUM_FILE_KEY_CHARS
    {
        return None;
    }
    Some(format!("{section}.{key}"))
}

fn flatten_owned(
    section: &str,
    table: &toml::Table,
    values: &mut BTreeMap<String, RawValue>,
) -> Result<(), OverlayError> {
    if section == "imagery.costs" || section.starts_with("imagery.costs.") {
        return Err(OverlayError::UnknownField {
            field: section.to_owned(),
        });
    }
    if section == "imagery.transform_profiles" || section.starts_with("imagery.transform_profiles.")
    {
        return Err(OverlayError::UnknownField {
            field: section.to_owned(),
        });
    }
    if section == "imagery.prefetch" || section.starts_with("imagery.prefetch.") {
        return Err(OverlayError::UnknownField {
            field: section.to_owned(),
        });
    }
    // An empty table carries no keys for the field handlers below, so an
    // unknown backend/purpose name would otherwise pass silently. Refuse it
    // here; non-empty unknown tables still fail per key in the file layer,
    // keeping the exact offending path.
    if table.is_empty() {
        if let Some(rest) = section.strip_prefix("imagery.backends.")
            && !rest.is_empty()
            && !rest.contains('.')
            && parse_backend_section(rest).is_none()
        {
            return Err(OverlayError::UnknownField {
                field: section.to_owned(),
            });
        }
        if let Some(rest) = section.strip_prefix("imagery.outputs.")
            && !rest.is_empty()
            && !rest.contains('.')
            && crate::config::OutputProfileName::new(rest).is_err()
        {
            return Err(OverlayError::UnknownField {
                field: section.to_owned(),
            });
        }
        if let Some(rest) = section.strip_prefix("imagery.purposes.")
            && !rest.is_empty()
            && !rest.contains('.')
            && parse_purpose(rest).is_none()
        {
            return Err(OverlayError::UnknownField {
                field: section.to_owned(),
            });
        }
        if section.starts_with("pictures.") {
            return Err(OverlayError::UnknownField {
                field: section.to_owned(),
            });
        }
    }
    if !is_known_table_section(section) {
        return Err(OverlayError::UnknownField {
            field: section.to_owned(),
        });
    }
    for (key, value) in table {
        let full = format!("{section}.{key}");
        if key.len() > MAXIMUM_FILE_KEY_CHARS || full.len() > MAXIMUM_FILE_KEY_CHARS {
            return Err(invalid(&format!("file:{section}"), "malformed key"));
        }
        match value {
            toml::Value::Table(nested) => {
                flatten_owned(&full, nested, values)?;
            }
            toml::Value::Array(_) => {
                return Err(invalid(&full, "invalid schema"));
            }
            toml::Value::String(text) => {
                if text.len() > MAXIMUM_TEXT_VALUE_BYTES {
                    return Err(invalid(&full, "string exceeds bound"));
                }
                insert_value(values, full, RawValue::Text(text.clone()))?;
            }
            toml::Value::Boolean(flag) => {
                insert_value(values, full, RawValue::Flag(*flag))?;
            }
            toml::Value::Integer(count) => {
                if *count < 0 {
                    return Err(invalid(&full, "must not be negative"));
                }
                let converted =
                    u64::try_from(*count).map_err(|_| invalid(&full, "integer outside range"))?;
                insert_value(values, full, RawValue::Count(converted))?;
            }
            toml::Value::Float(_) | toml::Value::Datetime(_) => {
                return Err(invalid(
                    &full,
                    "expected an integer, true, false, or string",
                ));
            }
        }
    }
    Ok(())
}

fn is_known_table_section(section: &str) -> bool {
    if section == "imagery" || section == "pictures" {
        return true;
    }
    if section == "imagery.backends"
        || section == "imagery.purposes"
        || section == "imagery.transforms"
        || section == "imagery.scheduler"
        || section == "imagery.request_limits"
        || section == "imagery.analysis.segmentation_preparation"
        || section == "imagery.analysis.segmentation"
        || section == "imagery.analysis.head_locations"
        || section == "imagery.analysis.staging_identity"
        || section == "imagery.analysis.staging_facing"
        || section == "imagery.creator"
        || section == "imagery.quota"
        || section == "imagery.analysis.preparation"
        || section == "imagery.analysis"
        || section == "imagery.analysis.classification"
        || section == "imagery.analysis.inspection"
        || section == "imagery.codex"
        || section == "imagery.outputs"
        || section == "imagery.retry"
        || section == "imagery.accounting"
        || section == "imagery.quality"
    {
        return true;
    }
    if let Some(rest) = section.strip_prefix("imagery.backends.") {
        return !rest.contains('.');
    }
    if let Some(rest) = section.strip_prefix("imagery.purposes.") {
        return !rest.contains('.');
    }
    if let Some(rest) = section.strip_prefix("imagery.outputs.") {
        return !rest.contains('.');
    }
    if let Some(rest) = section.strip_prefix("pictures.") {
        return !rest.contains('.');
    }
    false
}

fn insert_value(
    values: &mut BTreeMap<String, RawValue>,
    full: String,
    value: RawValue,
) -> Result<(), OverlayError> {
    if values.len() >= MAXIMUM_FILE_ENTRIES {
        return Err(invalid("imagery", "too many entries"));
    }
    if values.insert(full.clone(), value).is_some() {
        return Err(OverlayError::DuplicateField { field: full });
    }
    Ok(())
}

/// Named provider generation quality; historical numeric values are codec-only.
pub(super) fn parse_generation_quality(text: &str) -> Option<crate::config::GenerationQuality> {
    use crate::config::GenerationQuality;
    match text {
        "low" => Some(GenerationQuality::Low),
        "medium" => Some(GenerationQuality::Medium),
        "high" => Some(GenerationQuality::High),
        "auto" => Some(GenerationQuality::Auto),
        _ => None,
    }
}

pub(super) fn parse_provider_moderation(text: &str) -> Option<crate::request::ProviderModeration> {
    match text {
        "auto" => Some(crate::request::ProviderModeration::Auto),
        "low" => Some(crate::request::ProviderModeration::Low),
        _ => None,
    }
}
