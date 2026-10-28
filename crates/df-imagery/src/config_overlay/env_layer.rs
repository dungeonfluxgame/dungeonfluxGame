//! Environment-layer precedence for the imagery config overlay.
//!
//! Precedence is defaults, then the file layer, then legacy launcher
//! environment, then new `DF_IMAGERY_*` environment, then the typed runtime
//! patch. A higher layer wins silently over a lower one. Names are recognized
//! before values are inspected: unrelated environment (including credentials)
//! is ignored without cloning its values. Only imagery-owned forbidden secret
//! names are refused, never a generic substring.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{AccountHandle, BackendId, ImageConfig};
use crate::request::Purpose;

use super::parse::{
    backend_env_tag, parse_backend_id, parse_count, parse_global_mode, parse_legacy_switch,
    parse_overlay_bool, purpose_env_tag,
};
use super::shared::{apply_u32_pick, apply_u64_pick, apply_usize_pick, invalid, note, push_ignored, Pick};
use super::{OverlayError, OverlaySource, Provenance};

pub(super) const LEGACY_PICTURES: &str = "DF_GAMEPLAY_IMAGERY";
pub(super) const LEGACY_PLATES: &str = "DF_GAMEPLAY_PLATES";
pub(super) const LEGACY_CREATOR: &str = "DF_GAMEPLAY_CREATOR_PORTRAITS";
pub(super) const LEGACY_PROVIDER: &str = "DF_GAMEPLAY_IMAGERY_PROVIDER";
pub(super) const LEGACY_DEADLINE: &str = "DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS";
pub(super) const LEGACY_CHARACTER_VIDEO: &str = "DF_GAMEPLAY_CHARACTER_VIDEO";
pub(super) const LEGACY_DISABLE_OPENAI: &str = "DF_GAMEPLAY_DISABLE_OPENAI_API";

pub(super) const IGNORED_ENV: &[&str] = &[
    "DF_GAMEPLAY_IMAGERY_QUALITY",
    "DF_GAMEPLAY_IMAGERY_PROFILE",
    "DF_GAMEPLAY_IMAGERY_MODEL",
    "DF_GAMEPLAY_IMAGERY_INTERVAL_SECONDS",
    "DF_GAMEPLAY_IMAGERY_CACHE",
    "DF_GAMEPLAY_PRICING",
    "DF_GAMEPLAY_CODEX_MODE",
    "DF_GAMEPLAY_CODEX_HOME",
    "DF_GAMEPLAY_CODEX_WORKDIR",
    "DF_GAMEPLAY_CODEX_EXECUTABLE",
    "DF_GAMEPLAY_CODEX_SHA256",
];

pub(super) const NEW_MODE: &str = "DF_IMAGERY_MODE";
pub(super) const NEW_BACKEND: &str = "DF_IMAGERY_BACKEND";

fn is_imagery_owned_env_name(name: &str) -> bool {
    if name == LEGACY_PICTURES
        || name == LEGACY_PLATES
        || name == LEGACY_CREATOR
        || name == LEGACY_PROVIDER
        || name == LEGACY_DEADLINE
        || name == LEGACY_CHARACTER_VIDEO
        || name == LEGACY_DISABLE_OPENAI
        || name == NEW_MODE
        || name == NEW_BACKEND
    {
        return true;
    }
    if IGNORED_ENV.contains(&name) {
        return true;
    }
    if name.starts_with("DF_IMAGERY_") || name.starts_with("DF_GAMEPLAY_IMAGERY") {
        return true;
    }
    false
}

/// Only imagery-owned names with a whole underscore-delimited secret token are
/// refused. A generic substring would misfire on ordinary words; unrelated
/// names are ignored before this check ever runs.
fn is_forbidden_secret_env_name(name: &str) -> bool {
    if !is_imagery_owned_env_name(name) {
        return false;
    }
    name.to_ascii_uppercase()
        .split('_')
        .any(|token| matches!(token, "KEY" | "SECRET" | "TOKEN" | "PASSWORD" | "CREDENTIAL"))
}

fn take_env<'a>(
    table: &'a BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    name: &str,
) -> Option<&'a str> {
    if IGNORED_ENV.contains(&name) {
        return None;
    }
    let value = table.get(name)?;
    consumed.insert(name.to_owned());
    Some(value.as_str())
}

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

pub(super) fn apply_env_layer(
    env: &[(&str, &str)],
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    // Recognize names before inspecting values. Unrelated environment is
    // skipped without cloning its value, so credentials never enter the map.
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (name, _) in env {
        if !is_imagery_owned_env_name(name) {
            continue;
        }
        if is_forbidden_secret_env_name(name) {
            return Err(OverlayError::SecretRefused {
                name: (*name).to_owned(),
            });
        }
    }
    for (name, value) in env {
        if !is_imagery_owned_env_name(name) {
            continue;
        }
        // Secret names were refused above without cloning their values; the
        // remaining owned names are safe to record.
        if table
            .insert((*name).to_owned(), (*value).to_owned())
            .is_some()
        {
            return Err(OverlayError::DuplicateField {
                field: (*name).to_owned(),
            });
        }
    }
    let mut consumed: BTreeSet<String> = BTreeSet::new();
    for name in IGNORED_ENV {
        if table.contains_key(*name) {
            consumed.insert((*name).to_owned());
            push_ignored(provenance, &format!("env:{name}"));
        }
    }
    let legacy_flag = |name: &'static str,
                       table: &BTreeMap<String, String>,
                       consumed: &mut BTreeSet<String>|
     -> Result<Option<Pick<bool>>, OverlayError> {
        let Some(raw) = take_env(table, consumed, name) else {
            return Ok(None);
        };
        let Some(flag) = parse_legacy_switch(raw).map_err(|()| invalid(name, "expected on or off"))?
        else {
            return Ok(None);
        };
        Ok(Some(Pick::named(
            flag,
            OverlaySource::LegacyEnv(name.to_owned()),
            name,
        )))
    };
    let new_flag = |name: &str,
                    table: &BTreeMap<String, String>,
                    consumed: &mut BTreeSet<String>|
     -> Result<Option<Pick<bool>>, OverlayError> {
        let Some(raw) = take_env(table, consumed, name) else {
            return Ok(None);
        };
        let parsed = parse_overlay_bool(raw)
            .map_err(|()| invalid(name, "expected on, off, true, or false"))?;
        let Some(flag) = parsed else {
            return Ok(None);
        };
        Ok(Some(Pick::named(
            flag,
            OverlaySource::NewEnv(name.to_owned()),
            name,
        )))
    };
    let new_count = |name: &str,
                     table: &BTreeMap<String, String>,
                     consumed: &mut BTreeSet<String>|
     -> Result<Option<Pick<u64>>, OverlayError> {
        let Some(raw) = take_env(table, consumed, name) else {
            return Ok(None);
        };
        let Some(count) =
            parse_count(raw).map_err(|()| invalid(name, "expected an unsigned integer"))?
        else {
            return Ok(None);
        };
        Ok(Some(Pick::named(
            count,
            OverlaySource::NewEnv(name.to_owned()),
            name,
        )))
    };
    let leg_pictures = legacy_flag(LEGACY_PICTURES, &table, &mut consumed)?;
    let leg_plates = legacy_flag(LEGACY_PLATES, &table, &mut consumed)?;
    let leg_creator = legacy_flag(LEGACY_CREATOR, &table, &mut consumed)?;
    let leg_video = legacy_flag(LEGACY_CHARACTER_VIDEO, &table, &mut consumed)?;
    let leg_disable_openai = legacy_flag(LEGACY_DISABLE_OPENAI, &table, &mut consumed)?;
    let leg_provider = match take_env(&table, &mut consumed, LEGACY_PROVIDER) {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
            let Some(backend) = parse_backend_id(raw, true) else {
                return Err(invalid(LEGACY_PROVIDER, "unknown picture provider"));
            };
            Some(Pick::named(
                backend,
                OverlaySource::LegacyEnv(LEGACY_PROVIDER.to_owned()),
                LEGACY_PROVIDER,
            ))
        }
    };
    let leg_deadline: Option<Pick<u64>> = match take_env(&table, &mut consumed, LEGACY_DEADLINE) {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
            match parse_count(raw).map_err(|()| invalid(LEGACY_DEADLINE, "expected seconds"))? {
                Some(value) => Some(Pick::named(
                    value,
                    OverlaySource::LegacyEnv(LEGACY_DEADLINE.to_owned()),
                    LEGACY_DEADLINE,
                )),
                None => None,
            }
        }
    };
    let new_mode = match take_env(&table, &mut consumed, NEW_MODE) {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
            let Some(mode) = parse_global_mode(raw) else {
                return Err(invalid(NEW_MODE, "unknown mode"));
            };
            Some(Pick::named(
                mode,
                OverlaySource::NewEnv(NEW_MODE.to_owned()),
                NEW_MODE,
            ))
        }
    };
    let new_backend = match take_env(&table, &mut consumed, NEW_BACKEND) {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
            let Some(backend) = parse_backend_id(raw, false) else {
                return Err(invalid(NEW_BACKEND, "unknown backend"));
            };
            Some(Pick::named(
                backend,
                OverlaySource::NewEnv(NEW_BACKEND.to_owned()),
                NEW_BACKEND,
            ))
        }
    };
    // Legacy env is a lower layer than new env: a present new value wins
    // silently over a legacy one. No alias conflict across layers.
    if let Some(pick) = new_backend.or(leg_provider) {
        let source = pick.source.clone();
        let value = pick.value;
        config.default_backend = value;
        note(provenance, "default_backend", source);
    }
    if let Some(pick) = new_mode {
        config.mode = pick.value;
        note(provenance, "mode", pick.source);
    }
    let new_plate = purpose_env_enabled(&table, &mut consumed, Purpose::ScenePlate)?;
    let new_edit = purpose_env_enabled(&table, &mut consumed, Purpose::SceneEdit)?;
    let new_creator = purpose_env_enabled(&table, &mut consumed, Purpose::CreatorPortrait)?;
    let legacy_plate = combine_env_plate(&leg_pictures, &leg_plates);
    if let Some(pick) = new_plate.or(legacy_plate) {
        config.purposes[Purpose::ScenePlate.index()].enabled = pick.value;
        note(provenance, "purposes.scene_plate.enabled", pick.source);
    }
    if let Some(pick) = new_edit.or(leg_pictures.clone()) {
        config.purposes[Purpose::SceneEdit.index()].enabled = pick.value;
        note(provenance, "purposes.scene_edit.enabled", pick.source);
    }
    if let Some(pick) = new_creator.or(leg_creator.clone()) {
        config.purposes[Purpose::CreatorPortrait.index()].enabled = pick.value;
        note(provenance, "purposes.creator_portrait.enabled", pick.source);
    }
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
        if let Some(pick) = purpose_env_enabled(&table, &mut consumed, purpose)? {
            let field = format!("purposes.{}.enabled", purpose.label());
            config.purposes[index].enabled = pick.value;
            note(provenance, &field, pick.source);
        } else if let Some(legacy) = leg_pictures.clone() {
            // The legacy master switch is a lower layer: it applies only when
            // no new per-purpose value is present.
            let field = format!("purposes.{}.enabled", purpose.label());
            config.purposes[index].enabled = legacy.value;
            note(provenance, &field, legacy.source);
        }
    }
    if let Some(pick) = leg_video {
        if !pick.value {
            config.purposes[Purpose::AnimationFrame.index()].enabled = false;
            note(
                provenance,
                "purposes.animation_frame.enabled",
                pick.source,
            );
        }
    }
    if let Some(pick) = leg_disable_openai {
        if pick.value {
            config.backends[BackendId::OpenAi.index()].enabled = false;
            note(provenance, "backends.openai.enabled", pick.source);
        }
    }
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if purpose == Purpose::ScenePlate
            || purpose == Purpose::SceneEdit
            || purpose == Purpose::CreatorPortrait
        {
            continue;
        }
        if master_gated_purpose(purpose) {
            continue;
        }
        if let Some(pick) = purpose_env_enabled(&table, &mut consumed, purpose)? {
            let field = format!("purposes.{}.enabled", purpose.label());
            config.purposes[index].enabled = pick.value;
            note(provenance, &field, pick.source);
        }
    }
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if let Some(pick) = purpose_env_backend(&table, &mut consumed, purpose)? {
            let field = format!("purposes.{}.backend_override", purpose.label());
            config.purposes[index].backend_override = pick.value;
            note(provenance, &field, pick.source);
        }
    }
    if let Some(count) = leg_deadline {
        apply_u64_pick(
            Some(count),
            "scheduler.stage_deadline_secs",
            &mut config.scheduler.stage_deadline_secs,
            provenance,
        )?;
    }
    for index in 0..BackendId::COUNT {
        let label = super::shared::backend_label_from_index(index);
        let tag = backend_env_tag(match index {
            0 => BackendId::OpenAi,
            1 => BackendId::Fal,
            2 => BackendId::Bfl,
            _ => BackendId::CodexCli,
        });
        let enabled_name = format!("DF_IMAGERY_BACKEND_{tag}_ENABLED");
        if let Some(pick) = new_flag(&enabled_name, &table, &mut consumed)? {
            config.backends[index].enabled = pick.value;
            note(provenance, &format!("backends.{label}.enabled"), pick.source);
        }
        let account_name = format!("DF_IMAGERY_BACKEND_{tag}_ACCOUNT");
        if let Some(raw) = take_env(&table, &mut consumed, &account_name) {
            if !raw.is_empty() {
                let account = AccountHandle::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&account_name, reason)
                    }
                })?;
                config.backends[index].account = account;
                note(
                    provenance,
                    &format!("backends.{label}.account"),
                    OverlaySource::NewEnv(account_name.clone()),
                );
            }
        }
        let concurrency_name = format!("DF_IMAGERY_BACKEND_{tag}_MAX_CONCURRENT_JOBS");
        apply_u32_pick(
            new_count(&concurrency_name, &table, &mut consumed)?,
            &format!("backends.{label}.max_concurrent_jobs"),
            &mut config.backends[index].max_concurrent_jobs,
            provenance,
        )?;
    }
    apply_env_bool(
        new_flag("DF_IMAGERY_TRANSFORMS_ENABLED", &table, &mut consumed)?,
        "transforms.enabled",
        &mut config.transforms.enabled,
        provenance,
    );
    apply_u32_pick(
        new_count("DF_IMAGERY_TRANSFORMS_MAX_CONCURRENCY", &table, &mut consumed)?,
        "transforms.max_concurrency",
        &mut config.transforms.max_concurrency,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_TRANSFORMS_MAX_DECODED_BYTES", &table, &mut consumed)?,
        "transforms.max_decoded_bytes",
        &mut config.transforms.max_decoded_bytes,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_TRANSFORMS_MAX_OUTPUT_BYTES", &table, &mut consumed)?,
        "transforms.max_output_bytes",
        &mut config.transforms.max_output_bytes,
        provenance,
    )?;
    apply_u32_pick(
        new_count("DF_IMAGERY_TRANSFORMS_QUEUE_CAPACITY", &table, &mut consumed)?,
        "transforms.queue_capacity",
        &mut config.transforms.queue_capacity,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_TRANSFORMS_DEADLINE_SECS", &table, &mut consumed)?,
        "transforms.deadline_secs",
        &mut config.transforms.deadline_secs,
        provenance,
    )?;
    apply_u32_pick(
        new_count("DF_IMAGERY_SCHEDULER_FOREGROUND_LIMIT", &table, &mut consumed)?,
        "scheduler.foreground_limit",
        &mut config.scheduler.foreground_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_count("DF_IMAGERY_SCHEDULER_PRESENCE_LIMIT", &table, &mut consumed)?,
        "scheduler.presence_limit",
        &mut config.scheduler.presence_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_count("DF_IMAGERY_SCHEDULER_BACKGROUND_LIMIT", &table, &mut consumed)?,
        "scheduler.background_limit",
        &mut config.scheduler.background_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_count("DF_IMAGERY_SCHEDULER_MAX_PENDING_JOBS", &table, &mut consumed)?,
        "scheduler.max_pending_jobs",
        &mut config.scheduler.max_pending_jobs,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES", &table, &mut consumed)?,
        "scheduler.max_pending_bytes",
        &mut config.scheduler.max_pending_bytes,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_SCHEDULER_QUEUE_AGE_SECS", &table, &mut consumed)?,
        "scheduler.queue_age_secs",
        &mut config.scheduler.queue_age_secs,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_SCHEDULER_JOB_DEADLINE_SECS", &table, &mut consumed)?,
        "scheduler.job_deadline_secs",
        &mut config.scheduler.job_deadline_secs,
        provenance,
    )?;
    apply_u64_pick(
        new_count("DF_IMAGERY_SCHEDULER_STAGE_DEADLINE_SECS", &table, &mut consumed)?,
        "scheduler.stage_deadline_secs",
        &mut config.scheduler.stage_deadline_secs,
        provenance,
    )?;
    apply_usize_pick(
        new_count("DF_IMAGERY_REQUEST_MAX_PROMPT_CHARS", &table, &mut consumed)?,
        "request_limits.max_prompt_chars",
        &mut config.request_limits.max_prompt_chars,
        provenance,
    )?;
    apply_usize_pick(
        new_count("DF_IMAGERY_REQUEST_MAX_REFERENCES", &table, &mut consumed)?,
        "request_limits.max_references",
        &mut config.request_limits.max_references,
        provenance,
    )?;
    apply_usize_pick(
        new_count("DF_IMAGERY_REQUEST_MAX_TRANSFORM_STEPS", &table, &mut consumed)?,
        "request_limits.max_transform_steps",
        &mut config.request_limits.max_transform_steps,
        provenance,
    )?;
    apply_usize_pick(
        new_count("DF_IMAGERY_REQUEST_MAX_COMPOSITE_LAYERS", &table, &mut consumed)?,
        "request_limits.max_composite_layers",
        &mut config.request_limits.max_composite_layers,
        provenance,
    )?;
    for name in table.keys() {
        if !consumed.contains(name)
            && (name.starts_with("DF_IMAGERY_") || name.starts_with("DF_GAMEPLAY_IMAGERY"))
        {
            return Err(OverlayError::UnknownField {
                field: name.clone(),
            });
        }
    }
    Ok(())
}

fn combine_env_plate(
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
            Some(Pick::named(value, source.source.clone(), &source.name))
        }
        (Some(pick), None) | (None, Some(pick)) => {
            Some(Pick::named(pick.value, pick.source.clone(), &pick.name))
        }
        (None, None) => None,
    }
}

fn purpose_env_enabled(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    purpose: Purpose,
) -> Result<Option<Pick<bool>>, OverlayError> {
    let name = format!("DF_IMAGERY_PURPOSE_{}_ENABLED", purpose_env_tag(purpose));
    let Some(raw) = take_env(table, consumed, &name) else {
        return Ok(None);
    };
    let Some(flag) =
        parse_overlay_bool(raw).map_err(|()| invalid(&name, "expected on, off, true, or false"))?
    else {
        return Ok(None);
    };
    Ok(Some(Pick::named(
        flag,
        OverlaySource::NewEnv(name.clone()),
        &name,
    )))
}

fn purpose_env_backend(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    purpose: Purpose,
) -> Result<Option<Pick<Option<BackendId>>>, OverlayError> {
    let name = format!("DF_IMAGERY_PURPOSE_{}_BACKEND", purpose_env_tag(purpose));
    let Some(raw) = take_env(table, consumed, &name) else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    if raw == "none" {
        return Ok(Some(Pick::named(
            None,
            OverlaySource::NewEnv(name.clone()),
            &name,
        )));
    }
    let Some(backend) = parse_backend_id(raw, false) else {
        return Err(invalid(&name, "unknown backend"));
    };
    Ok(Some(Pick::named(
        Some(backend),
        OverlaySource::NewEnv(name.clone()),
        &name,
    )))
}

fn apply_env_bool(pick: Option<Pick<bool>>, field: &str, slot: &mut bool, provenance: &mut Provenance) {
    if let Some(pick) = pick {
        *slot = pick.value;
        note(provenance, field, pick.source);
    }
}
