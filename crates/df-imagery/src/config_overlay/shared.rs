//! Shared precedence helpers for the imagery config overlay.
//!
//! One choke point for per-field winning-source tracking. Higher layers win
//! over lower layers without conflict; within one layer contradictory new and
//! legacy spellings for the same field conflict only when their values differ.

use crate::config::BackendId;
use crate::request::Purpose;

use super::{FieldProvenance, OverlayError, OverlaySource, Provenance};

pub(super) fn invalid(field: &str, reason: &'static str) -> OverlayError {
    OverlayError::InvalidField {
        field: field.to_owned(),
        reason,
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
        for suffix in ["enabled", "backend_override"] {
            fields.push(FieldProvenance {
                field: format!("purposes.{}.{suffix}", purpose.label()),
                source: OverlaySource::Default,
            });
        }
    }
    for label in ["openai", "fal", "bfl", "codex-cli"] {
        for suffix in ["enabled", "account", "max_concurrent_jobs"] {
            fields.push(FieldProvenance {
                field: format!("backends.{label}.{suffix}"),
                source: OverlaySource::Default,
            });
        }
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
    let Some(entry) = provenance.fields.iter_mut().find(|entry| entry.field == field) else {
        return;
    };
    entry.source = source;
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
        *slot = u32::try_from(pick.value)
            .map_err(|_| invalid(&pick.name, "integer outside range"))?;
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
