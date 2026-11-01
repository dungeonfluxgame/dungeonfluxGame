//! File-layer precedence for the imagery config overlay.
//!
//! The file layer merges the new `[imagery]` spelling with the legacy
//! `[pictures]` spelling. Within this one layer contradictory aliases for the
//! same field conflict only when their values differ; a higher layer (legacy
//! env, new env, runtime) later wins silently.

use std::collections::BTreeMap;

use crate::config::{
    AccountHandle, AccountingMode, ApiEndpoint, BackendId, CliProfileRevision, CodexExecutable,
    ImageConfig, JobWorkDir, ModelName, OutputProfileName, PriceFileRef,
};
use crate::request::Purpose;

use super::parse::{
    parse_accounting_mode, parse_backend_id, parse_backend_section, parse_file_values,
    parse_global_mode, parse_purpose, take_count, take_flag, take_text,
};
use super::shared::{
    BodyField, Pick, ProfileEdit, apply_u32_pick, apply_u64_pick, apply_usize_pick, invalid,
    merge_pick, merge_profile_body, note, profile_body, push_ignored, seed_profile_body,
};
use super::{OverlayError, OverlaySource, Provenance};

/// Scene-art purposes gated by the legacy master `scene_pictures` switch.
/// Creator portraits keep their own switch; animation frames keep the video
/// dependency; local transform/import derivatives are not scene art.
fn master_gated_purpose(purpose: Purpose) -> bool {
    matches!(
        purpose,
        Purpose::CanonicalPerson
            | Purpose::CanonicalRoom
            | Purpose::ScenePlate
            | Purpose::SceneEdit
            | Purpose::Expression
            | Purpose::ItemCloseUp
            | Purpose::PartyMural
            | Purpose::CombatStill
    )
}

pub(super) fn apply_file_layer(
    text: &str,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    let values = parse_file_values(text)?;
    let mut analysis_classification = None;
    let mut analysis_inspection = None;
    let mut analysis_head_locations = None;
    let mut analysis_staging_identity = None;
    let mut analysis_staging_facing = None;
    let mut analysis_segmentation = None;
    let mut segmentation_preparation = BTreeMap::new();
    let mut analysis_preparation = BTreeMap::new();
    let mut new_mode: Option<Pick<crate::config::GlobalMode>> = None;
    let mut new_backend: Option<Pick<BackendId>> = None;
    let mut new_purpose_enabled: [Option<Pick<bool>>; Purpose::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_purpose_backend: [Option<Pick<Option<BackendId>>>; Purpose::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_enabled: [Option<Pick<bool>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_account: [Option<Pick<AccountHandle>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_concurrency: [Option<Pick<u64>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_model: [Option<Pick<ModelName>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_generation_model: [Option<Pick<ModelName>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_edit_model: [Option<Pick<ModelName>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_alpha_model: [Option<Pick<ModelName>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_backend_endpoint: [Option<Pick<ApiEndpoint>>; BackendId::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_purpose_moderation: [Option<Pick<Option<crate::request::ProviderModeration>>>;
        Purpose::COUNT] = std::array::from_fn(|_| None);
    let mut new_purpose_quality: [Option<Pick<Option<crate::config::GenerationQuality>>>;
        Purpose::COUNT] = std::array::from_fn(|_| None);
    let mut new_purpose_model: [Option<Pick<Option<ModelName>>>; Purpose::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_purpose_account: [Option<Pick<Option<AccountHandle>>>; Purpose::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_purpose_profile: [Option<Pick<Option<OutputProfileName>>>; Purpose::COUNT] =
        std::array::from_fn(|_| None);
    let mut new_codex_executable: Option<Pick<CodexExecutable>> = None;
    let mut new_codex_workdir: Option<Pick<JobWorkDir>> = None;
    let mut new_codex_cli_profile: Option<Pick<CliProfileRevision>> = None;
    let mut new_codex_agent_model: Option<Pick<ModelName>> = None;
    let mut new_codex_stdout: Option<Pick<u64>> = None;
    let mut new_codex_stderr: Option<Pick<u64>> = None;
    let mut new_codex_message: Option<Pick<u64>> = None;
    let mut creator_bfl_account = None;
    let mut quota_enabled = None;
    let mut quota_counts = BTreeMap::new();
    let mut new_default_profile: Option<Pick<OutputProfileName>> = None;
    let mut profile_edits: BTreeMap<String, ProfileEdit> = BTreeMap::new();
    let mut new_retry: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
    let mut new_accounting_mode: Option<Pick<AccountingMode>> = None;
    let mut new_accounting_price: Option<Pick<PriceFileRef>> = None;
    let mut new_jpeg_quality: Option<Pick<crate::output::LossyQuality>> = None;
    let mut new_provider_quality: Option<Pick<Option<crate::config::GenerationQuality>>> = None;
    let mut new_transforms: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
    let mut new_transforms_enabled: Option<Pick<bool>> = None;
    let mut new_scheduler: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
    let mut alias_job_deadline: Option<Pick<u64>> = None;
    let mut new_request: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
    let mut leg_pictures: Option<Pick<bool>> = None;
    let mut leg_plates: Option<Pick<bool>> = None;
    let mut leg_creator: Option<Pick<bool>> = None;
    let mut leg_provider: Option<Pick<BackendId>> = None;
    for (path, value) in &values {
        let segments: Vec<&str> = path.split('.').collect();
        match segments.as_slice() {
            ["imagery", "mode"] => {
                let text = take_text(path, value)?;
                let Some(mode) = parse_global_mode(text) else {
                    return Err(invalid(path, "unknown mode"));
                };
                new_mode = Some(Pick::named(mode, OverlaySource::FileImagery, path));
            }
            ["imagery", "backend" | "default_backend"] => {
                let text = take_text(path, value)?;
                let Some(backend) = parse_backend_id(text, false) else {
                    return Err(invalid(path, "unknown backend"));
                };
                new_backend = merge_pick(
                    "default_backend",
                    new_backend,
                    Some(Pick::named(backend, OverlaySource::FileImagery, path)),
                )?;
            }
            ["imagery", "profile"] => {
                return Err(invalid(path, "unsupported field"));
            }
            ["imagery", "job_deadline_seconds"] => {
                alias_job_deadline = Some(Pick::named(
                    take_count(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            [
                "imagery",
                "analysis",
                "preparation",
                key @ ("max_pixels" | "max_picture_bytes" | "max_request_bytes"),
            ] => {
                analysis_preparation.insert(
                    *key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            [
                "imagery",
                "analysis",
                "segmentation_preparation",
                key @ ("max_pixels" | "max_picture_bytes" | "max_request_bytes"),
            ] => {
                segmentation_preparation.insert(
                    *key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "analysis", "segmentation"]
            | ["imagery", "analysis", "segmentation", _] => {
                super::segmentation::parse_slot(
                    &mut analysis_segmentation,
                    segments.get(3).copied(),
                    path,
                    take_text(path, value)?,
                )?;
            }
            ["imagery", "analysis", stage] | ["imagery", "analysis", stage, _] => {
                let field = segments.get(3).copied();
                let raw = match (field, value) {
                    (Some("maximum_output_tokens"), super::parse::RawValue::Count(count)) => {
                        count.to_string()
                    }
                    (
                        None | Some("account" | "model" | "endpoint"),
                        super::parse::RawValue::Text(text),
                    ) => text.clone(),
                    _ => {
                        return Err(invalid(
                            path,
                            "an analysis route field is text or an unsigned integer",
                        ));
                    }
                };
                let slot = match *stage {
                    "classification" => &mut analysis_classification,
                    "inspection" => &mut analysis_inspection,
                    "head_locations" => &mut analysis_head_locations,
                    "staging_identity" => &mut analysis_staging_identity,
                    "staging_facing" => &mut analysis_staging_facing,
                    _ => {
                        return Err(OverlayError::UnknownField {
                            field: path.clone(),
                        });
                    }
                };
                super::analysis::parse_slot(slot, field, path, &raw)?;
            }
            ["imagery", "codex", "executable"] => {
                let text = take_text(path, value)?;
                let executable = CodexExecutable::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_codex_executable =
                    Some(Pick::named(executable, OverlaySource::FileImagery, path));
            }
            ["imagery", "codex", "workdir"] => {
                let text = take_text(path, value)?;
                let workdir = JobWorkDir::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_codex_workdir = Some(Pick::named(workdir, OverlaySource::FileImagery, path));
            }
            ["imagery", "codex", "cli_profile"] => {
                let text = take_text(path, value)?;
                let revision = CliProfileRevision::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_codex_cli_profile =
                    Some(Pick::named(revision, OverlaySource::FileImagery, path));
            }
            ["imagery", "codex", "agent_model"] => {
                let text = take_text(path, value)?;
                let model = ModelName::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_codex_agent_model = Some(Pick::named(model, OverlaySource::FileImagery, path));
            }
            ["imagery", "codex", "max_stdout_bytes"] => {
                new_codex_stdout = Some(Pick::named(
                    take_count(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            ["imagery", "codex", "max_stderr_bytes"] => {
                new_codex_stderr = Some(Pick::named(
                    take_count(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            ["imagery", "creator", "bfl_account"] => {
                let raw = take_text(path, value)?;
                let account = if raw == "none" {
                    None
                } else {
                    Some(AccountHandle::new(raw).map_err(|error| match error {
                        crate::config::ConfigError::InvalidField { reason, .. } => {
                            invalid(path, reason)
                        }
                    })?)
                };
                creator_bfl_account = Some(Pick::named(account, OverlaySource::FileImagery, path));
            }
            ["imagery", "quota", "enabled"] => {
                quota_enabled = Some(Pick::named(
                    take_flag(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            [
                "imagery",
                "quota",
                key @ ("cooldown_ms" | "maximum_accounts"),
            ] => {
                quota_counts.insert(
                    *key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "codex", "max_message_bytes"] => {
                new_codex_message = Some(Pick::named(
                    take_count(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            ["imagery", "codex", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["imagery", "outputs", "default_profile"] => {
                let text = take_text(path, value)?;
                let name = OutputProfileName::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_default_profile = Some(Pick::named(name, OverlaySource::FileImagery, path));
            }
            ["imagery", "outputs", name, key] => {
                let profile_name = OutputProfileName::new(name).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                match *key {
                    "generated_width_px"
                    | "generated_height_px"
                    | "width_px"
                    | "height_px"
                    | "format"
                    | "quality"
                    | "alpha"
                    | "matte" => {}
                    _ => {
                        return Err(OverlayError::UnknownField {
                            field: path.clone(),
                        });
                    }
                }
                let body_field = match value {
                    super::parse::RawValue::Count(count) => BodyField::Count(*count),
                    super::parse::RawValue::Text(text) => BodyField::Text(text.clone()),
                    super::parse::RawValue::Flag(_) => {
                        return Err(invalid(path, "expected an integer or a quoted string"));
                    }
                };
                let edit = profile_edits
                    .entry(name.to_string())
                    .or_insert_with(|| ProfileEdit {
                        name: profile_name,
                        source: OverlaySource::FileImagery,
                        fields: BTreeMap::new(),
                    });
                edit.fields.insert((*key).to_string(), body_field);
            }
            [
                "imagery",
                "retry",
                key @ ("max_attempts" | "retry_deadline_secs" | "backoff_secs"),
            ] => {
                new_retry.insert(
                    key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "retry", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["imagery", "accounting", "mode"] => {
                let text = take_text(path, value)?;
                let Some(mode) = parse_accounting_mode(text) else {
                    return Err(invalid(path, "unknown accounting mode"));
                };
                new_accounting_mode = Some(Pick::named(mode, OverlaySource::FileImagery, path));
            }
            ["imagery", "accounting", "price_file"] => {
                let text = take_text(path, value)?;
                let reference = PriceFileRef::new(text).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(path, reason)
                    }
                })?;
                new_accounting_price =
                    Some(Pick::named(reference, OverlaySource::FileImagery, path));
            }
            ["imagery", "accounting", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["imagery", "quality", "jpeg_quality"] => {
                let count = take_count(path, value)?;
                let value =
                    u8::try_from(count).map_err(|_| invalid(path, "integer outside range"))?;
                let lossy = crate::output::LossyQuality::new(value)
                    .map_err(|_| invalid(path, "a quality is 1 to 100"))?;
                new_jpeg_quality = Some(Pick::named(lossy, OverlaySource::FileImagery, path));
            }
            ["imagery", "quality", "provider_quality"] => {
                let text = take_text(path, value)?;
                let selected = if text == "none" {
                    None
                } else {
                    Some(super::parse::parse_generation_quality(text).ok_or_else(|| {
                        invalid(path, "expected low, medium, high, auto, or none")
                    })?)
                };
                new_provider_quality =
                    Some(Pick::named(selected, OverlaySource::FileImagery, path));
            }
            ["imagery", "quality", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["imagery", "backends", backend_name, key] => {
                let Some(backend) = parse_backend_section(backend_name) else {
                    return Err(OverlayError::UnknownField {
                        field: path.clone(),
                    });
                };
                match *key {
                    "enabled" => {
                        let field = format!("backends.{}.enabled", backend.label());
                        let pick =
                            Pick::named(take_flag(path, value)?, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_enabled[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_enabled[backend.index()] = merged;
                    }
                    "account" => {
                        let text = take_text(path, value)?;
                        let account = AccountHandle::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.account", backend.label());
                        let pick = Pick::named(account, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_account[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_account[backend.index()] = merged;
                    }
                    "max_concurrent_jobs" => {
                        let field = format!("backends.{}.max_concurrent_jobs", backend.label());
                        let pick =
                            Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_concurrency[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_concurrency[backend.index()] = merged;
                    }
                    "model" => {
                        let text = take_text(path, value)?;
                        let model = ModelName::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.model", backend.label());
                        let pick = Pick::named(model, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_model[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_model[backend.index()] = merged;
                    }
                    "generation_model" => {
                        let text = take_text(path, value)?;
                        let model = ModelName::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.generation_model", backend.label());
                        let pick = Pick::named(model, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_generation_model[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_generation_model[backend.index()] = merged;
                    }
                    "edit_model" => {
                        let text = take_text(path, value)?;
                        let model = ModelName::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.edit_model", backend.label());
                        let pick = Pick::named(model, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_edit_model[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_edit_model[backend.index()] = merged;
                    }
                    "alpha_model" => {
                        let text = take_text(path, value)?;
                        let model = ModelName::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.alpha_model", backend.label());
                        let pick = Pick::named(model, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_alpha_model[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_alpha_model[backend.index()] = merged;
                    }
                    "endpoint" => {
                        let text = take_text(path, value)?;
                        let endpoint = ApiEndpoint::new(text).map_err(|error| match error {
                            crate::config::ConfigError::InvalidField { reason, .. } => {
                                invalid(path, reason)
                            }
                        })?;
                        let field = format!("backends.{}.endpoint", backend.label());
                        let pick = Pick::named(endpoint, OverlaySource::FileImagery, path);
                        let merged = merge_pick(
                            &field,
                            new_backend_endpoint[backend.index()].take(),
                            Some(pick),
                        )?;
                        new_backend_endpoint[backend.index()] = merged;
                    }
                    "executable" => {
                        return Err(invalid(path, "unsupported field"));
                    }
                    _ => {
                        return Err(OverlayError::UnknownField {
                            field: path.clone(),
                        });
                    }
                }
            }
            ["imagery", "purposes", purpose_name, key] => {
                let Some(purpose) = parse_purpose(purpose_name) else {
                    return Err(OverlayError::UnknownField {
                        field: path.clone(),
                    });
                };
                match *key {
                    "enabled" => {
                        new_purpose_enabled[purpose.index()] = Some(Pick::named(
                            take_flag(path, value)?,
                            OverlaySource::FileImagery,
                            path,
                        ));
                    }
                    "backend" => {
                        let text = take_text(path, value)?;
                        let backend = if text == "none" {
                            None
                        } else {
                            let Some(backend) = parse_backend_id(text, false) else {
                                return Err(invalid(path, "unknown backend"));
                            };
                            Some(backend)
                        };
                        new_purpose_backend[purpose.index()] =
                            Some(Pick::named(backend, OverlaySource::FileImagery, path));
                    }
                    "moderation_override" => {
                        let text = take_text(path, value)?;
                        let selected = if text == "none" {
                            None
                        } else {
                            Some(
                                super::parse::parse_provider_moderation(text)
                                    .ok_or_else(|| invalid(path, "expected auto, low, or none"))?,
                            )
                        };
                        new_purpose_moderation[purpose.index()] =
                            Some(Pick::named(selected, OverlaySource::FileImagery, path));
                    }
                    "quality_override" => {
                        let text = take_text(path, value)?;
                        let selected = if text == "none" {
                            None
                        } else {
                            Some(super::parse::parse_generation_quality(text).ok_or_else(|| {
                                invalid(path, "expected low, medium, high, auto, or none")
                            })?)
                        };
                        new_purpose_quality[purpose.index()] =
                            Some(Pick::named(selected, OverlaySource::FileImagery, path));
                    }
                    "model_override" => {
                        let text = take_text(path, value)?;
                        let selected = if text == "none" {
                            None
                        } else {
                            Some(super::need_model(path, text)?)
                        };
                        new_purpose_model[purpose.index()] =
                            Some(Pick::named(selected, OverlaySource::FileImagery, path));
                    }
                    "account_override" => {
                        let text = take_text(path, value)?;
                        let selected = if text == "none" {
                            None
                        } else {
                            Some(super::need_account(path, text)?)
                        };
                        new_purpose_account[purpose.index()] =
                            Some(Pick::named(selected, OverlaySource::FileImagery, path));
                    }
                    "output_profile" => {
                        let text = take_text(path, value)?;
                        let selected = if text == "none" {
                            None
                        } else {
                            let name =
                                OutputProfileName::new(text).map_err(|error| match error {
                                    crate::config::ConfigError::InvalidField { reason, .. } => {
                                        invalid(path, reason)
                                    }
                                })?;
                            Some(name)
                        };
                        new_purpose_profile[purpose.index()] =
                            Some(Pick::named(selected, OverlaySource::FileImagery, path));
                    }
                    "quality" | "minimum_refresh_interval_seconds" => {
                        return Err(invalid(path, "unsupported field"));
                    }
                    _ => {
                        return Err(OverlayError::UnknownField {
                            field: path.clone(),
                        });
                    }
                }
            }
            [
                "imagery",
                "transforms",
                key @ ("max_concurrency" | "max_decoded_bytes" | "max_output_bytes"
                | "queue_capacity" | "deadline_secs"),
            ] => {
                new_transforms.insert(
                    key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "transforms", "enabled"] => {
                new_transforms_enabled = Some(Pick::named(
                    take_flag(path, value)?,
                    OverlaySource::FileImagery,
                    path,
                ));
            }
            ["imagery", "transforms", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            [
                "imagery",
                "scheduler",
                key @ ("foreground_limit"
                | "presence_limit"
                | "background_limit"
                | "max_pending_jobs"
                | "max_pending_bytes"
                | "queue_age_secs"
                | "job_deadline_secs"
                | "stage_deadline_secs"),
            ] => {
                new_scheduler.insert(
                    key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "scheduler", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            [
                "imagery",
                "request_limits",
                key @ ("max_prompt_chars"
                | "max_references"
                | "max_transform_steps"
                | "max_composite_layers"),
            ] => {
                new_request.insert(
                    key,
                    Pick::named(take_count(path, value)?, OverlaySource::FileImagery, path),
                );
            }
            ["imagery", "request_limits", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["pictures", "scene_pictures"] => {
                leg_pictures = Some(Pick::named(
                    take_flag(path, value)?,
                    OverlaySource::FilePictures,
                    path,
                ));
            }
            ["pictures", "scene_plates"] => {
                leg_plates = Some(Pick::named(
                    take_flag(path, value)?,
                    OverlaySource::FilePictures,
                    path,
                ));
            }
            ["pictures", "picture_provider"] => {
                let text = take_text(path, value)?;
                let Some(backend) = parse_backend_id(text, true) else {
                    return Err(invalid(path, "unknown picture provider"));
                };
                leg_provider = Some(Pick::named(backend, OverlaySource::FilePictures, path));
            }
            ["pictures", "creator_portraits"] => {
                leg_creator = Some(Pick::named(
                    take_flag(path, value)?,
                    OverlaySource::FilePictures,
                    path,
                ));
            }
            ["pictures", "creator_alternate_route"] => {
                // Alternate-route selection is creator route policy and credential
                // enablement, not a portrait backend override. The foundation
                // contract carries no such field, so acknowledge the name and
                // keep classification routing untouched.
                let _ = take_flag(path, value)?;
                push_ignored(provenance, &format!("file:{path}"));
            }
            ["pictures", "picture_quality"]
            | ["pictures", "picture_profile"]
            | ["pictures", "splat_worlds"] => {
                return Err(invalid(path, "unsupported field"));
            }
            _ => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
        }
    }
    if let Some(pick) = merge_pick("default_backend", new_backend, leg_provider)? {
        config.default_backend = pick.value;
        note(provenance, "default_backend", pick.source);
    }
    if let Some(pick) = merge_pick("mode", new_mode, None)? {
        config.mode = pick.value;
        note(provenance, "mode", pick.source);
    }
    apply_file_purpose(
        config,
        provenance,
        Purpose::ScenePlate,
        new_purpose_enabled[Purpose::ScenePlate.index()].take(),
        combine_scene_plate(&leg_pictures, &leg_plates),
    )?;
    apply_file_purpose(
        config,
        provenance,
        Purpose::SceneEdit,
        new_purpose_enabled[Purpose::SceneEdit.index()].take(),
        combine_scene_plate(&leg_pictures, &leg_plates),
    )?;
    apply_file_purpose(
        config,
        provenance,
        Purpose::CreatorPortrait,
        new_purpose_enabled[Purpose::CreatorPortrait.index()].take(),
        leg_creator.clone(),
    )?;
    for (index, selected) in new_purpose_enabled.iter_mut().enumerate() {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if !master_gated_purpose(purpose)
            || purpose == Purpose::ScenePlate
            || purpose == Purpose::SceneEdit
            || purpose == Purpose::CreatorPortrait
        {
            continue;
        }
        apply_file_purpose(
            config,
            provenance,
            purpose,
            selected.take(),
            leg_pictures.clone(),
        )?;
    }
    // Remaining purposes have no legacy fan-out; apply explicit toggles directly.
    for (index, selected) in new_purpose_enabled.iter_mut().enumerate() {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if master_gated_purpose(purpose) || purpose == Purpose::CreatorPortrait {
            continue;
        }
        apply_file_purpose(config, provenance, purpose, selected.take(), None)?;
    }
    // New per-purpose backend overrides apply directly; the legacy alternate
    // route is acknowledged above and never becomes a backend override.
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if let Some(pick) = new_purpose_backend[index].take() {
            let field = format!("purposes.{}.backend_override", purpose.label());
            config.purposes[index].backend_override = pick.value;
            note(provenance, &field, pick.source);
        }
        if let Some(pick) = new_purpose_moderation[index].take() {
            config.purposes[index].moderation_override = pick.value;
            note(
                provenance,
                &format!("purposes.{}.moderation_override", purpose.label()),
                pick.source,
            );
        }
        if let Some(pick) = new_purpose_quality[index].take() {
            config.purposes[index].quality_override = pick.value;
            note(
                provenance,
                &format!("purposes.{}.quality_override", purpose.label()),
                pick.source,
            );
        }
        if let Some(pick) = new_purpose_model[index].take() {
            config.purposes[index].model_override = pick.value;
            note(
                provenance,
                &format!("purposes.{}.model_override", purpose.label()),
                pick.source,
            );
        }
        if let Some(pick) = new_purpose_account[index].take() {
            config.purposes[index].account_override = pick.value;
            note(
                provenance,
                &format!("purposes.{}.account_override", purpose.label()),
                pick.source,
            );
        }
        if let Some(pick) = new_purpose_profile[index].take() {
            let field = format!("purposes.{}.output_profile", purpose.label());
            config.purposes[index].output_profile = pick.value;
            note(provenance, &field, pick.source);
        }
    }
    for index in 0..BackendId::COUNT {
        let label = super::shared::backend_label_from_index(index);
        if let Some(pick) = new_backend_enabled[index].take() {
            config.backends[index].enabled = pick.value;
            note(
                provenance,
                &format!("backends.{label}.enabled"),
                pick.source,
            );
        }
        if let Some(pick) = new_backend_account[index].take() {
            config.backends[index].account = pick.value;
            note(
                provenance,
                &format!("backends.{label}.account"),
                pick.source,
            );
        }
        apply_u32_pick(
            new_backend_concurrency[index].take(),
            &format!("backends.{label}.max_concurrent_jobs"),
            &mut config.backends[index].max_concurrent_jobs,
            provenance,
        )?;
        if let Some(pick) = new_backend_model[index].take() {
            config.backends[index].model = Some(pick.value);
            note(provenance, &format!("backends.{label}.model"), pick.source);
        }
        if let Some(pick) = new_backend_generation_model[index].take() {
            config.backends[index].generation_model = Some(pick.value);
            note(
                provenance,
                &format!("backends.{label}.generation_model"),
                pick.source,
            );
        }
        if let Some(pick) = new_backend_edit_model[index].take() {
            config.backends[index].edit_model = Some(pick.value);
            note(
                provenance,
                &format!("backends.{label}.edit_model"),
                pick.source,
            );
        }
        if let Some(pick) = new_backend_alpha_model[index].take() {
            config.backends[index].alpha_model = Some(pick.value);
            note(
                provenance,
                &format!("backends.{label}.alpha_model"),
                pick.source,
            );
        }
        if let Some(pick) = new_backend_endpoint[index].take() {
            config.backends[index].endpoint = Some(pick.value);
            note(
                provenance,
                &format!("backends.{label}.endpoint"),
                pick.source,
            );
        }
    }
    for (field, slot) in [
        ("max_pixels", &mut config.analysis.preparation.max_pixels),
        (
            "max_picture_bytes",
            &mut config.analysis.preparation.max_picture_bytes,
        ),
        (
            "max_request_bytes",
            &mut config.analysis.preparation.max_request_bytes,
        ),
    ] {
        apply_u64_pick(
            analysis_preparation.remove(field),
            &format!("analysis.preparation.{field}"),
            slot,
            provenance,
        )?;
    }
    for (field, slot) in [
        (
            "max_pixels",
            &mut config.analysis.segmentation_preparation.max_pixels,
        ),
        (
            "max_picture_bytes",
            &mut config.analysis.segmentation_preparation.max_picture_bytes,
        ),
        (
            "max_request_bytes",
            &mut config.analysis.segmentation_preparation.max_request_bytes,
        ),
    ] {
        apply_u64_pick(
            segmentation_preparation.remove(field),
            &format!("analysis.segmentation_preparation.{field}"),
            slot,
            provenance,
        )?;
    }
    if let Some(edit) = &analysis_segmentation {
        super::segmentation::apply_route(
            &mut config.analysis.segmentation,
            edit,
            provenance,
            |_| OverlaySource::FileImagery,
        )?;
    }
    for (stage, slot, edit) in [
        (
            "classification",
            &mut config.analysis.classification,
            &analysis_classification,
        ),
        (
            "inspection",
            &mut config.analysis.inspection,
            &analysis_inspection,
        ),
        (
            "head_locations",
            &mut config.analysis.head_locations,
            &analysis_head_locations,
        ),
        (
            "staging_identity",
            &mut config.analysis.staging_identity,
            &analysis_staging_identity,
        ),
        (
            "staging_facing",
            &mut config.analysis.staging_facing,
            &analysis_staging_facing,
        ),
    ] {
        if let Some(edit) = edit {
            super::analysis::apply_route(slot, edit, stage, provenance, |_| {
                OverlaySource::FileImagery
            })?;
        }
    }
    if let Some(pick) = new_codex_executable {
        config.codex.executable = pick.value;
        note(provenance, "codex.executable", pick.source);
    }
    if let Some(pick) = new_codex_workdir {
        config.codex.workdir = pick.value;
        note(provenance, "codex.workdir", pick.source);
    }
    if let Some(pick) = new_codex_cli_profile {
        config.codex.cli_profile = pick.value;
        note(provenance, "codex.cli_profile", pick.source);
    }
    if let Some(pick) = new_codex_stdout {
        config.codex.max_stdout_bytes = pick.value;
        note(provenance, "codex.max_stdout_bytes", pick.source);
    }
    if let Some(pick) = new_codex_stderr {
        config.codex.max_stderr_bytes = pick.value;
        note(provenance, "codex.max_stderr_bytes", pick.source);
    }
    if let Some(pick) = creator_bfl_account {
        config.creator_bfl_account = pick.value;
        note(provenance, "creator.bfl_account", pick.source);
    }
    if let Some(pick) = quota_enabled {
        config.quota.enabled = pick.value;
        note(provenance, "quota.enabled", pick.source);
    }
    apply_u64_pick(
        quota_counts.remove("cooldown_ms"),
        "quota.cooldown_ms",
        &mut config.quota.cooldown_ms,
        provenance,
    )?;
    apply_u32_pick(
        quota_counts.remove("maximum_accounts"),
        "quota.maximum_accounts",
        &mut config.quota.maximum_accounts,
        provenance,
    )?;
    if let Some(pick) = new_codex_message {
        config.codex.max_message_bytes = pick.value;
        note(provenance, "codex.max_message_bytes", pick.source);
    }
    if let Some(pick) = new_codex_agent_model {
        config.codex.agent_model = Some(pick.value);
        note(provenance, "codex.agent_model", pick.source);
    }
    if let Some(pick) = new_default_profile {
        config.default_output_profile = pick.value;
        note(provenance, "outputs.default_profile", pick.source);
    }
    for (name_text, edit) in &profile_edits {
        let section = format!("imagery.outputs.{name_text}");
        let seeded = seed_profile_body(config, &edit.name)?;
        let body = profile_body(seeded, edit, &section)?;
        let fields: Vec<String> = edit.fields.keys().cloned().collect();
        merge_profile_body(
            config,
            provenance,
            edit.name.clone(),
            body,
            edit.source.clone(),
            &fields,
        );
    }
    apply_u32_pick(
        new_retry.remove("max_attempts"),
        "retry.max_attempts",
        &mut config.retry.max_attempts,
        provenance,
    )?;
    apply_u64_pick(
        new_retry.remove("retry_deadline_secs"),
        "retry.retry_deadline_secs",
        &mut config.retry.retry_deadline_secs,
        provenance,
    )?;
    apply_u64_pick(
        new_retry.remove("backoff_secs"),
        "retry.backoff_secs",
        &mut config.retry.backoff_secs,
        provenance,
    )?;
    if let Some(pick) = new_accounting_mode {
        config.accounting.mode = pick.value;
        note(provenance, "accounting.mode", pick.source);
    }
    if let Some(pick) = new_accounting_price {
        config.accounting.price_file = Some(pick.value);
        note(provenance, "accounting.price_file", pick.source);
    }
    if let Some(pick) = new_jpeg_quality {
        config.quality.jpeg_quality = pick.value;
        note(provenance, "quality.jpeg_quality", pick.source);
    }
    if let Some(pick) = new_provider_quality {
        config.quality.provider_quality = pick.value;
        note(provenance, "quality.provider_quality", pick.source);
    }
    if let Some(pick) = new_transforms_enabled {
        config.transforms.enabled = pick.value;
        note(provenance, "transforms.enabled", pick.source);
    }
    apply_u32_pick(
        new_transforms.remove("max_concurrency"),
        "transforms.max_concurrency",
        &mut config.transforms.max_concurrency,
        provenance,
    )?;
    apply_u64_pick(
        new_transforms.remove("max_decoded_bytes"),
        "transforms.max_decoded_bytes",
        &mut config.transforms.max_decoded_bytes,
        provenance,
    )?;
    apply_u64_pick(
        new_transforms.remove("max_output_bytes"),
        "transforms.max_output_bytes",
        &mut config.transforms.max_output_bytes,
        provenance,
    )?;
    apply_u32_pick(
        new_transforms.remove("queue_capacity"),
        "transforms.queue_capacity",
        &mut config.transforms.queue_capacity,
        provenance,
    )?;
    apply_u64_pick(
        new_transforms.remove("deadline_secs"),
        "transforms.deadline_secs",
        &mut config.transforms.deadline_secs,
        provenance,
    )?;
    apply_u32_pick(
        new_scheduler.remove("foreground_limit"),
        "scheduler.foreground_limit",
        &mut config.scheduler.foreground_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_scheduler.remove("presence_limit"),
        "scheduler.presence_limit",
        &mut config.scheduler.presence_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_scheduler.remove("background_limit"),
        "scheduler.background_limit",
        &mut config.scheduler.background_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_scheduler.remove("max_pending_jobs"),
        "scheduler.max_pending_jobs",
        &mut config.scheduler.max_pending_jobs,
        provenance,
    )?;
    apply_u64_pick(
        new_scheduler.remove("max_pending_bytes"),
        "scheduler.max_pending_bytes",
        &mut config.scheduler.max_pending_bytes,
        provenance,
    )?;
    apply_u64_pick(
        new_scheduler.remove("queue_age_secs"),
        "scheduler.queue_age_secs",
        &mut config.scheduler.queue_age_secs,
        provenance,
    )?;
    let canonical_deadline = new_scheduler.remove("job_deadline_secs");
    if let Some(pick) = merge_deadline_alias(alias_job_deadline, canonical_deadline)? {
        apply_u64_pick(
            Some(pick),
            "scheduler.job_deadline_secs",
            &mut config.scheduler.job_deadline_secs,
            provenance,
        )?;
    }
    apply_u64_pick(
        new_scheduler.remove("stage_deadline_secs"),
        "scheduler.stage_deadline_secs",
        &mut config.scheduler.stage_deadline_secs,
        provenance,
    )?;
    apply_usize_pick(
        new_request.remove("max_prompt_chars"),
        "request_limits.max_prompt_chars",
        &mut config.request_limits.max_prompt_chars,
        provenance,
    )?;
    apply_usize_pick(
        new_request.remove("max_references"),
        "request_limits.max_references",
        &mut config.request_limits.max_references,
        provenance,
    )?;
    apply_usize_pick(
        new_request.remove("max_transform_steps"),
        "request_limits.max_transform_steps",
        &mut config.request_limits.max_transform_steps,
        provenance,
    )?;
    apply_usize_pick(
        new_request.remove("max_composite_layers"),
        "request_limits.max_composite_layers",
        &mut config.request_limits.max_composite_layers,
        provenance,
    )?;
    Ok(())
}

fn combine_scene_plate(
    pictures: &Option<Pick<bool>>,
    plates: &Option<Pick<bool>>,
) -> Option<Pick<bool>> {
    match (pictures, plates) {
        (Some(pictures_pick), Some(plates_pick)) => {
            let value = pictures_pick.value && plates_pick.value;
            let source = if pictures_pick.value {
                plates_pick
            } else {
                pictures_pick
            };
            Some(Pick::named(
                value,
                OverlaySource::FilePictures,
                &source.name,
            ))
        }
        (Some(pick), None) | (None, Some(pick)) => Some(Pick::named(
            pick.value,
            OverlaySource::FilePictures,
            &pick.name,
        )),
        (None, None) => None,
    }
}

fn merge_deadline_alias(
    alias: Option<Pick<u64>>,
    canonical: Option<Pick<u64>>,
) -> Result<Option<Pick<u64>>, OverlayError> {
    match (alias, canonical) {
        (Some(alias_pick), Some(canonical_pick)) => {
            if alias_pick.value == canonical_pick.value {
                Ok(Some(canonical_pick))
            } else {
                Err(OverlayError::AliasConflict {
                    field: "scheduler.job_deadline_secs".to_owned(),
                    first: alias_pick.name,
                    second: canonical_pick.name,
                })
            }
        }
        (Some(pick), None) | (None, Some(pick)) => Ok(Some(pick)),
        (None, None) => Ok(None),
    }
}

fn apply_file_purpose(
    config: &mut ImageConfig,
    provenance: &mut Provenance,
    purpose: Purpose,
    updated: Option<Pick<bool>>,
    legacy: Option<Pick<bool>>,
) -> Result<(), OverlayError> {
    let field = format!("purposes.{}.enabled", purpose.label());
    if let Some(pick) = merge_pick(&field, updated, legacy)? {
        config.purposes[purpose.index()].enabled = pick.value;
        note(provenance, &field, pick.source);
    }
    Ok(())
}
