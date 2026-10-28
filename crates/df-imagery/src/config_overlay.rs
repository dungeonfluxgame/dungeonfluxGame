//! Pure image-configuration overlay: supplied text, pairs and patch in.
//!
//! Resolves one validated [`ImageConfig`] from supplied layers above the
//! built-in defaults. Precedence is defaults, then operator file text, then
//! legacy launcher environment pairs, then new `DF_IMAGERY_*` pairs, then a
//! typed runtime patch. Within the file layer contradictory new and legacy
//! spellings for one field conflict only when their values differ; across
//! layers a higher layer wins silently over a lower one and records its
//! provenance.
//!
//! This module owns parsing and precedence only. It never reads ambient
//! environment, the filesystem, or the network; it never stores, logs, or
//! echoes secrets; it never invents supplier models, executable paths, price
//! data, or provider credentials. Fields with no [`ImageConfig`] counterpart
//! are acknowledged by name in [`Provenance::ignored`] and reported to the
//! coordinator, never silently applied. Disabling a backend or purpose never
//! reroutes work: the recorded backend selection is kept as written and the
//! later service answers `Disabled` for affected demand.
//!
//! Uses only the Rust standard library so no manifest change is required.
//!
//! Implementation lives in sibling modules beside this file; Rust resolves
//! `config_overlay/foo.rs` for `mod foo` declared here, so the file and its
//! directory coexist.

mod env_layer;
mod file_layer;
mod parse;
mod shared;

use crate::config::{AccountHandle, BackendId, GlobalMode, ImageConfig};
use crate::request::Purpose;

/// Where one effective field won its value. All payloads are nonsecret
/// names; values never appear in provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlaySource {
    Default,
    FileImagery,
    FilePictures,
    LegacyEnv(String),
    NewEnv(String),
    Runtime,
}

/// One canonical field and the layer that decided it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldProvenance {
    pub field: String,
    pub source: OverlaySource,
}

/// Nonsecret resolution record: winning source per field, acknowledged
/// no-counterpart inputs, and the caller-supplied revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    pub revision: u64,
    pub fields: Vec<FieldProvenance>,
    pub ignored: Vec<String>,
}

impl Provenance {
    /// The winning source for a canonical field path, if tracked.
    pub fn source_of(&self, field: &str) -> Option<OverlaySource> {
        self.fields
            .iter()
            .find(|entry| entry.field == field)
            .map(|entry| entry.source.clone())
    }
}

/// A validated configuration plus its nonsecret provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedConfig {
    pub config: ImageConfig,
    pub provenance: Provenance,
}

/// Why supplied configuration was refused. Field names are echoed; values
/// never are, so errors stay redacted even for mistyped secrets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlayError {
    UnknownField { field: String },
    DuplicateField { field: String },
    AliasConflict {
        field: String,
        first: String,
        second: String,
    },
    InvalidField { field: String, reason: &'static str },
    SecretRefused { name: String },
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownField { field } => write!(formatter, "unknown field {field}"),
            Self::DuplicateField { field } => write!(formatter, "duplicate field {field}"),
            Self::AliasConflict { field, first, second } => write!(
                formatter,
                "conflicting aliases for {field}: {first} versus {second}"
            ),
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid field {field}: {reason}")
            }
            Self::SecretRefused { name } => {
                write!(formatter, "refused secret-bearing name {name}")
            }
        }
    }
}

impl std::error::Error for OverlayError {}

/// Typed runtime override. Every entry is optional; `None` leaves the lower
/// layers untouched. Purpose backend uses two levels: outer `None` leaves the
/// override alone, `Some(None)` clears it, `Some(Some(_))` sets it.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimePatch {
    pub mode: Option<GlobalMode>,
    pub default_backend: Option<BackendId>,
    pub purpose_enabled: [Option<bool>; Purpose::COUNT],
    pub purpose_backend: [Option<Option<BackendId>>; Purpose::COUNT],
    pub backend_enabled: [Option<bool>; BackendId::COUNT],
    pub backend_account: [Option<AccountHandle>; BackendId::COUNT],
    pub backend_concurrency: [Option<u32>; BackendId::COUNT],
    pub transforms_enabled: Option<bool>,
    pub transforms_max_concurrency: Option<u32>,
    pub transforms_max_decoded_bytes: Option<u64>,
    pub transforms_max_output_bytes: Option<u64>,
    pub transforms_queue_capacity: Option<u32>,
    pub transforms_deadline_secs: Option<u64>,
    pub scheduler_foreground_limit: Option<u32>,
    pub scheduler_presence_limit: Option<u32>,
    pub scheduler_background_limit: Option<u32>,
    pub scheduler_max_pending_jobs: Option<u32>,
    pub scheduler_max_pending_bytes: Option<u64>,
    pub scheduler_queue_age_secs: Option<u64>,
    pub scheduler_job_deadline_secs: Option<u64>,
    pub scheduler_stage_deadline_secs: Option<u64>,
    pub request_max_prompt_chars: Option<usize>,
    pub request_max_references: Option<usize>,
    pub request_max_transform_steps: Option<usize>,
    pub request_max_composite_layers: Option<usize>,
}

impl RuntimePatch {
    /// No overrides; every lower layer stands.
    pub fn empty() -> Self {
        Self {
            mode: None,
            default_backend: None,
            purpose_enabled: [None; Purpose::COUNT],
            purpose_backend: [None; Purpose::COUNT],
            backend_enabled: [None; BackendId::COUNT],
            backend_account: std::array::from_fn(|_| None),
            backend_concurrency: [None; BackendId::COUNT],
            transforms_enabled: None,
            transforms_max_concurrency: None,
            transforms_max_decoded_bytes: None,
            transforms_max_output_bytes: None,
            transforms_queue_capacity: None,
            transforms_deadline_secs: None,
            scheduler_foreground_limit: None,
            scheduler_presence_limit: None,
            scheduler_background_limit: None,
            scheduler_max_pending_jobs: None,
            scheduler_max_pending_bytes: None,
            scheduler_queue_age_secs: None,
            scheduler_job_deadline_secs: None,
            scheduler_stage_deadline_secs: None,
            request_max_prompt_chars: None,
            request_max_references: None,
            request_max_transform_steps: None,
            request_max_composite_layers: None,
        }
    }
}

/// Resolve a validated [`ImageConfig`] from supplied file text, supplied
/// environment pairs, and a typed runtime patch. Reads nothing ambient: the
/// caller supplies every input. Any error leaves no partial result behind.
pub fn resolve(
    config_text: &str,
    env: &[(&str, &str)],
    runtime: &RuntimePatch,
    revision: u64,
) -> Result<ResolvedConfig, OverlayError> {
    let mut config = ImageConfig::defaults().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => {
            OverlayError::InvalidField {
                field: field.to_owned(),
                reason,
            }
        }
    })?;
    let mut provenance = Provenance {
        revision,
        fields: shared::tracked_fields(),
        ignored: Vec::new(),
    };
    file_layer::apply_file_layer(config_text, &mut config, &mut provenance)?;
    env_layer::apply_env_layer(env, &mut config, &mut provenance)?;
    apply_patch(runtime, &mut config, &mut provenance)?;
    config.validate().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => {
            OverlayError::InvalidField {
                field: field.to_owned(),
                reason,
            }
        }
    })?;
    Ok(ResolvedConfig { config, provenance })
}

/// Apply a runtime patch all or nothing: an invalid patch returns an error
/// and the base configuration is returned unchanged by value.
pub fn apply_runtime(
    base: &ImageConfig,
    patch: &RuntimePatch,
) -> Result<ImageConfig, OverlayError> {
    let mut next = base.clone();
    let mut provenance = Provenance {
        revision: 0,
        fields: shared::tracked_fields(),
        ignored: Vec::new(),
    };
    apply_patch(patch, &mut next, &mut provenance)?;
    next.validate().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => {
            OverlayError::InvalidField {
                field: field.to_owned(),
                reason,
            }
        }
    })?;
    Ok(next)
}

fn apply_patch(
    patch: &RuntimePatch,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    if let Some(mode) = patch.mode {
        config.mode = mode;
        shared::note(provenance, "mode", OverlaySource::Runtime);
    }
    if let Some(backend) = patch.default_backend {
        config.default_backend = backend;
        shared::note(provenance, "default_backend", OverlaySource::Runtime);
    }
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if let Some(enabled) = patch.purpose_enabled[index] {
            config.purposes[index].enabled = enabled;
            shared::note(
                provenance,
                &format!("purposes.{}.enabled", purpose.label()),
                OverlaySource::Runtime,
            );
        }
        if let Some(backend) = patch.purpose_backend[index] {
            config.purposes[index].backend_override = backend;
            shared::note(
                provenance,
                &format!("purposes.{}.backend_override", purpose.label()),
                OverlaySource::Runtime,
            );
        }
    }
    for index in 0..BackendId::COUNT {
        let label = shared::backend_label_from_index(index);
        if let Some(enabled) = patch.backend_enabled[index] {
            config.backends[index].enabled = enabled;
            shared::note(
                provenance,
                &format!("backends.{label}.enabled"),
                OverlaySource::Runtime,
            );
        }
        if let Some(account) = patch.backend_account[index].clone() {
            config.backends[index].account = account;
            shared::note(
                provenance,
                &format!("backends.{label}.account"),
                OverlaySource::Runtime,
            );
        }
        if let Some(max_jobs) = patch.backend_concurrency[index] {
            config.backends[index].max_concurrent_jobs = max_jobs;
            shared::note(
                provenance,
                &format!("backends.{label}.max_concurrent_jobs"),
                OverlaySource::Runtime,
            );
        }
    }
    let transforms = &mut config.transforms;
    if let Some(value) = patch.transforms_enabled {
        transforms.enabled = value;
        shared::note(provenance, "transforms.enabled", OverlaySource::Runtime);
    }
    if let Some(value) = patch.transforms_max_concurrency {
        transforms.max_concurrency = value;
        shared::note(
            provenance,
            "transforms.max_concurrency",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.transforms_max_decoded_bytes {
        transforms.max_decoded_bytes = value;
        shared::note(
            provenance,
            "transforms.max_decoded_bytes",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.transforms_max_output_bytes {
        transforms.max_output_bytes = value;
        shared::note(
            provenance,
            "transforms.max_output_bytes",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.transforms_queue_capacity {
        transforms.queue_capacity = value;
        shared::note(
            provenance,
            "transforms.queue_capacity",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.transforms_deadline_secs {
        transforms.deadline_secs = value;
        shared::note(
            provenance,
            "transforms.deadline_secs",
            OverlaySource::Runtime,
        );
    }
    let scheduler = &mut config.scheduler;
    if let Some(value) = patch.scheduler_foreground_limit {
        scheduler.foreground_limit = value;
        shared::note(
            provenance,
            "scheduler.foreground_limit",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.scheduler_presence_limit {
        scheduler.presence_limit = value;
        shared::note(provenance, "scheduler.presence_limit", OverlaySource::Runtime);
    }
    if let Some(value) = patch.scheduler_background_limit {
        scheduler.background_limit = value;
        shared::note(
            provenance,
            "scheduler.background_limit",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.scheduler_max_pending_jobs {
        scheduler.max_pending_jobs = value;
        shared::note(
            provenance,
            "scheduler.max_pending_jobs",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.scheduler_max_pending_bytes {
        scheduler.max_pending_bytes = value;
        shared::note(
            provenance,
            "scheduler.max_pending_bytes",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.scheduler_queue_age_secs {
        scheduler.queue_age_secs = value;
        shared::note(provenance, "scheduler.queue_age_secs", OverlaySource::Runtime);
    }
    if let Some(value) = patch.scheduler_job_deadline_secs {
        scheduler.job_deadline_secs = value;
        shared::note(
            provenance,
            "scheduler.job_deadline_secs",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.scheduler_stage_deadline_secs {
        scheduler.stage_deadline_secs = value;
        shared::note(
            provenance,
            "scheduler.stage_deadline_secs",
            OverlaySource::Runtime,
        );
    }
    let limits = &mut config.request_limits;
    if let Some(value) = patch.request_max_prompt_chars {
        limits.max_prompt_chars = value;
        shared::note(
            provenance,
            "request_limits.max_prompt_chars",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.request_max_references {
        limits.max_references = value;
        shared::note(
            provenance,
            "request_limits.max_references",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.request_max_transform_steps {
        limits.max_transform_steps = value;
        shared::note(
            provenance,
            "request_limits.max_transform_steps",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.request_max_composite_layers {
        limits.max_composite_layers = value;
        shared::note(
            provenance,
            "request_limits.max_composite_layers",
            OverlaySource::Runtime,
        );
    }
    Ok(())
}

/// Set one runtime-patch entry from a dotted path and a raw string, for the
/// administrative surface. Unknown paths and invalid values are refused and
/// leave the patch untouched.
pub fn parse_runtime_value(
    patch: &mut RuntimePatch,
    dotted: &str,
    value: &str,
) -> Result<(), OverlayError> {
    let parts: Vec<&str> = dotted.split('.').collect();
    match parts.as_slice() {
        ["mode"] => {
            let Some(mode) = parse::parse_global_mode(value) else {
                return Err(shared::invalid(dotted, "unknown mode"));
            };
            patch.mode = Some(mode);
        }
        ["default_backend"] => {
            let Some(backend) = parse::parse_backend_id(value, false) else {
                return Err(shared::invalid(dotted, "unknown backend"));
            };
            patch.default_backend = Some(backend);
        }
        ["purposes", purpose_name, "enabled"] => {
            let Some(purpose) = parse::parse_purpose(purpose_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            patch.purpose_enabled[purpose.index()] = Some(need_bool(dotted, value)?);
        }
        ["purposes", purpose_name, "backend"] => {
            let Some(purpose) = parse::parse_purpose(purpose_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.purpose_backend[purpose.index()] = Some(None);
            } else {
                let Some(backend) = parse::parse_backend_id(value, false) else {
                    return Err(shared::invalid(dotted, "unknown backend"));
                };
                patch.purpose_backend[purpose.index()] = Some(Some(backend));
            }
        }
        ["backends", backend_name, "enabled"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            patch.backend_enabled[backend.index()] = Some(need_bool(dotted, value)?);
        }
        ["backends", backend_name, "account"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            patch.backend_account[backend.index()] = Some(need_account(dotted, value)?);
        }
        ["backends", backend_name, "max_concurrent_jobs"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            patch.backend_concurrency[backend.index()] = Some(need_u32(dotted, value)?);
        }
        ["transforms", key] => match *key {
            "enabled" => patch.transforms_enabled = Some(need_bool(dotted, value)?),
            "max_concurrency" => {
                patch.transforms_max_concurrency = Some(need_u32(dotted, value)?);
            }
            "max_decoded_bytes" => {
                patch.transforms_max_decoded_bytes = Some(need_u64(dotted, value)?);
            }
            "max_output_bytes" => {
                patch.transforms_max_output_bytes = Some(need_u64(dotted, value)?);
            }
            "queue_capacity" => {
                patch.transforms_queue_capacity = Some(need_u32(dotted, value)?);
            }
            "deadline_secs" => {
                patch.transforms_deadline_secs = Some(need_u64(dotted, value)?);
            }
            _ => {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            }
        },
        ["scheduler", key] => match *key {
            "foreground_limit" => {
                patch.scheduler_foreground_limit = Some(need_u32(dotted, value)?);
            }
            "presence_limit" => {
                patch.scheduler_presence_limit = Some(need_u32(dotted, value)?);
            }
            "background_limit" => {
                patch.scheduler_background_limit = Some(need_u32(dotted, value)?);
            }
            "max_pending_jobs" => {
                patch.scheduler_max_pending_jobs = Some(need_u32(dotted, value)?);
            }
            "max_pending_bytes" => {
                patch.scheduler_max_pending_bytes = Some(need_u64(dotted, value)?);
            }
            "queue_age_secs" => {
                patch.scheduler_queue_age_secs = Some(need_u64(dotted, value)?);
            }
            "job_deadline_secs" => {
                patch.scheduler_job_deadline_secs = Some(need_u64(dotted, value)?);
            }
            "stage_deadline_secs" => {
                patch.scheduler_stage_deadline_secs = Some(need_u64(dotted, value)?);
            }
            _ => {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            }
        },
        ["request_limits", key] => match *key {
            "max_prompt_chars" => {
                patch.request_max_prompt_chars = Some(need_usize(dotted, value)?);
            }
            "max_references" => {
                patch.request_max_references = Some(need_usize(dotted, value)?);
            }
            "max_transform_steps" => {
                patch.request_max_transform_steps = Some(need_usize(dotted, value)?);
            }
            "max_composite_layers" => {
                patch.request_max_composite_layers = Some(need_usize(dotted, value)?);
            }
            _ => {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            }
        },
        _ => {
            return Err(OverlayError::UnknownField {
                field: dotted.to_owned(),
            });
        }
    }
    Ok(())
}

fn need_bool(dotted: &str, value: &str) -> Result<bool, OverlayError> {
    match parse::parse_overlay_bool(value) {
        Ok(Some(flag)) => Ok(flag),
        _ => Err(shared::invalid(dotted, "expected on, off, true, or false")),
    }
}

fn need_u64(dotted: &str, value: &str) -> Result<u64, OverlayError> {
    match parse::parse_count(value) {
        Ok(Some(count)) => Ok(count),
        _ => Err(shared::invalid(dotted, "expected an unsigned integer")),
    }
}

fn need_u32(dotted: &str, value: &str) -> Result<u32, OverlayError> {
    let count = need_u64(dotted, value)?;
    u32::try_from(count).map_err(|_| shared::invalid(dotted, "integer outside range"))
}

fn need_usize(dotted: &str, value: &str) -> Result<usize, OverlayError> {
    let count = need_u64(dotted, value)?;
    usize::try_from(count).map_err(|_| shared::invalid(dotted, "integer outside range"))
}

fn need_account(dotted: &str, value: &str) -> Result<AccountHandle, OverlayError> {
    AccountHandle::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => {
            shared::invalid(dotted, reason)
        }
    })
}
