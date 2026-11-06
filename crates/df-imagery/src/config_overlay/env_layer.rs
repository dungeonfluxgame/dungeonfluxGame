//! Environment-layer precedence for the imagery config overlay.
//!
//! Precedence is defaults, then the file layer, then legacy launcher
//! environment, then new `DF_IMAGERY_*` environment, then the typed runtime
//! patch. A higher layer wins silently over a lower one. Names are recognized
//! before values are inspected: unrelated environment (including credentials)
//! is ignored without cloning its values. Only imagery-owned forbidden secret
//! names are refused, never a generic substring.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{
    AccountHandle, ApiEndpoint, BackendId, CliProfileRevision, CodexExecutable, ImageConfig,
    JobWorkDir, ModelName, OutputProfileName, PriceFileRef,
};
use crate::request::Purpose;

use super::parse::{
    backend_env_tag, parse_backend_id, parse_count, parse_global_mode, parse_legacy_switch,
    parse_overlay_bool, profile_name_from_tag, purpose_env_tag,
};
use super::shared::{
    BodyField, Pick, ProfileEdit, apply_u32_pick, apply_u64_pick, apply_usize_pick, invalid,
    merge_profile_body, note, note_owned, profile_body, seed_profile_body,
};
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
    name.to_ascii_uppercase().split('_').any(|token| {
        matches!(
            token,
            "KEY" | "SECRET" | "TOKEN" | "PASSWORD" | "CREDENTIAL"
        )
    })
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

/// Allocation guards for recognized environment only. Unrelated names are
/// skipped before any bound or clone, so ordinary credentials never enter
/// the map. Bounds exceed every valid shape: short names, small scalars, and
/// a named output-profile table with no fixed count (eight variables each).
const MAXIMUM_ENV_ENTRIES: usize = 16_384;
const MAXIMUM_ENV_NAME_CHARS: usize = 128;
const MAXIMUM_ENV_VALUE_BYTES: usize = 4096;

pub(super) fn apply_env_layer(
    env: &[(&str, &str)],
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    // Recognize names before inspecting values. Unrelated environment is
    // skipped without cloning its value, so credentials never enter the map.
    // Each bound is checked before the pair is retained, so a hostile
    // oversized input never grows the collection and never echoes back.
    let mut recognized: Vec<(&str, &str)> = Vec::new();
    for (name, value) in env {
        if !is_imagery_owned_env_name(name) {
            continue;
        }
        if recognized.len() >= MAXIMUM_ENV_ENTRIES {
            return Err(invalid("imagery", "too many entries"));
        }
        if name.len() > MAXIMUM_ENV_NAME_CHARS {
            return Err(invalid("imagery", "env name exceeds bound"));
        }
        if value.len() > MAXIMUM_ENV_VALUE_BYTES {
            return Err(invalid(name, "env value exceeds bound"));
        }
        if is_forbidden_secret_env_name(name) {
            return Err(OverlayError::SecretRefused {
                name: (*name).to_owned(),
            });
        }
        recognized.push((*name, *value));
    }
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in recognized {
        // Secret names were refused above without cloning their values; the
        // remaining owned names are safe to record.
        if table.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(OverlayError::DuplicateField {
                field: name.to_owned(),
            });
        }
    }
    let mut consumed: BTreeSet<String> = BTreeSet::new();
    for name in IGNORED_ENV {
        if table.contains_key(*name) {
            return Err(invalid(name, "unsupported field"));
        }
    }
    let legacy_flag = |name: &'static str,
                       table: &BTreeMap<String, String>,
                       consumed: &mut BTreeSet<String>|
     -> Result<Option<Pick<bool>>, OverlayError> {
        let Some(raw) = take_env(table, consumed, name) else {
            return Ok(None);
        };
        let Some(flag) =
            parse_legacy_switch(raw).map_err(|()| invalid(name, "expected on or off"))?
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
        Some("") => None,
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
        Some("") => None,
        Some(raw) => parse_count(raw)
            .map_err(|()| invalid(LEGACY_DEADLINE, "expected seconds"))?
            .map(|value| {
                Pick::named(
                    value,
                    OverlaySource::LegacyEnv(LEGACY_DEADLINE.to_owned()),
                    LEGACY_DEADLINE,
                )
            }),
    };
    let new_mode = match take_env(&table, &mut consumed, NEW_MODE) {
        None => None,
        Some("") => None,
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
        Some("") => None,
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
    let mut legacy_plate = combine_env_plate(leg_pictures.as_ref(), leg_plates.as_ref());
    let mut legacy_edit = combine_env_plate(leg_pictures.as_ref(), leg_plates.as_ref());
    // A lower-layer master off is sticky for legacy plates alone: a legacy
    // plates-only enable must not re-enable a plate the file master turned
    // off. A higher-layer master on, or a new per-purpose on, still wins.
    if leg_pictures.is_none() {
        let plate_master_off = !config.purposes[Purpose::ScenePlate.index()].enabled
            && matches!(
                provenance.source_of("purposes.scene_plate.enabled"),
                Some(OverlaySource::FilePictures)
            );
        if plate_master_off && matches!(&legacy_plate, Some(pick) if pick.value) {
            legacy_plate = None;
        }
        let edit_master_off = !config.purposes[Purpose::SceneEdit.index()].enabled
            && matches!(
                provenance.source_of("purposes.scene_edit.enabled"),
                Some(OverlaySource::FilePictures)
            );
        if edit_master_off && matches!(&legacy_edit, Some(pick) if pick.value) {
            legacy_edit = None;
        }
    }
    if let Some(pick) = new_plate.or(legacy_plate) {
        config.purposes[Purpose::ScenePlate.index()].enabled = pick.value;
        note(provenance, "purposes.scene_plate.enabled", pick.source);
    }
    if let Some(pick) = new_edit.or(legacy_edit) {
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
            let field = format!("purposes.{}.enabled", purpose.key());
            config.purposes[index].enabled = pick.value;
            note(provenance, &field, pick.source);
        } else if let Some(legacy) = leg_pictures.clone() {
            // The legacy master switch is a lower layer: it applies only when
            // no new per-purpose value is present.
            let field = format!("purposes.{}.enabled", purpose.key());
            config.purposes[index].enabled = legacy.value;
            note(provenance, &field, legacy.source);
        }
    }
    if let Some(pick) = leg_video
        && !pick.value
    {
        config.purposes[Purpose::AnimationFrame.index()].enabled = false;
        note(provenance, "purposes.animation_frame.enabled", pick.source);
    }
    if let Some(pick) = leg_disable_openai
        && pick.value
    {
        config.backends[BackendId::OpenAi.index()].enabled = false;
        note(provenance, "backends.openai.enabled", pick.source);
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
            let field = format!("purposes.{}.enabled", purpose.key());
            config.purposes[index].enabled = pick.value;
            note(provenance, &field, pick.source);
        }
    }
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if let Some(pick) = purpose_env_backend(&table, &mut consumed, purpose)? {
            let field = format!("purposes.{}.backend_override", purpose.key());
            config.purposes[index].backend_override = pick.value;
            note(provenance, &field, pick.source);
        }
        let account_name = format!(
            "DF_IMAGERY_PURPOSE_{}_ACCOUNT_OVERRIDE",
            purpose_env_tag(purpose)
        );
        if let Some(raw) =
            take_env(&table, &mut consumed, &account_name).filter(|value| !value.is_empty())
        {
            config.purposes[index].account_override = if raw == "none" {
                None
            } else {
                Some(super::need_account(&account_name, raw)?)
            };
            note(
                provenance,
                &format!("purposes.{}.account_override", purpose.key()),
                OverlaySource::NewEnv(account_name),
            );
        }
        let model_name = format!(
            "DF_IMAGERY_PURPOSE_{}_MODEL_OVERRIDE",
            purpose_env_tag(purpose)
        );
        if let Some(raw) =
            take_env(&table, &mut consumed, &model_name).filter(|value| !value.is_empty())
        {
            let selected = if raw == "none" {
                None
            } else {
                Some(super::need_model(&model_name, raw)?)
            };
            config.purposes[index].model_override = selected;
            note(
                provenance,
                &format!("purposes.{}.model_override", purpose.key()),
                OverlaySource::NewEnv(model_name),
            );
        }
        let moderation_name = format!(
            "DF_IMAGERY_PURPOSE_{}_MODERATION_OVERRIDE",
            purpose_env_tag(purpose)
        );
        if let Some(raw) =
            take_env(&table, &mut consumed, &moderation_name).filter(|value| !value.is_empty())
        {
            let selected = if raw == "none" {
                None
            } else {
                Some(
                    super::parse::parse_provider_moderation(raw)
                        .ok_or_else(|| invalid(&moderation_name, "expected auto, low, or none"))?,
                )
            };
            config.purposes[index].moderation_override = selected;
            note(
                provenance,
                &format!("purposes.{}.moderation_override", purpose.key()),
                OverlaySource::NewEnv(moderation_name),
            );
        }
        let quality_name = format!(
            "DF_IMAGERY_PURPOSE_{}_QUALITY_OVERRIDE",
            purpose_env_tag(purpose)
        );
        if let Some(raw) =
            take_env(&table, &mut consumed, &quality_name).filter(|value| !value.is_empty())
        {
            let selected = if raw == "none" {
                None
            } else {
                Some(super::parse::parse_generation_quality(raw).ok_or_else(|| {
                    invalid(&quality_name, "expected low, medium, high, auto, or none")
                })?)
            };
            config.purposes[index].quality_override = selected;
            note(
                provenance,
                &format!("purposes.{}.quality_override", purpose.key()),
                OverlaySource::NewEnv(quality_name),
            );
        }
        if let Some(pick) = purpose_env_output_profile(&table, &mut consumed, purpose)? {
            let field = format!("purposes.{}.output_profile", purpose.key());
            config.purposes[index].output_profile = pick.value;
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
            note(
                provenance,
                &format!("backends.{label}.enabled"),
                pick.source,
            );
        }
        let account_name = format!("DF_IMAGERY_BACKEND_{tag}_ACCOUNT");
        if let Some(raw) = take_env(&table, &mut consumed, &account_name)
            && !raw.is_empty()
        {
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
        let concurrency_name = format!("DF_IMAGERY_BACKEND_{tag}_MAX_CONCURRENT_JOBS");
        apply_u32_pick(
            new_count(&concurrency_name, &table, &mut consumed)?,
            &format!("backends.{label}.max_concurrent_jobs"),
            &mut config.backends[index].max_concurrent_jobs,
            provenance,
        )?;
        let model_name = format!("DF_IMAGERY_BACKEND_{tag}_MODEL");
        if let Some(raw) = take_env(&table, &mut consumed, &model_name)
            && !raw.is_empty()
        {
            if raw == "none" {
                config.backends[index].model = None;
            } else {
                let model = ModelName::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&model_name, reason)
                    }
                })?;
                config.backends[index].model = Some(model);
            }
            note(
                provenance,
                &format!("backends.{label}.model"),
                OverlaySource::NewEnv(model_name.clone()),
            );
        }
        let model_name = format!("DF_IMAGERY_BACKEND_{tag}_GENERATION_MODEL");
        if let Some(raw) = take_env(&table, &mut consumed, &model_name)
            && !raw.is_empty()
        {
            if raw == "none" {
                config.backends[index].generation_model = None;
            } else {
                let model = ModelName::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&model_name, reason)
                    }
                })?;
                config.backends[index].generation_model = Some(model);
            }
            note(
                provenance,
                &format!("backends.{label}.generation_model"),
                OverlaySource::NewEnv(model_name.clone()),
            );
        }
        let model_name = format!("DF_IMAGERY_BACKEND_{tag}_EDIT_MODEL");
        if let Some(raw) = take_env(&table, &mut consumed, &model_name)
            && !raw.is_empty()
        {
            if raw == "none" {
                config.backends[index].edit_model = None;
            } else {
                let model = ModelName::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&model_name, reason)
                    }
                })?;
                config.backends[index].edit_model = Some(model);
            }
            note(
                provenance,
                &format!("backends.{label}.edit_model"),
                OverlaySource::NewEnv(model_name.clone()),
            );
        }
        let model_name = format!("DF_IMAGERY_BACKEND_{tag}_ALPHA_MODEL");
        if let Some(raw) = take_env(&table, &mut consumed, &model_name)
            && !raw.is_empty()
        {
            if raw == "none" {
                config.backends[index].alpha_model = None;
            } else {
                let model = ModelName::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&model_name, reason)
                    }
                })?;
                config.backends[index].alpha_model = Some(model);
            }
            note(
                provenance,
                &format!("backends.{label}.alpha_model"),
                OverlaySource::NewEnv(model_name.clone()),
            );
        }
        let endpoint_name = format!("DF_IMAGERY_BACKEND_{tag}_ENDPOINT");
        if let Some(raw) = take_env(&table, &mut consumed, &endpoint_name)
            && !raw.is_empty()
        {
            if raw == "none" {
                config.backends[index].endpoint = None;
            } else {
                let endpoint = ApiEndpoint::new(raw).map_err(|error| match error {
                    crate::config::ConfigError::InvalidField { reason, .. } => {
                        invalid(&endpoint_name, reason)
                    }
                })?;
                config.backends[index].endpoint = Some(endpoint);
            }
            note(
                provenance,
                &format!("backends.{label}.endpoint"),
                OverlaySource::NewEnv(endpoint_name.clone()),
            );
        }
    }
    apply_env_bool(
        new_flag("DF_IMAGERY_TRANSFORMS_ENABLED", &table, &mut consumed)?,
        "transforms.enabled",
        &mut config.transforms.enabled,
        provenance,
    );
    apply_u32_pick(
        new_count(
            "DF_IMAGERY_TRANSFORMS_MAX_CONCURRENCY",
            &table,
            &mut consumed,
        )?,
        "transforms.max_concurrency",
        &mut config.transforms.max_concurrency,
        provenance,
    )?;
    apply_u64_pick(
        new_count(
            "DF_IMAGERY_TRANSFORMS_MAX_DECODED_BYTES",
            &table,
            &mut consumed,
        )?,
        "transforms.max_decoded_bytes",
        &mut config.transforms.max_decoded_bytes,
        provenance,
    )?;
    apply_u64_pick(
        new_count(
            "DF_IMAGERY_TRANSFORMS_MAX_OUTPUT_BYTES",
            &table,
            &mut consumed,
        )?,
        "transforms.max_output_bytes",
        &mut config.transforms.max_output_bytes,
        provenance,
    )?;
    apply_u32_pick(
        new_count(
            "DF_IMAGERY_TRANSFORMS_QUEUE_CAPACITY",
            &table,
            &mut consumed,
        )?,
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
        new_count(
            "DF_IMAGERY_SCHEDULER_FOREGROUND_LIMIT",
            &table,
            &mut consumed,
        )?,
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
        new_count(
            "DF_IMAGERY_SCHEDULER_BACKGROUND_LIMIT",
            &table,
            &mut consumed,
        )?,
        "scheduler.background_limit",
        &mut config.scheduler.background_limit,
        provenance,
    )?;
    apply_u32_pick(
        new_count(
            "DF_IMAGERY_SCHEDULER_MAX_PENDING_JOBS",
            &table,
            &mut consumed,
        )?,
        "scheduler.max_pending_jobs",
        &mut config.scheduler.max_pending_jobs,
        provenance,
    )?;
    apply_u64_pick(
        new_count(
            "DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES",
            &table,
            &mut consumed,
        )?,
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
        new_count(
            "DF_IMAGERY_SCHEDULER_JOB_DEADLINE_SECS",
            &table,
            &mut consumed,
        )?,
        "scheduler.job_deadline_secs",
        &mut config.scheduler.job_deadline_secs,
        provenance,
    )?;
    apply_u64_pick(
        new_count(
            "DF_IMAGERY_SCHEDULER_STAGE_DEADLINE_SECS",
            &table,
            &mut consumed,
        )?,
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
        new_count(
            "DF_IMAGERY_REQUEST_MAX_TRANSFORM_STEPS",
            &table,
            &mut consumed,
        )?,
        "request_limits.max_transform_steps",
        &mut config.request_limits.max_transform_steps,
        provenance,
    )?;
    apply_usize_pick(
        new_count(
            "DF_IMAGERY_REQUEST_MAX_COMPOSITE_LAYERS",
            &table,
            &mut consumed,
        )?,
        "request_limits.max_composite_layers",
        &mut config.request_limits.max_composite_layers,
        provenance,
    )?;
    apply_analysis_env(&table, &mut consumed, config, provenance)?;
    if let Some(pick) = typed_env(&table, &mut consumed, "DF_IMAGERY_QUOTA_ENABLED", |raw| {
        parse_overlay_bool(raw)
            .map_err(|()| {
                invalid(
                    "DF_IMAGERY_QUOTA_ENABLED",
                    "expected on, off, true, or false",
                )
            })?
            .ok_or_else(|| {
                invalid(
                    "DF_IMAGERY_QUOTA_ENABLED",
                    "expected on, off, true, or false",
                )
            })
    })? {
        config.quota.enabled = pick.value;
        note(provenance, "quota.enabled", pick.source);
    }
    apply_u64_pick(
        env_count(&table, &mut consumed, "DF_IMAGERY_QUOTA_COOLDOWN_MS")?,
        "quota.cooldown_ms",
        &mut config.quota.cooldown_ms,
        provenance,
    )?;
    apply_u32_pick(
        env_count(&table, &mut consumed, "DF_IMAGERY_QUOTA_MAXIMUM_ACCOUNTS")?,
        "quota.maximum_accounts",
        &mut config.quota.maximum_accounts,
        provenance,
    )?;
    if let Some(pick) = typed_env(
        &table,
        &mut consumed,
        "DF_IMAGERY_CREATOR_BFL_ACCOUNT",
        |raw| {
            if raw == "none" {
                Ok(None)
            } else {
                AccountHandle::new(raw)
                    .map(Some)
                    .map_err(|error| match error {
                        crate::config::ConfigError::InvalidField { reason, .. } => {
                            invalid("DF_IMAGERY_CREATOR_BFL_ACCOUNT", reason)
                        }
                    })
            }
        },
    )? {
        config.creator_bfl_account = pick.value;
        note(provenance, "creator.bfl_account", pick.source);
    }
    apply_codex_env(&table, &mut consumed, config, provenance)?;
    apply_retry_env(&table, &mut consumed, config, provenance)?;
    apply_accounting_env(&table, &mut consumed, config, provenance)?;
    apply_quality_env(&table, &mut consumed, config, provenance)?;
    apply_default_profile_env(&table, &mut consumed, config, provenance)?;
    apply_profile_body_env(&table, &mut consumed, config, provenance)?;
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
    pictures: Option<&Pick<bool>>,
    plates: Option<&Pick<bool>>,
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

fn purpose_env_output_profile(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    purpose: Purpose,
) -> Result<Option<Pick<Option<OutputProfileName>>>, OverlayError> {
    let name = format!(
        "DF_IMAGERY_PURPOSE_{}_OUTPUT_PROFILE",
        purpose_env_tag(purpose)
    );
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
    let profile = OutputProfileName::new(raw).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => invalid(&name, reason),
    })?;
    Ok(Some(Pick::named(
        Some(profile),
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

fn apply_env_bool(
    pick: Option<Pick<bool>>,
    field: &str,
    slot: &mut bool,
    provenance: &mut Provenance,
) {
    if let Some(pick) = pick {
        *slot = pick.value;
        note(provenance, field, pick.source);
    }
}

fn typed_env<T>(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    name: &str,
    build: impl FnOnce(&str) -> Result<T, OverlayError>,
) -> Result<Option<Pick<T>>, OverlayError> {
    let Some(raw) = take_env(table, consumed, name) else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let value = build(raw)?;
    Ok(Some(Pick::named(
        value,
        OverlaySource::NewEnv(name.to_owned()),
        name,
    )))
}

fn apply_codex_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(pick) = typed_env(table, consumed, "DF_IMAGERY_CODEX_EXECUTABLE", |raw| {
        CodexExecutable::new(raw).map_err(|error| match error {
            crate::config::ConfigError::InvalidField { reason, .. } => {
                invalid("DF_IMAGERY_CODEX_EXECUTABLE", reason)
            }
        })
    })? {
        config.codex.executable = pick.value;
        note(provenance, "codex.executable", pick.source);
    }
    if let Some(pick) = typed_env(table, consumed, "DF_IMAGERY_CODEX_WORKDIR", |raw| {
        JobWorkDir::new(raw).map_err(|error| match error {
            crate::config::ConfigError::InvalidField { reason, .. } => {
                invalid("DF_IMAGERY_CODEX_WORKDIR", reason)
            }
        })
    })? {
        config.codex.workdir = pick.value;
        note(provenance, "codex.workdir", pick.source);
    }
    if let Some(pick) = typed_env(table, consumed, "DF_IMAGERY_CODEX_CLI_PROFILE", |raw| {
        CliProfileRevision::new(raw).map_err(|error| match error {
            crate::config::ConfigError::InvalidField { reason, .. } => {
                invalid("DF_IMAGERY_CODEX_CLI_PROFILE", reason)
            }
        })
    })? {
        config.codex.cli_profile = pick.value;
        note(provenance, "codex.cli_profile", pick.source);
    }
    if let Some(pick) = env_count(table, consumed, "DF_IMAGERY_CODEX_MAX_STDOUT_BYTES")? {
        config.codex.max_stdout_bytes = pick.value;
        note(provenance, "codex.max_stdout_bytes", pick.source);
    }
    if let Some(pick) = env_count(table, consumed, "DF_IMAGERY_CODEX_MAX_STDERR_BYTES")? {
        config.codex.max_stderr_bytes = pick.value;
        note(provenance, "codex.max_stderr_bytes", pick.source);
    }
    if let Some(pick) = env_count(table, consumed, "DF_IMAGERY_CODEX_MAX_MESSAGE_BYTES")? {
        config.codex.max_message_bytes = pick.value;
        note(provenance, "codex.max_message_bytes", pick.source);
    }
    if let Some(raw) = take_env(table, consumed, "DF_IMAGERY_CODEX_AGENT_MODEL")
        && !raw.is_empty()
    {
        if raw == "none" {
            config.codex.agent_model = None;
        } else {
            let model = ModelName::new(raw).map_err(|error| match error {
                crate::config::ConfigError::InvalidField { reason, .. } => {
                    invalid("DF_IMAGERY_CODEX_AGENT_MODEL", reason)
                }
            })?;
            config.codex.agent_model = Some(model);
        }
        note(
            provenance,
            "codex.agent_model",
            OverlaySource::NewEnv("DF_IMAGERY_CODEX_AGENT_MODEL".to_owned()),
        );
    }
    Ok(())
}

fn env_count(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    name: &str,
) -> Result<Option<Pick<u64>>, OverlayError> {
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
}

fn apply_retry_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    apply_u32_pick(
        env_count(table, consumed, "DF_IMAGERY_RETRY_MAX_ATTEMPTS")?,
        "retry.max_attempts",
        &mut config.retry.max_attempts,
        provenance,
    )?;
    apply_u64_pick(
        env_count(table, consumed, "DF_IMAGERY_RETRY_DEADLINE_SECS")?,
        "retry.retry_deadline_secs",
        &mut config.retry.retry_deadline_secs,
        provenance,
    )?;
    apply_u64_pick(
        env_count(table, consumed, "DF_IMAGERY_RETRY_BACKOFF_SECS")?,
        "retry.backoff_secs",
        &mut config.retry.backoff_secs,
        provenance,
    )
}

fn apply_accounting_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(raw) = take_env(table, consumed, "DF_IMAGERY_ACCOUNTING_MODE")
        && !raw.is_empty()
    {
        let Some(mode) = crate::config::AccountingMode::from_key(raw) else {
            return Err(invalid(
                "DF_IMAGERY_ACCOUNTING_MODE",
                "unknown accounting mode",
            ));
        };
        config.accounting.mode = mode;
        note(
            provenance,
            "accounting.mode",
            OverlaySource::NewEnv("DF_IMAGERY_ACCOUNTING_MODE".to_owned()),
        );
    }
    if let Some(raw) = take_env(table, consumed, "DF_IMAGERY_ACCOUNTING_PRICE_FILE")
        && !raw.is_empty()
    {
        if raw == "none" {
            config.accounting.price_file = None;
        } else {
            let reference = PriceFileRef::new(raw).map_err(|error| match error {
                crate::config::ConfigError::InvalidField { reason, .. } => {
                    invalid("DF_IMAGERY_ACCOUNTING_PRICE_FILE", reason)
                }
            })?;
            config.accounting.price_file = Some(reference);
        }
        note(
            provenance,
            "accounting.price_file",
            OverlaySource::NewEnv("DF_IMAGERY_ACCOUNTING_PRICE_FILE".to_owned()),
        );
    }
    Ok(())
}

fn apply_quality_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(raw) = take_env(table, consumed, "DF_IMAGERY_QUALITY_JPEG_QUALITY")
        && !raw.is_empty()
    {
        let count = parse_count(raw).map_err(|()| {
            invalid(
                "DF_IMAGERY_QUALITY_JPEG_QUALITY",
                "expected an unsigned integer",
            )
        })?;
        let Some(count) = count else {
            return Ok(());
        };
        let value = u8::try_from(count)
            .map_err(|_| invalid("DF_IMAGERY_QUALITY_JPEG_QUALITY", "integer outside range"))?;
        let lossy = crate::output::LossyQuality::new(value)
            .map_err(|_| invalid("DF_IMAGERY_QUALITY_JPEG_QUALITY", "a quality is 1 to 100"))?;
        config.quality.jpeg_quality = lossy;
        note(
            provenance,
            "quality.jpeg_quality",
            OverlaySource::NewEnv("DF_IMAGERY_QUALITY_JPEG_QUALITY".to_owned()),
        );
    }
    if let Some(raw) = take_env(table, consumed, "DF_IMAGERY_QUALITY_PROVIDER_QUALITY")
        && !raw.is_empty()
    {
        if raw == "none" {
            config.quality.provider_quality = None;
        } else {
            let quality = super::parse::parse_generation_quality(raw).ok_or_else(|| {
                invalid(
                    "DF_IMAGERY_QUALITY_PROVIDER_QUALITY",
                    "expected low, medium, high, auto, or none",
                )
            })?;
            config.quality.provider_quality = Some(quality);
        }
        note(
            provenance,
            "quality.provider_quality",
            OverlaySource::NewEnv("DF_IMAGERY_QUALITY_PROVIDER_QUALITY".to_owned()),
        );
    }
    Ok(())
}

fn apply_default_profile_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(pick) = typed_env(
        table,
        consumed,
        "DF_IMAGERY_OUTPUTS_DEFAULT_PROFILE",
        |raw| {
            OutputProfileName::new(raw).map_err(|error| match error {
                crate::config::ConfigError::InvalidField { reason, .. } => {
                    invalid("DF_IMAGERY_OUTPUTS_DEFAULT_PROFILE", reason)
                }
            })
        },
    )? {
        config.default_output_profile = pick.value;
        note(provenance, "outputs.default_profile", pick.source);
    }
    Ok(())
}

/// Profile bodies from `DF_IMAGERY_OUTPUT_<TAG>_<FIELD>` variables, merged
/// by name over the lower layers. The tag reverses to the file spelling;
/// names that cannot be profile names stay unconsumed and surface as
/// unknown fields below.
fn apply_profile_body_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    const FIELDS: &[&str] = &[
        "GENERATED_WIDTH_PX",
        "GENERATED_HEIGHT_PX",
        "WIDTH_PX",
        "HEIGHT_PX",
        "FORMAT",
        "QUALITY",
        "ALPHA",
        "MATTE",
    ];
    let mut grouped: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for name in table.keys() {
        if consumed.contains(name) {
            continue;
        }
        let Some(rest) = name.strip_prefix("DF_IMAGERY_OUTPUT_") else {
            continue;
        };
        let Some(field) = FIELDS.iter().find(|suffix| {
            rest.len() > suffix.len()
                && rest.ends_with(*suffix)
                && rest.as_bytes()[rest.len() - suffix.len() - 1] == b'_'
        }) else {
            continue;
        };
        let tag = &rest[..rest.len() - field.len() - 1];
        let Some(profile) = profile_name_from_tag(tag) else {
            continue;
        };
        grouped
            .entry(profile)
            .or_default()
            .push(((*field).to_string(), name.clone()));
    }
    for (profile_text, vars) in &grouped {
        let name = OutputProfileName::new(profile_text).map_err(|error| match error {
            crate::config::ConfigError::InvalidField { reason, .. } => {
                invalid(&format!("DF_IMAGERY_OUTPUT_{profile_text}"), reason)
            }
        })?;
        let mut fields = BTreeMap::new();
        let mut field_sources: BTreeMap<String, OverlaySource> = BTreeMap::new();
        for (field, var) in vars {
            let Some(raw) = table.get(var).map(String::as_str) else {
                continue;
            };
            if raw.is_empty() {
                continue;
            }
            let body_field = match field.as_str() {
                "GENERATED_WIDTH_PX"
                | "GENERATED_HEIGHT_PX"
                | "WIDTH_PX"
                | "HEIGHT_PX"
                | "QUALITY" => {
                    if raw == "none" {
                        BodyField::Text("none".to_owned())
                    } else {
                        let count = parse_count(raw)
                            .map_err(|()| invalid(var, "expected an unsigned integer"))?;
                        let Some(count) = count else {
                            continue;
                        };
                        BodyField::Count(count)
                    }
                }
                _ => BodyField::Text(raw.to_owned()),
            };
            fields.insert(field.to_ascii_lowercase(), body_field);
            field_sources.insert(
                field.to_ascii_lowercase(),
                OverlaySource::NewEnv(var.clone()),
            );
            consumed.insert(var.clone());
        }
        if fields.is_empty() {
            continue;
        }
        let Some((_, first)) = vars.first() else {
            continue;
        };
        let edit = ProfileEdit {
            name: name.clone(),
            source: OverlaySource::NewEnv(first.clone()),
            fields,
        };
        let section = first.clone();
        let seeded = seed_profile_body(config, &name)?;
        let body = profile_body(seeded, &edit, &section)?;
        let edited: Vec<String> = edit.fields.keys().cloned().collect();
        let edit_source = edit.source.clone();
        merge_profile_body(config, provenance, name.clone(), body, edit_source, &edited);
        for (field, source) in &field_sources {
            note_owned(
                provenance,
                format!("outputs.{}.{field}", name.as_str()),
                source.clone(),
            );
        }
    }
    Ok(())
}

fn apply_analysis_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    for (group, prefix, limits) in [
        (
            "preparation",
            "DF_IMAGERY_ANALYSIS_PREPARATION",
            &mut config.analysis.preparation,
        ),
        (
            "segmentation_preparation",
            "DF_IMAGERY_ANALYSIS_SEGMENTATION_PREPARATION",
            &mut config.analysis.segmentation_preparation,
        ),
    ] {
        for (field, slot) in [
            ("max_pixels", &mut limits.max_pixels),
            ("max_picture_bytes", &mut limits.max_picture_bytes),
            ("max_request_bytes", &mut limits.max_request_bytes),
        ] {
            let name = format!("{prefix}_{}", field.to_ascii_uppercase());
            if let Some(pick) = env_count(table, consumed, &name)? {
                *slot = pick.value;
                note(
                    provenance,
                    &format!("analysis.{group}.{field}"),
                    pick.source,
                );
            }
        }
    }
    for (stage, slot) in [
        ("classification", &mut config.analysis.classification),
        ("inspection", &mut config.analysis.inspection),
        ("head_locations", &mut config.analysis.head_locations),
        ("staging_identity", &mut config.analysis.staging_identity),
        ("staging_facing", &mut config.analysis.staging_facing),
    ] {
        let base = format!("DF_IMAGERY_ANALYSIS_{}", stage.to_ascii_uppercase());
        let mut patch = None;
        let clear = take_env(table, consumed, &base).filter(|value| !value.is_empty());
        let mut names = BTreeMap::new();
        for field in ["account", "model", "endpoint", "maximum_output_tokens"] {
            let name = format!("{base}_{}", field.to_ascii_uppercase());
            if let Some(raw) = take_env(table, consumed, &name).filter(|value| !value.is_empty()) {
                if clear.is_some() {
                    return Err(invalid(
                        &base,
                        "a route clear cannot be combined with route field edits",
                    ));
                }
                super::analysis::parse_slot(&mut patch, Some(field), &name, raw)?;
                names.insert(field, name);
            }
        }
        if let Some(raw) = clear {
            super::analysis::parse_slot(&mut patch, None, &base, raw)?;
        }
        if let Some(patch) = patch {
            super::analysis::apply_route(slot, &patch, stage, provenance, |field| {
                OverlaySource::NewEnv(
                    names
                        .get(field)
                        .or_else(|| names.values().next())
                        .cloned()
                        .unwrap_or_else(|| base.clone()),
                )
            })?;
        }
    }
    apply_segmentation_env(table, consumed, config, provenance)
}
fn apply_segmentation_env(
    table: &BTreeMap<String, String>,
    consumed: &mut BTreeSet<String>,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    const BASE: &str = "DF_IMAGERY_ANALYSIS_SEGMENTATION";
    let mut patch = None;
    let clear = take_env(table, consumed, BASE).filter(|value| !value.is_empty());
    let mut names = BTreeMap::new();
    for field in ["provider", "account", "model", "endpoint"] {
        let name = format!("{BASE}_{}", field.to_ascii_uppercase());
        if let Some(raw) = take_env(table, consumed, &name).filter(|value| !value.is_empty()) {
            if clear.is_some() {
                return Err(invalid(
                    BASE,
                    "a route clear cannot be combined with route field edits",
                ));
            }
            super::segmentation::parse_slot(&mut patch, Some(field), &name, raw)?;
            names.insert(field, name);
        }
    }
    if let Some(raw) = clear {
        super::segmentation::parse_slot(&mut patch, None, BASE, raw)?;
    }
    if let Some(patch) = patch {
        super::segmentation::apply_route(
            &mut config.analysis.segmentation,
            &patch,
            provenance,
            |field| {
                OverlaySource::NewEnv(
                    names
                        .get(field)
                        .or_else(|| names.values().next())
                        .cloned()
                        .unwrap_or_else(|| BASE.to_owned()),
                )
            },
        )?;
    }
    Ok(())
}
