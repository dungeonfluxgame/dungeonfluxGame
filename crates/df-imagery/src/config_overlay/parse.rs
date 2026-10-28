//! Supplied file-text parsing for the imagery config overlay.
//!
//! The file layer reads one operator-supplied TOML-ish text. Bounds below are
//! allocation guards, not product ceilings: they exceed every valid
//! configuration shape (a few dozen recognized paths, short names, small
//! scalars), so a larger input is refused before any map or string grows.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::BackendId;
use crate::request::Purpose;

use super::shared::invalid;
use super::OverlayError;

/// A file larger than this is refused before line splitting.
const MAXIMUM_FILE_TEXT_BYTES: usize = 1_048_576;
/// More distinct entries than this cannot be valid: the overlay tracks about
/// eighty canonical fields plus a small ignored set, so anything larger holds
/// duplicates or unknown paths.
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

pub(super) fn is_ignored_section(section: &str) -> bool {
    section == "imagery.prefetch"
        || section == "imagery.costs"
        || section == "imagery.transform_profiles"
        || section.starts_with("imagery.transform_profiles.")
}

pub(super) fn is_owned_section(section: &str) -> bool {
    section == "pictures" || section == "imagery" || section.starts_with("imagery.")
}

pub(super) fn parse_file_values(
    text: &str,
) -> Result<(BTreeMap<String, RawValue>, Vec<String>), OverlayError> {
    if text.len() > MAXIMUM_FILE_TEXT_BYTES {
        return Err(invalid("imagery", "file text exceeds bound"));
    }
    let mut values = BTreeMap::new();
    let mut ignored: Vec<String> = Vec::new();
    let mut seen_ignored: BTreeSet<String> = BTreeSet::new();
    let mut section = String::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let Some(inner) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                return Err(invalid("imagery", "malformed section header"));
            };
            section = inner.trim().to_owned();
            if section.is_empty() {
                return Err(invalid("imagery", "malformed section header"));
            }
            if section.len() > MAXIMUM_FILE_KEY_CHARS {
                return Err(invalid("imagery", "malformed section header"));
            }
            if is_ignored_section(&section) && seen_ignored.insert(section.clone()) {
                ignored.push(section.clone());
            }
            continue;
        }
        if section.is_empty() {
            return Err(invalid("imagery", "expected a section header"));
        }
        if !is_owned_section(&section) || is_ignored_section(&section) {
            continue;
        }
        if values.len() >= MAXIMUM_FILE_ENTRIES {
            return Err(invalid("imagery", "too many entries"));
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            return Err(invalid(
                &format!("file:{section}"),
                "expected key = value",
            ));
        };
        let key = raw_key.trim();
        if key.is_empty()
            || key.len() > MAXIMUM_FILE_KEY_CHARS
            || key.contains('.')
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(invalid(&format!("file:{section}"), "malformed key"));
        }
        let full = format!("{section}.{key}");
        if full.len() > MAXIMUM_FILE_KEY_CHARS {
            return Err(invalid(&format!("file:{section}"), "malformed key"));
        }
        let value = parse_raw_value(&full, raw_value)?;
        if values.insert(full.clone(), value).is_some() {
            return Err(OverlayError::DuplicateField { field: full });
        }
    }
    Ok((values, ignored))
}

fn parse_raw_value(full: &str, raw: &str) -> Result<RawValue, OverlayError> {
    let text = raw.trim();
    if text.starts_with('"') {
        let mut out = String::new();
        // Byte 0 is the ASCII quote checked above, so slicing at 1 keeps a boundary.
        let mut chars = text[1..].chars();
        loop {
            let Some(next) = chars.next() else {
                return Err(invalid(full, "unterminated string"));
            };
            match next {
                '"' => break,
                '\\' => {
                    let Some(escaped) = chars.next() else {
                        return Err(invalid(full, "unterminated string"));
                    };
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        _ => return Err(invalid(full, "unsupported string escape")),
                    }
                }
                _ => out.push(next),
            }
            if out.len() > MAXIMUM_TEXT_VALUE_BYTES {
                return Err(invalid(full, "string exceeds bound"));
            }
        }
        let tail = chars.as_str().trim_start();
        if !tail.is_empty() && !tail.starts_with('#') {
            return Err(invalid(full, "expected end of line"));
        }
        return Ok(RawValue::Text(out));
    }
    let bare = text.split('#').next().unwrap_or("").trim();
    match bare {
        "true" => Ok(RawValue::Flag(true)),
        "false" => Ok(RawValue::Flag(false)),
        _ => parse_bare_int(full, bare),
    }
}

fn parse_bare_int(full: &str, bare: &str) -> Result<RawValue, OverlayError> {
    if bare.is_empty() {
        return Err(invalid(full, "expected a value"));
    }
    if bare.len() > 32 {
        return Err(invalid(full, "integer outside range"));
    }
    let digits = match bare.strip_prefix('-') {
        Some(_) => return Err(invalid(full, "must not be negative")),
        None => bare.strip_prefix('+').unwrap_or(bare),
    };
    if digits.is_empty()
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
        || !digits.bytes().any(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(full, "expected an integer, true, false, or string"));
    }
    let compact: String = digits
        .bytes()
        .filter(|byte| *byte != b'_')
        .map(char::from)
        .collect();
    let magnitude: u64 = compact
        .parse()
        .map_err(|_| invalid(full, "integer outside range"))?;
    Ok(RawValue::Count(magnitude))
}
