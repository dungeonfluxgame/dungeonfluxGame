//! Shared precedence helpers for the imagery config overlay.
//!
//! One choke point for per-field winning-source tracking. Higher layers win
//! over lower layers without conflict; within one layer contradictory new and
//! legacy spellings for the same field conflict only when their values differ.

use std::collections::BTreeMap;

use crate::config::{
    BackendId, ConfigError, ImageConfig, NamedOutputProfile, OutputProfileName,
    default_profile_body,
};
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputProfile, RasterOutputSpec,
};
use crate::request::Purpose;

use super::parse::{parse_alpha, parse_matte_hex, parse_output_format};
use super::{FieldProvenance, OverlayError, OverlaySource, Provenance};

pub(super) fn invalid(field: &str, reason: &'static str) -> OverlayError {
    OverlayError::InvalidField {
        field: field.to_owned(),
        reason,
    }
}

/// The overlay form of a snapshot validation refusal: same field, same reason.
pub(crate) fn overlay_field_error(error: ConfigError) -> OverlayError {
    match error {
        ConfigError::InvalidField { field, reason } => invalid(field, reason),
    }
}

/// One candidate value with its winning-layer source and display name.
#[derive(Clone, Debug)]
pub(super) struct Pick<T> {
    pub(super) value: T,
    pub(super) source: OverlaySource,
    pub(super) name: String,
}

impl<T> Pick<T> {
    pub(super) fn named(value: T, source: OverlaySource, name: &str) -> Self {
        Self {
            value,
            source,
            name: name.to_owned(),
        }
    }
}

pub(super) fn merge_pick<T>(
    field: &str,
    updated: Option<Pick<T>>,
    legacy: Option<Pick<T>>,
) -> Result<Option<Pick<T>>, OverlayError>
where
    T: PartialEq,
{
    match (updated, legacy) {
        (Some(updated_pick), Some(legacy_pick)) => {
            if updated_pick.value == legacy_pick.value {
                Ok(Some(updated_pick))
            } else {
                Err(OverlayError::AliasConflict {
                    field: field.to_owned(),
                    first: updated_pick.name,
                    second: legacy_pick.name,
                })
            }
        }
        (Some(pick), None) | (None, Some(pick)) => Ok(Some(pick)),
        (None, None) => Ok(None),
    }
}

pub(super) fn backend_label_from_index(index: usize) -> &'static str {
    match index {
        0 => BackendId::OpenAi.label(),
        1 => BackendId::Fal.label(),
        2 => BackendId::Bfl.label(),
        _ => BackendId::CodexCli.label(),
    }
}

pub(super) fn tracked_fields() -> Vec<FieldProvenance> {
    let mut fields = vec![
        FieldProvenance {
            field: "mode".to_owned(),
            source: OverlaySource::Default,
        },
        FieldProvenance {
            field: "default_backend".to_owned(),
            source: OverlaySource::Default,
        },
    ];
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        for suffix in [
            "enabled",
            "backend_override",
            "account_override",
            "output_profile",
            "quality_override",
            "moderation_override",
            "model_override",
        ] {
            fields.push(FieldProvenance {
                field: format!("purposes.{}.{suffix}", purpose.label()),
                source: OverlaySource::Default,
            });
        }
    }
    for field in ["max_pixels", "max_picture_bytes", "max_request_bytes"] {
        fields.push(FieldProvenance {
            field: format!("analysis.preparation.{field}"),
            source: OverlaySource::Default,
        });
    }
    for stage in [
        "classification",
        "inspection",
        "head_locations",
        "staging_identity",
        "staging_facing",
    ] {
        for field in [
            "",
            ".account",
            ".model",
            ".endpoint",
            ".maximum_output_tokens",
        ] {
            fields.push(FieldProvenance {
                field: format!("analysis.{stage}{field}"),
                source: OverlaySource::Default,
            });
        }
    }
    for field in ["", ".provider", ".account", ".model", ".endpoint"] {
        fields.push(FieldProvenance {
            field: format!("analysis.segmentation{field}"),
            source: OverlaySource::Default,
        });
    }
    for field in ["max_pixels", "max_picture_bytes", "max_request_bytes"] {
        fields.push(FieldProvenance {
            field: format!("analysis.segmentation_preparation.{field}"),
            source: OverlaySource::Default,
        });
    }
    for label in ["openai", "fal", "bfl", "codex-cli"] {
        for suffix in [
            "enabled",
            "account",
            "max_concurrent_jobs",
            "model",
            "generation_model",
            "edit_model",
            "alpha_model",
            "endpoint",
        ] {
            fields.push(FieldProvenance {
                field: format!("backends.{label}.{suffix}"),
                source: OverlaySource::Default,
            });
        }
    }
    for name in [
        "codex.executable",
        "codex.workdir",
        "codex.cli_profile",
        "codex.agent_model",
        "codex.max_stdout_bytes",
        "codex.max_stderr_bytes",
        "codex.max_message_bytes",
        "creator.bfl_account",
        "quota.enabled",
        "quota.cooldown_ms",
        "quota.maximum_accounts",
        "outputs.default_profile",
        "retry.max_attempts",
        "retry.retry_deadline_secs",
        "retry.backoff_secs",
        "accounting.mode",
        "accounting.price_file",
        "quality.jpeg_quality",
        "quality.provider_quality",
    ] {
        fields.push(FieldProvenance {
            field: name.to_owned(),
            source: OverlaySource::Default,
        });
    }
    for name in [
        "transforms.enabled",
        "transforms.max_concurrency",
        "transforms.max_decoded_bytes",
        "transforms.max_output_bytes",
        "transforms.queue_capacity",
        "transforms.deadline_secs",
        "scheduler.foreground_limit",
        "scheduler.presence_limit",
        "scheduler.background_limit",
        "scheduler.max_pending_jobs",
        "scheduler.max_pending_bytes",
        "scheduler.queue_age_secs",
        "scheduler.job_deadline_secs",
        "scheduler.stage_deadline_secs",
        "request_limits.max_prompt_chars",
        "request_limits.max_references",
        "request_limits.max_transform_steps",
        "request_limits.max_composite_layers",
    ] {
        fields.push(FieldProvenance {
            field: name.to_owned(),
            source: OverlaySource::Default,
        });
    }
    fields
}

pub(super) fn note(provenance: &mut Provenance, field: &str, source: OverlaySource) {
    let Some(entry) = provenance
        .fields
        .iter_mut()
        .find(|entry| entry.field == field)
    else {
        return;
    };
    entry.source = source;
}

/// Record a winning source for a dynamic field such as one named output
/// profile body, adding the entry when the fixed tracked set lacks it.
pub(super) fn note_owned(provenance: &mut Provenance, field: String, source: OverlaySource) {
    if let Some(entry) = provenance
        .fields
        .iter_mut()
        .find(|entry| entry.field == field)
    {
        entry.source = source;
    } else {
        provenance.fields.push(FieldProvenance { field, source });
    }
}

pub(super) fn push_ignored(provenance: &mut Provenance, name: &str) {
    if !provenance.ignored.iter().any(|entry| entry == name) {
        provenance.ignored.push(name.to_owned());
    }
}

pub(super) fn apply_u32_pick(
    pick: Option<Pick<u64>>,
    field: &str,
    slot: &mut u32,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(pick) = pick {
        *slot =
            u32::try_from(pick.value).map_err(|_| invalid(&pick.name, "integer outside range"))?;
        note(provenance, field, pick.source);
    }
    Ok(())
}

pub(super) fn apply_u64_pick(
    pick: Option<Pick<u64>>,
    field: &str,
    slot: &mut u64,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(pick) = pick {
        *slot = pick.value;
        note(provenance, field, pick.source);
    }
    Ok(())
}

pub(super) fn apply_usize_pick(
    pick: Option<Pick<u64>>,
    field: &str,
    slot: &mut usize,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(pick) = pick {
        *slot = usize::try_from(pick.value)
            .map_err(|_| invalid(&pick.name, "integer outside range"))?;
        note(provenance, field, pick.source);
    }
    Ok(())
}

/// One profile-body field edit. Counts carry raw integers; text carries
/// the raw string for format/alpha/matte parsing.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyField {
    Count(u64),
    Text(String),
}

/// Seeded body plus edits for one named profile, merged by name over the
/// lower layer. Missing keys keep the seeded body, which is the matching
/// lower-layer profile or the built-in default body for a new name. This
/// travels inside [`crate::config_overlay::RuntimePatch`] so dotted
/// administrative paths can shape one profile body per layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileEdit {
    pub name: OutputProfileName,
    pub source: OverlaySource,
    pub fields: BTreeMap<String, BodyField>,
}

/// Build the concrete output contract for one profile edit. Missing keys
/// keep the seeded body, which is the matching lower-layer profile or the
/// built-in default body for a new name. Every error names a field path;
/// values never echo back.
pub(super) fn profile_body(
    seeded: OutputProfile,
    edit: &ProfileEdit,
    section: &str,
) -> Result<OutputProfile, OverlayError> {
    let field_path = |key: &str| format!("{section}.{key}");
    let (seeded_generated, seeded_delivered) = match seeded {
        OutputProfile::Provider {
            generated,
            delivered,
        } => (Some(generated), delivered),
        OutputProfile::Local { delivered } => (None, delivered),
    };
    let mut generated_width = seeded_generated.map(|size| size.width_px());
    let mut generated_height = seeded_generated.map(|size| size.height_px());
    let mut delivered_width = seeded_delivered.dimensions().width_px();
    let mut delivered_height = seeded_delivered.dimensions().height_px();
    let mut format = seeded_delivered.encoding().format();
    let mut quality: Option<u8> = match seeded_delivered.encoding() {
        Encoding::Jpeg { quality } => Some(quality.value()),
        Encoding::WebP { quality } => quality.map(|lossy| lossy.value()),
        Encoding::Png => None,
    };
    let mut quality_edited = false;
    let mut alpha_edit = AlphaEdit::Keep;
    let mut matte: Option<crate::output::Rgb> = match seeded_delivered.alpha() {
        AlphaHandling::Matte(rgb) => Some(rgb),
        _ => None,
    };
    let mut matte_edited = false;
    for (key, value) in &edit.fields {
        match key.as_str() {
            "generated_width_px" => {
                generated_width = take_optional_profile_count(&field_path(key), value)?;
            }
            "generated_height_px" => {
                generated_height = take_optional_profile_count(&field_path(key), value)?;
            }
            "width_px" => {
                delivered_width = take_profile_count(&field_path(key), value)?;
            }
            "height_px" => {
                delivered_height = take_profile_count(&field_path(key), value)?;
            }
            "format" => {
                let text = take_profile_text(&field_path(key), value)?;
                format = parse_output_format(text)
                    .ok_or_else(|| invalid(&field_path(key), "unknown format"))?;
            }
            "quality" => match value {
                BodyField::Count(count) => {
                    let byte = u8::try_from(*count)
                        .map_err(|_| invalid(&field_path(key), "integer outside range"))?;
                    if LossyQuality::new(byte).is_err() {
                        return Err(invalid(&field_path(key), "a quality is 1 to 100"));
                    }
                    quality = Some(byte);
                    quality_edited = true;
                }
                BodyField::Text(text) if text == "none" => {
                    quality = None;
                }
                BodyField::Text(_) => {
                    return Err(invalid(&field_path(key), "expected an integer or \"none\""));
                }
            },
            "alpha" => {
                let text = take_profile_text(&field_path(key), value)?;
                let parsed =
                    parse_alpha(text).ok_or_else(|| invalid(&field_path(key), "unknown alpha"))?;
                alpha_edit = match parsed {
                    super::parse::ParsedAlpha::Opaque => AlphaEdit::Opaque,
                    super::parse::ParsedAlpha::Preserve => AlphaEdit::Preserve,
                    super::parse::ParsedAlpha::Matte => AlphaEdit::Matte,
                };
            }
            "matte" => {
                let text = take_profile_text(&field_path(key), value)?;
                if text == "none" {
                    matte = None;
                } else {
                    matte = Some(
                        parse_matte_hex(text)
                            .ok_or_else(|| invalid(&field_path(key), "a matte is #rrggbb"))?,
                    );
                    matte_edited = true;
                }
            }
            _ => {
                return Err(OverlayError::UnknownField {
                    field: field_path(key),
                });
            }
        }
    }
    let alpha = match alpha_edit {
        AlphaEdit::Opaque => AlphaHandling::Opaque,
        AlphaEdit::Preserve => AlphaHandling::PreserveAlpha,
        AlphaEdit::Matte => match matte {
            Some(rgb) => AlphaHandling::Matte(rgb),
            None => {
                return Err(invalid(
                    &field_path("matte"),
                    "a matte alpha needs a matte color",
                ));
            }
        },
        AlphaEdit::Keep => match seeded_delivered.alpha() {
            AlphaHandling::Matte(_) => match matte {
                Some(rgb) => AlphaHandling::Matte(rgb),
                None => AlphaHandling::Opaque,
            },
            prior => {
                if matte_edited {
                    return Err(invalid(
                        &field_path("matte"),
                        "a matte color needs alpha matte",
                    ));
                }
                prior
            }
        },
    };
    if matte_edited && !matches!(alpha, AlphaHandling::Matte(_)) {
        return Err(invalid(
            &field_path("matte"),
            "a matte color needs alpha matte",
        ));
    }
    let encoding = match format {
        crate::output::OutputFormat::Jpeg => {
            let value = match quality {
                Some(value) => value,
                None => crate::output::DEFAULT_JPEG_QUALITY_0_100,
            };
            let lossy =
                LossyQuality::new(value).map_err(|_| invalid(section, "a quality is 1 to 100"))?;
            Encoding::Jpeg { quality: lossy }
        }
        crate::output::OutputFormat::WebP => {
            let lossy = match quality {
                Some(value) => Some(
                    LossyQuality::new(value)
                        .map_err(|_| invalid(section, "a quality is 1 to 100"))?,
                ),
                None => None,
            };
            Encoding::WebP { quality: lossy }
        }
        crate::output::OutputFormat::Png => {
            if quality_edited {
                return Err(invalid(
                    &field_path("quality"),
                    "a PNG profile carries no quality",
                ));
            }
            Encoding::Png
        }
    };
    let delivered_size = Dimensions::new(delivered_width, delivered_height)
        .map_err(|error| map_output_error(&field_path("width_px"), error))?;
    let delivered = RasterOutputSpec::new(delivered_size, alpha, encoding)
        .map_err(|error| map_output_error(section, error))?;
    match (generated_width, generated_height) {
        (Some(width), Some(height)) => {
            let generated = Dimensions::new(width, height)
                .map_err(|error| map_output_error(&field_path("generated_width_px"), error))?;
            Ok(OutputProfile::provider(generated, delivered))
        }
        (None, None) => Ok(OutputProfile::local(delivered)),
        _ => {
            let missing = if generated_width.is_none() {
                "generated_width_px"
            } else {
                "generated_height_px"
            };
            Err(invalid(
                &field_path(missing),
                "a provider profile names both generated dims",
            ))
        }
    }
}

/// Alpha selection requested by one profile edit.
enum AlphaEdit {
    Keep,
    Opaque,
    Preserve,
    Matte,
}

fn take_profile_count(path: &str, value: &BodyField) -> Result<u32, OverlayError> {
    match value {
        BodyField::Count(count) => {
            u32::try_from(*count).map_err(|_| invalid(path, "integer outside range"))
        }
        BodyField::Text(_) => Err(invalid(path, "expected an integer")),
    }
}

/// Optional generated dimension: an integer sets the provider side, `none`
/// drops it so the profile becomes local. Both sides dropped together make
/// a local profile; one side alone is refused below.
fn take_optional_profile_count(path: &str, value: &BodyField) -> Result<Option<u32>, OverlayError> {
    match value {
        BodyField::Count(count) => Ok(Some(
            u32::try_from(*count).map_err(|_| invalid(path, "integer outside range"))?,
        )),
        BodyField::Text(text) if text == "none" => Ok(None),
        BodyField::Text(_) => Err(invalid(path, "expected an integer or \"none\"")),
    }
}

fn take_profile_text<'a>(path: &str, value: &'a BodyField) -> Result<&'a str, OverlayError> {
    match value {
        BodyField::Text(text) => Ok(text),
        BodyField::Count(_) => Err(invalid(path, "expected a quoted string")),
    }
}

fn map_output_error(path: &str, error: crate::output::OutputError) -> OverlayError {
    use crate::output::OutputError;
    let reason = match error {
        OutputError::ZeroDimension => "a dimension is nonzero",
        OutputError::TooManyPixels { .. } => "pixels stay within the asset ceiling",
        OutputError::QualityOutOfRange { .. } => "a quality is 1 to 100",
        OutputError::AlphaWithoutMatte => "JPEG keeps opaque or matte alpha",
    };
    invalid(path, reason)
}

/// Seed body for a profile edit: the matching lower-layer profile, or the
/// built-in default body when the name is new.
pub(super) fn seed_profile_body(
    config: &ImageConfig,
    name: &OutputProfileName,
) -> Result<OutputProfile, OverlayError> {
    if let Some(entry) = config
        .output_profiles
        .iter()
        .find(|entry| entry.name == *name)
    {
        return Ok(entry.profile);
    }
    default_profile_body().map_err(overlay_field_error)
}

/// Merge one built profile body into the snapshot by name and record the
/// winning source per edited field (`outputs.<name>.<field>`). A whole-table
/// sync names no fields and records the coarser `outputs.<name>` entry.
pub(super) fn merge_profile_body(
    config: &mut ImageConfig,
    provenance: &mut Provenance,
    name: OutputProfileName,
    body: OutputProfile,
    source: OverlaySource,
    fields: &[String],
) {
    if let Some(entry) = config
        .output_profiles
        .iter_mut()
        .find(|entry| entry.name == name)
    {
        entry.profile = body;
    } else {
        config
            .output_profiles
            .push(NamedOutputProfile::new(name.clone(), body));
    }
    if fields.is_empty() {
        note_owned(provenance, format!("outputs.{}", name.as_str()), source);
    } else {
        for field in fields {
            note_owned(
                provenance,
                format!("outputs.{}.{field}", name.as_str()),
                source.clone(),
            );
        }
    }
}
