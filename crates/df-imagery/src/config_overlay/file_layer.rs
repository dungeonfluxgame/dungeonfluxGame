//! File-layer precedence for the imagery config overlay.
//!
//! The file layer merges the new `[imagery]` spelling with the legacy
//! `[pictures]` spelling. Within this one layer contradictory aliases for the
//! same field conflict only when their values differ; a higher layer (legacy
//! env, new env, runtime) later wins silently.

use std::collections::BTreeMap;

use crate::config::{AccountHandle, BackendId, ImageConfig};
use crate::request::Purpose;

use super::parse::{
    parse_backend_id, parse_backend_section, parse_file_values, parse_global_mode,
    parse_purpose, take_count, take_flag, take_text,
};
use super::shared::{
    apply_u32_pick, apply_u64_pick, apply_usize_pick, invalid, merge_pick, note, push_ignored,
    Pick,
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
    let (values, ignored_sections) = parse_file_values(text)?;
    for section in ignored_sections {
        provenance.ignored.push(format!("file:{section}"));
    }
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
    let mut new_transforms: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
    let mut new_transforms_enabled: Option<Pick<bool>> = None;
    let mut new_scheduler: BTreeMap<&str, Pick<u64>> = BTreeMap::new();
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
            ["imagery", "backend"] => {
                let text = take_text(path, value)?;
                let Some(backend) = parse_backend_id(text, false) else {
                    return Err(invalid(path, "unknown backend"));
                };
                new_backend = Some(Pick::named(backend, OverlaySource::FileImagery, path));
            }
            ["imagery", "profile"] => {
                push_ignored(provenance, &format!("file:{path}"));
            }
            ["imagery", "job_deadline_seconds"] => {
                new_scheduler.insert(
                    "job_deadline_secs",
                    Pick::named(
                        take_count(path, value)?,
                        OverlaySource::FileImagery,
                        path,
                    ),
                );
            }
            ["imagery", "backends", backend_name, key] => {
                let Some(backend) = parse_backend_section(backend_name) else {
                    return Err(OverlayError::UnknownField {
                        field: path.clone(),
                    });
                };
                match *key {
                    "enabled" => {
                        new_backend_enabled[backend.index()] = Some(Pick::named(
                            take_flag(path, value)?,
                            OverlaySource::FileImagery,
                            path,
                        ));
                    }
                    "account" => {
                        let text = take_text(path, value)?;
                        let account =
                            AccountHandle::new(text).map_err(|error| match error {
                                crate::config::ConfigError::InvalidField { reason, .. } => {
                                    invalid(path, reason)
                                }
                            })?;
                        new_backend_account[backend.index()] = Some(Pick::named(
                            account,
                            OverlaySource::FileImagery,
                            path,
                        ));
                    }
                    "max_concurrent_jobs" => {
                        new_backend_concurrency[backend.index()] = Some(Pick::named(
                            take_count(path, value)?,
                            OverlaySource::FileImagery,
                            path,
                        ));
                    }
                    "executable" => push_ignored(provenance, &format!("file:{path}")),
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
                        new_purpose_backend[purpose.index()] = Some(Pick::named(
                            backend,
                            OverlaySource::FileImagery,
                            path,
                        ));
                    }
                    "quality" | "minimum_refresh_interval_seconds" => {
                        push_ignored(provenance, &format!("file:{path}"));
                    }
                    _ => {
                        return Err(OverlayError::UnknownField {
                            field: path.clone(),
                        });
                    }
                }
            }
            ["imagery", "transforms", key @ ("max_concurrency" | "max_decoded_bytes"
            | "max_output_bytes" | "queue_capacity" | "deadline_secs")] => {
                new_transforms.insert(
                    key,
                    Pick::named(
                        take_count(path, value)?,
                        OverlaySource::FileImagery,
                        path,
                    ),
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
            ["imagery", "scheduler", key @ ("foreground_limit" | "presence_limit"
            | "background_limit" | "max_pending_jobs" | "max_pending_bytes" | "queue_age_secs"
            | "job_deadline_secs" | "stage_deadline_secs")] => {
                new_scheduler.insert(
                    key,
                    Pick::named(
                        take_count(path, value)?,
                        OverlaySource::FileImagery,
                        path,
                    ),
                );
            }
            ["imagery", "scheduler", _] => {
                return Err(OverlayError::UnknownField {
                    field: path.clone(),
                });
            }
            ["imagery", "request_limits", key @ ("max_prompt_chars" | "max_references"
            | "max_transform_steps" | "max_composite_layers")] => {
                new_request.insert(
                    key,
                    Pick::named(
                        take_count(path, value)?,
                        OverlaySource::FileImagery,
                        path,
                    ),
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
                push_ignored(provenance, &format!("file:{path}"));
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
        leg_pictures.clone(),
    )?;
    apply_file_purpose(
        config,
        provenance,
        Purpose::CreatorPortrait,
        new_purpose_enabled[Purpose::CreatorPortrait.index()].take(),
        leg_creator.clone(),
    )?;
    for index in 0..Purpose::COUNT {
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
            new_purpose_enabled[index].take(),
            leg_pictures.clone(),
        )?;
    }
    // Remaining purposes have no legacy fan-out; apply explicit toggles directly.
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if master_gated_purpose(purpose) || purpose == Purpose::CreatorPortrait {
            continue;
        }
        apply_file_purpose(
            config,
            provenance,
            purpose,
            new_purpose_enabled[index].take(),
            None,
        )?;
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
    }
    for index in 0..BackendId::COUNT {
        let label = super::shared::backend_label_from_index(index);
        if let Some(pick) = new_backend_enabled[index].take() {
            config.backends[index].enabled = pick.value;
            note(provenance, &format!("backends.{label}.enabled"), pick.source);
        }
        if let Some(pick) = new_backend_account[index].take() {
            config.backends[index].account = pick.value;
            note(provenance, &format!("backends.{label}.account"), pick.source);
        }
        apply_u32_pick(
            new_backend_concurrency[index].take(),
            &format!("backends.{label}.max_concurrent_jobs"),
            &mut config.backends[index].max_concurrent_jobs,
            provenance,
        )?;
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
    apply_u64_pick(
        new_scheduler.remove("job_deadline_secs"),
        "scheduler.job_deadline_secs",
        &mut config.scheduler.job_deadline_secs,
        provenance,
    )?;
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
