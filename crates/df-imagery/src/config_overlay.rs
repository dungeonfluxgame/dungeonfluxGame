//! Pure image-configuration overlay: supplied text, pairs and patch in.
//!
//! Resolves one validated [`ImageConfig`] from supplied layers above the
//! built-in defaults. Precedence is defaults, then operator file text, then
//! legacy launcher environment pairs, then new `DF_IMAGERY_*` pairs, then a
//! typed runtime patch. The crate-native configuration owner
//! (`crate::config_control`) applies an explicit launch patch between the
//! new-environment layer and the runtime patch and records it as
//! [`OverlaySource::Launch`]. Within the file layer contradictory new and
//! legacy spellings for one field conflict only when their values differ;
//! legacy and new environment are distinct layers, so a present new value
//! wins silently over a legacy one. Across layers a higher layer wins
//! silently over a lower one and records its provenance.
//!
//! This module owns parsing and precedence only. It never reads ambient
//! environment, the filesystem, or the network; it never stores, logs, or
//! echoes secrets or credentials. Nonsecret supplier models, executable
//! selections, price-file references and endpoint bases are typed settings
//! consumed by private executors; credentials stay at deployment
//! composition. Recognized owned fields with no [`ImageConfig`] counterpart
//! are refused before yielding a config; only the legacy creator
//! alternate-route hint is acknowledged by name in [`Provenance::ignored`],
//! never applied. Disabling a backend or purpose never reroutes work: the
//! recorded backend selection is kept as written and the later service
//! answers `Disabled` for affected demand.
//!
//! File text parses through the pinned `toml` crate (root owns the manifest
//! entry); environment pairs and the runtime patch use only the standard
//! library.
//!
//! Implementation lives in sibling modules beside this file; Rust resolves
//! `config_overlay/foo.rs` for `mod foo` declared here, so the file and its
//! directory coexist.

mod analysis;
mod env_layer;
mod file_layer;
mod parse;
mod segmentation;
mod shared;

pub use analysis::{AnalysisRouteFields, AnalysisRoutePatch};
pub use segmentation::{SegmentationRouteFields, SegmentationRoutePatch};
pub use shared::{BodyField, ProfileEdit};

use crate::config::{
    AccountHandle, AccountingMode, ApiEndpoint, BackendId, CliProfileRevision, CodexExecutable,
    GenerationQuality, GlobalMode, ImageConfig, JobWorkDir, ModelName, NamedOutputProfile,
    OutputProfileName, PriceFileRef,
};
use crate::output::LossyQuality;
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
    Launch,
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
    UnknownField {
        field: String,
    },
    DuplicateField {
        field: String,
    },
    AliasConflict {
        field: String,
        first: String,
        second: String,
    },
    InvalidField {
        field: String,
        reason: &'static str,
    },
    SecretRefused {
        name: String,
    },
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownField { field } => write!(formatter, "unknown field {field}"),
            Self::DuplicateField { field } => write!(formatter, "duplicate field {field}"),
            Self::AliasConflict {
                field,
                first,
                second,
            } => write!(
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
/// override alone, `Some(None)` clears it, `Some(Some(_))` sets it. The same
/// two levels apply to backend model/endpoint, purpose model/quality/output profile,
/// the Codex agent model, the accounting price-file reference, and the
/// provider generation-quality default (`Some(None)` clears it back to the
/// provider deciding). Profile bodies merge per edited field over the seeded
/// body: `output_profile_edits` names `none` for a generated dimension to
/// drop the provider side, for `quality` to select lossless WebP, and for
/// `matte` to clear the flatten color.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimePatch {
    pub mode: Option<GlobalMode>,
    pub analysis_classification: Option<AnalysisRoutePatch>,
    pub analysis_inspection: Option<AnalysisRoutePatch>,
    pub analysis_head_locations: Option<AnalysisRoutePatch>,
    pub analysis_staging_identity: Option<AnalysisRoutePatch>,
    pub analysis_staging_facing: Option<AnalysisRoutePatch>,
    pub analysis_segmentation: Option<SegmentationRoutePatch>,
    pub analysis_segmentation_preparation_max_pixels: Option<u64>,
    pub analysis_segmentation_preparation_max_picture_bytes: Option<u64>,
    pub analysis_segmentation_preparation_max_request_bytes: Option<u64>,
    pub analysis_preparation_max_pixels: Option<u64>,
    pub analysis_preparation_max_picture_bytes: Option<u64>,
    pub analysis_preparation_max_request_bytes: Option<u64>,
    pub default_backend: Option<BackendId>,
    pub purpose_enabled: [Option<bool>; Purpose::COUNT],
    pub purpose_account_override: [Option<Option<AccountHandle>>; Purpose::COUNT],
    pub purpose_moderation_override:
        [Option<Option<crate::request::ProviderModeration>>; Purpose::COUNT],
    pub purpose_quality_override: [Option<Option<GenerationQuality>>; Purpose::COUNT],
    pub purpose_model_override: [Option<Option<ModelName>>; Purpose::COUNT],
    pub purpose_backend: [Option<Option<BackendId>>; Purpose::COUNT],
    pub purpose_output_profile: [Option<Option<OutputProfileName>>; Purpose::COUNT],
    pub backend_enabled: [Option<bool>; BackendId::COUNT],
    pub backend_account: [Option<AccountHandle>; BackendId::COUNT],
    pub backend_concurrency: [Option<u32>; BackendId::COUNT],
    pub backend_generation_model: [Option<Option<ModelName>>; BackendId::COUNT],
    pub backend_edit_model: [Option<Option<ModelName>>; BackendId::COUNT],
    pub backend_alpha_model: [Option<Option<ModelName>>; BackendId::COUNT],
    pub backend_model: [Option<Option<ModelName>>; BackendId::COUNT],
    pub backend_endpoint: [Option<Option<ApiEndpoint>>; BackendId::COUNT],
    pub codex_executable: Option<CodexExecutable>,
    pub codex_workdir: Option<JobWorkDir>,
    pub codex_cli_profile: Option<CliProfileRevision>,
    pub codex_agent_model: Option<Option<ModelName>>,
    pub codex_max_stdout_bytes: Option<u64>,
    pub codex_max_stderr_bytes: Option<u64>,
    pub codex_max_message_bytes: Option<u64>,
    pub creator_bfl_account: Option<Option<AccountHandle>>,
    pub quota_enabled: Option<bool>,
    pub quota_cooldown_ms: Option<u64>,
    pub quota_maximum_accounts: Option<u32>,
    pub default_output_profile: Option<OutputProfileName>,
    pub output_profile_edits: Vec<ProfileEdit>,
    pub output_profiles: Option<Vec<NamedOutputProfile>>,
    pub retry_max_attempts: Option<u32>,
    pub retry_deadline_secs: Option<u64>,
    pub retry_backoff_secs: Option<u64>,
    pub accounting_mode: Option<AccountingMode>,
    pub accounting_price_file: Option<Option<PriceFileRef>>,
    pub quality_jpeg_quality: Option<LossyQuality>,
    pub quality_provider_quality: Option<Option<GenerationQuality>>,
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
            analysis_classification: None,
            analysis_inspection: None,
            analysis_head_locations: None,
            analysis_staging_identity: None,
            analysis_staging_facing: None,
            analysis_segmentation: None,
            analysis_segmentation_preparation_max_pixels: None,
            analysis_segmentation_preparation_max_picture_bytes: None,
            analysis_segmentation_preparation_max_request_bytes: None,
            analysis_preparation_max_pixels: None,
            analysis_preparation_max_picture_bytes: None,
            analysis_preparation_max_request_bytes: None,
            default_backend: None,
            purpose_enabled: [None; Purpose::COUNT],
            purpose_account_override: std::array::from_fn(|_| None),
            purpose_quality_override: [None; Purpose::COUNT],
            purpose_moderation_override: [None; Purpose::COUNT],
            purpose_model_override: std::array::from_fn(|_| None),
            purpose_backend: [None; Purpose::COUNT],
            purpose_output_profile: std::array::from_fn(|_| None),
            backend_enabled: [None; BackendId::COUNT],
            backend_account: std::array::from_fn(|_| None),
            backend_concurrency: [None; BackendId::COUNT],
            backend_generation_model: std::array::from_fn(|_| None),
            backend_edit_model: std::array::from_fn(|_| None),
            backend_alpha_model: std::array::from_fn(|_| None),
            backend_model: std::array::from_fn(|_| None),
            backend_endpoint: std::array::from_fn(|_| None),
            codex_executable: None,
            codex_workdir: None,
            codex_cli_profile: None,
            codex_agent_model: None,
            codex_max_stdout_bytes: None,
            codex_max_stderr_bytes: None,
            codex_max_message_bytes: None,
            creator_bfl_account: None,
            quota_enabled: None,
            quota_cooldown_ms: None,
            quota_maximum_accounts: None,
            default_output_profile: None,
            output_profile_edits: Vec::new(),
            output_profiles: None,
            retry_max_attempts: None,
            retry_deadline_secs: None,
            retry_backoff_secs: None,
            accounting_mode: None,
            accounting_price_file: None,
            quality_jpeg_quality: None,
            quality_provider_quality: None,
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
    let defaults = ImageConfig::defaults().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
    })?;
    resolve_with_defaults(defaults, config_text, env, runtime, revision)
}

/// Resolves operator layers above validated composition defaults, retaining Default provenance.
pub fn resolve_with_defaults(
    mut config: ImageConfig,
    config_text: &str,
    env: &[(&str, &str)],
    runtime: &RuntimePatch,
    revision: u64,
) -> Result<ResolvedConfig, OverlayError> {
    config.validate().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
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
        crate::config::ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
    })?;
    config.validate_codex_lines().map_err(|error| match error {
        crate::config::ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
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
        crate::config::ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
    })?;
    Ok(next)
}

/// Apply an explicit launch patch for the crate-native configuration owner.
/// Field mapping is shared with [`apply_patch`]; entries it touches are
/// retagged from [`OverlaySource::Runtime`] to [`OverlaySource::Launch`].
/// The caller must hold no prior runtime entries (a base provenance resolved
/// with an empty runtime patch satisfies this), so every `Runtime` entry
/// found here came from this patch.
pub(super) fn apply_launch_patch(
    patch: &RuntimePatch,
    config: &mut ImageConfig,
    provenance: &mut Provenance,
) -> Result<(), OverlayError> {
    apply_patch(patch, config, provenance)?;
    for entry in &mut provenance.fields {
        if entry.source == OverlaySource::Runtime {
            entry.source = OverlaySource::Launch;
        }
    }
    Ok(())
}

pub(super) fn apply_patch(
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
        shared::note(
            provenance,
            "scheduler.presence_limit",
            OverlaySource::Runtime,
        );
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
        shared::note(
            provenance,
            "scheduler.queue_age_secs",
            OverlaySource::Runtime,
        );
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
    for index in 0..Purpose::COUNT {
        let Some(purpose) = Purpose::from_index(index) else {
            continue;
        };
        if let Some(value) = patch.purpose_moderation_override[index] {
            config.purposes[index].moderation_override = value;
            shared::note(
                provenance,
                &format!("purposes.{}.moderation_override", purpose.label()),
                OverlaySource::Runtime,
            );
        }
        if let Some(value) = patch.purpose_account_override[index].clone() {
            config.purposes[index].account_override = value;
            shared::note(
                provenance,
                &format!("purposes.{}.account_override", purpose.label()),
                OverlaySource::Runtime,
            );
        }
        if let Some(value) = patch.purpose_quality_override[index] {
            config.purposes[index].quality_override = value;
            shared::note(
                provenance,
                &format!("purposes.{}.quality_override", purpose.label()),
                OverlaySource::Runtime,
            );
        }
        if let Some(value) = patch.purpose_model_override[index].clone() {
            config.purposes[index].model_override = value;
            shared::note(
                provenance,
                &format!("purposes.{}.model_override", purpose.label()),
                OverlaySource::Runtime,
            );
        }
        if let Some(selected) = patch.purpose_output_profile[index].clone() {
            config.purposes[index].output_profile = selected;
            shared::note(
                provenance,
                &format!("purposes.{}.output_profile", purpose.label()),
                OverlaySource::Runtime,
            );
        }
    }
    for index in 0..BackendId::COUNT {
        let label = shared::backend_label_from_index(index);
        if let Some(model) = patch.backend_model[index].clone() {
            config.backends[index].model = model;
            shared::note(
                provenance,
                &format!("backends.{label}.model"),
                OverlaySource::Runtime,
            );
        }
        if let Some(model) = patch.backend_generation_model[index].clone() {
            config.backends[index].generation_model = model;
            shared::note(
                provenance,
                &format!("backends.{label}.generation_model"),
                OverlaySource::Runtime,
            );
        }
        if let Some(model) = patch.backend_edit_model[index].clone() {
            config.backends[index].edit_model = model;
            shared::note(
                provenance,
                &format!("backends.{label}.edit_model"),
                OverlaySource::Runtime,
            );
        }
        if let Some(model) = patch.backend_alpha_model[index].clone() {
            config.backends[index].alpha_model = model;
            shared::note(
                provenance,
                &format!("backends.{label}.alpha_model"),
                OverlaySource::Runtime,
            );
        }
        if let Some(endpoint) = patch.backend_endpoint[index].clone() {
            config.backends[index].endpoint = endpoint;
            shared::note(
                provenance,
                &format!("backends.{label}.endpoint"),
                OverlaySource::Runtime,
            );
        }
    }
    for (field, slot, value) in [
        (
            "max_pixels",
            &mut config.analysis.preparation.max_pixels,
            patch.analysis_preparation_max_pixels,
        ),
        (
            "max_picture_bytes",
            &mut config.analysis.preparation.max_picture_bytes,
            patch.analysis_preparation_max_picture_bytes,
        ),
        (
            "max_request_bytes",
            &mut config.analysis.preparation.max_request_bytes,
            patch.analysis_preparation_max_request_bytes,
        ),
    ] {
        if let Some(value) = value {
            *slot = value;
            shared::note(
                provenance,
                &format!("analysis.preparation.{field}"),
                OverlaySource::Runtime,
            );
        }
    }
    for (field, slot, value) in [
        (
            "max_pixels",
            &mut config.analysis.segmentation_preparation.max_pixels,
            patch.analysis_segmentation_preparation_max_pixels,
        ),
        (
            "max_picture_bytes",
            &mut config.analysis.segmentation_preparation.max_picture_bytes,
            patch.analysis_segmentation_preparation_max_picture_bytes,
        ),
        (
            "max_request_bytes",
            &mut config.analysis.segmentation_preparation.max_request_bytes,
            patch.analysis_segmentation_preparation_max_request_bytes,
        ),
    ] {
        if let Some(value) = value {
            *slot = value;
            shared::note(
                provenance,
                &format!("analysis.segmentation_preparation.{field}"),
                OverlaySource::Runtime,
            );
        }
    }
    if let Some(edit) = &patch.analysis_segmentation {
        segmentation::apply_route(&mut config.analysis.segmentation, edit, provenance, |_| {
            OverlaySource::Runtime
        })?;
    }
    for (stage, slot, edit) in [
        (
            "classification",
            &mut config.analysis.classification,
            &patch.analysis_classification,
        ),
        (
            "inspection",
            &mut config.analysis.inspection,
            &patch.analysis_inspection,
        ),
        (
            "head_locations",
            &mut config.analysis.head_locations,
            &patch.analysis_head_locations,
        ),
        (
            "staging_identity",
            &mut config.analysis.staging_identity,
            &patch.analysis_staging_identity,
        ),
        (
            "staging_facing",
            &mut config.analysis.staging_facing,
            &patch.analysis_staging_facing,
        ),
    ] {
        if let Some(edit) = edit {
            analysis::apply_route(slot, edit, stage, provenance, |_| OverlaySource::Runtime)?;
        }
    }
    if let Some(executable) = patch.codex_executable.clone() {
        config.codex.executable = executable;
        shared::note(provenance, "codex.executable", OverlaySource::Runtime);
    }
    if let Some(workdir) = patch.codex_workdir.clone() {
        config.codex.workdir = workdir;
        shared::note(provenance, "codex.workdir", OverlaySource::Runtime);
    }
    if let Some(revision) = patch.codex_cli_profile.clone() {
        config.codex.cli_profile = revision;
        shared::note(provenance, "codex.cli_profile", OverlaySource::Runtime);
    }
    if let Some(limit) = patch.codex_max_stdout_bytes {
        config.codex.max_stdout_bytes = limit;
        shared::note(provenance, "codex.max_stdout_bytes", OverlaySource::Runtime);
    }
    if let Some(limit) = patch.codex_max_stderr_bytes {
        config.codex.max_stderr_bytes = limit;
        shared::note(provenance, "codex.max_stderr_bytes", OverlaySource::Runtime);
    }
    if let Some(value) = &patch.creator_bfl_account {
        config.creator_bfl_account = value.clone();
        shared::note(provenance, "creator.bfl_account", OverlaySource::Runtime);
    }
    if let Some(value) = patch.quota_enabled {
        config.quota.enabled = value;
        shared::note(provenance, "quota.enabled", OverlaySource::Runtime);
    }
    if let Some(value) = patch.quota_cooldown_ms {
        config.quota.cooldown_ms = value;
        shared::note(provenance, "quota.cooldown_ms", OverlaySource::Runtime);
    }
    if let Some(value) = patch.quota_maximum_accounts {
        config.quota.maximum_accounts = value;
        shared::note(provenance, "quota.maximum_accounts", OverlaySource::Runtime);
    }
    if let Some(limit) = patch.codex_max_message_bytes {
        config.codex.max_message_bytes = limit;
        shared::note(
            provenance,
            "codex.max_message_bytes",
            OverlaySource::Runtime,
        );
    }
    if let Some(model) = patch.codex_agent_model.clone() {
        config.codex.agent_model = model;
        shared::note(provenance, "codex.agent_model", OverlaySource::Runtime);
    }
    if let Some(name) = patch.default_output_profile.clone() {
        config.default_output_profile = name;
        shared::note(
            provenance,
            "outputs.default_profile",
            OverlaySource::Runtime,
        );
    }
    if let Some(table) = patch.output_profiles.clone() {
        for entry in table {
            shared::merge_profile_body(
                config,
                provenance,
                entry.name.clone(),
                entry.profile,
                OverlaySource::Runtime,
                &[],
            );
        }
    }
    for edit in &patch.output_profile_edits {
        let section = format!("runtime.outputs.{}", edit.name.as_str());
        let seeded = shared::seed_profile_body(config, &edit.name)?;
        let body = shared::profile_body(seeded, edit, &section)?;
        let fields: Vec<String> = edit.fields.keys().cloned().collect();
        shared::merge_profile_body(
            config,
            provenance,
            edit.name.clone(),
            body,
            edit.source.clone(),
            &fields,
        );
    }
    if let Some(value) = patch.retry_max_attempts {
        config.retry.max_attempts = value;
        shared::note(provenance, "retry.max_attempts", OverlaySource::Runtime);
    }
    if let Some(value) = patch.retry_deadline_secs {
        config.retry.retry_deadline_secs = value;
        shared::note(
            provenance,
            "retry.retry_deadline_secs",
            OverlaySource::Runtime,
        );
    }
    if let Some(value) = patch.retry_backoff_secs {
        config.retry.backoff_secs = value;
        shared::note(provenance, "retry.backoff_secs", OverlaySource::Runtime);
    }
    if let Some(mode) = patch.accounting_mode {
        config.accounting.mode = mode;
        shared::note(provenance, "accounting.mode", OverlaySource::Runtime);
    }
    if let Some(reference) = patch.accounting_price_file.clone() {
        config.accounting.price_file = reference;
        shared::note(provenance, "accounting.price_file", OverlaySource::Runtime);
    }
    if let Some(quality) = patch.quality_jpeg_quality {
        config.quality.jpeg_quality = quality;
        shared::note(provenance, "quality.jpeg_quality", OverlaySource::Runtime);
    }
    if let Some(quality) = patch.quality_provider_quality {
        config.quality.provider_quality = quality;
        shared::note(
            provenance,
            "quality.provider_quality",
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

/// Allocation guards for the administrative runtime surface, not product
/// ceilings: every recognized path is short and every valid scalar is small,
/// so a larger input is refused before any split, Vec, or error string grows.
const MAXIMUM_RUNTIME_PATH_CHARS: usize = 128;
/// Longer raw values than this cannot be a valid mode, backend, or handle.
const MAXIMUM_RUNTIME_VALUE_BYTES: usize = 4096;

/// Set one runtime-patch entry from a dotted path and a raw string, for the
/// administrative surface. Unknown paths and invalid values are refused and
/// leave the patch untouched.
pub fn parse_runtime_value(
    patch: &mut RuntimePatch,
    dotted: &str,
    value: &str,
) -> Result<(), OverlayError> {
    if dotted.len() > MAXIMUM_RUNTIME_PATH_CHARS {
        return Err(shared::invalid("runtime", "path exceeds bound"));
    }
    if value.len() > MAXIMUM_RUNTIME_VALUE_BYTES {
        return Err(shared::invalid("runtime", "value exceeds bound"));
    }
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
        ["purposes", purpose_name, "moderation_override"] => {
            let purpose =
                parse::parse_purpose(purpose_name).ok_or_else(|| OverlayError::UnknownField {
                    field: dotted.to_owned(),
                })?;
            let selected = if value == "none" {
                None
            } else {
                Some(
                    parse::parse_provider_moderation(value)
                        .ok_or_else(|| shared::invalid(dotted, "expected auto, low, or none"))?,
                )
            };
            patch.purpose_moderation_override[purpose.index()] = Some(selected);
        }
        ["purposes", purpose_name, "account_override"] => {
            let purpose =
                parse::parse_purpose(purpose_name).ok_or_else(|| OverlayError::UnknownField {
                    field: dotted.to_owned(),
                })?;
            patch.purpose_account_override[purpose.index()] = Some(if value == "none" {
                None
            } else {
                Some(need_account(dotted, value)?)
            });
        }
        ["purposes", purpose_name, "quality_override"] => {
            let purpose =
                parse::parse_purpose(purpose_name).ok_or_else(|| OverlayError::UnknownField {
                    field: dotted.to_owned(),
                })?;
            let selected = if value == "none" {
                None
            } else {
                Some(parse::parse_generation_quality(value).ok_or_else(|| {
                    shared::invalid(dotted, "expected low, medium, high, auto, or none")
                })?)
            };
            patch.purpose_quality_override[purpose.index()] = Some(selected);
        }
        ["purposes", purpose_name, "model_override"] => {
            let purpose =
                parse::parse_purpose(purpose_name).ok_or_else(|| OverlayError::UnknownField {
                    field: dotted.to_owned(),
                })?;
            let selected = if value == "none" {
                None
            } else {
                Some(need_model(dotted, value)?)
            };
            patch.purpose_model_override[purpose.index()] = Some(selected);
        }
        ["purposes", purpose_name, "output_profile"] => {
            let Some(purpose) = parse::parse_purpose(purpose_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.purpose_output_profile[purpose.index()] = Some(None);
            } else {
                let name = need_output_profile(dotted, value)?;
                patch.purpose_output_profile[purpose.index()] = Some(Some(name));
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
        ["backends", backend_name, "model"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.backend_model[backend.index()] = Some(None);
            } else {
                patch.backend_model[backend.index()] = Some(Some(need_model(dotted, value)?));
            }
        }
        ["backends", backend_name, "generation_model"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.backend_generation_model[backend.index()] = Some(None);
            } else {
                patch.backend_generation_model[backend.index()] =
                    Some(Some(need_model(dotted, value)?));
            }
        }
        ["backends", backend_name, "edit_model"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.backend_edit_model[backend.index()] = Some(None);
            } else {
                patch.backend_edit_model[backend.index()] = Some(Some(need_model(dotted, value)?));
            }
        }
        ["backends", backend_name, "alpha_model"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.backend_alpha_model[backend.index()] = Some(None);
            } else {
                patch.backend_alpha_model[backend.index()] = Some(Some(need_model(dotted, value)?));
            }
        }
        ["backends", backend_name, "endpoint"] => {
            let Some(backend) = parse::parse_backend_section(backend_name) else {
                return Err(OverlayError::UnknownField {
                    field: dotted.to_owned(),
                });
            };
            if value == "none" {
                patch.backend_endpoint[backend.index()] = Some(None);
            } else {
                patch.backend_endpoint[backend.index()] = Some(Some(need_endpoint(dotted, value)?));
            }
        }
        ["analysis", "preparation", "max_pixels"] => {
            patch.analysis_preparation_max_pixels = Some(need_u64(dotted, value)?)
        }
        ["analysis", "preparation", "max_picture_bytes"] => {
            patch.analysis_preparation_max_picture_bytes = Some(need_u64(dotted, value)?)
        }
        ["analysis", "preparation", "max_request_bytes"] => {
            patch.analysis_preparation_max_request_bytes = Some(need_u64(dotted, value)?)
        }
        ["analysis", "segmentation_preparation", "max_pixels"] => {
            patch.analysis_segmentation_preparation_max_pixels = Some(need_u64(dotted, value)?)
        }
        ["analysis", "segmentation_preparation", "max_picture_bytes"] => {
            patch.analysis_segmentation_preparation_max_picture_bytes =
                Some(need_u64(dotted, value)?)
        }
        ["analysis", "segmentation_preparation", "max_request_bytes"] => {
            patch.analysis_segmentation_preparation_max_request_bytes =
                Some(need_u64(dotted, value)?)
        }
        ["analysis", "segmentation"] => {
            segmentation::parse_slot(&mut patch.analysis_segmentation, None, dotted, value)?
        }
        ["analysis", "segmentation", field] => {
            segmentation::parse_slot(&mut patch.analysis_segmentation, Some(field), dotted, value)?
        }
        ["analysis", stage] => {
            let slot = analysis::route_slot(patch, stage, dotted)?;
            analysis::parse_slot(slot, None, dotted, value)?;
        }
        ["analysis", stage, field] => {
            let slot = analysis::route_slot(patch, stage, dotted)?;
            analysis::parse_slot(slot, Some(field), dotted, value)?;
        }
        ["codex", "executable"] => {
            patch.codex_executable = Some(need_executable(dotted, value)?);
        }
        ["codex", "workdir"] => {
            patch.codex_workdir = Some(need_workdir(dotted, value)?);
        }
        ["codex", "cli_profile"] => {
            patch.codex_cli_profile = Some(need_cli_profile(dotted, value)?);
        }
        ["codex", "max_stdout_bytes"] => {
            patch.codex_max_stdout_bytes = Some(need_u64(dotted, value)?);
        }
        ["codex", "max_stderr_bytes"] => {
            patch.codex_max_stderr_bytes = Some(need_u64(dotted, value)?);
        }
        ["creator", "bfl_account"] => {
            patch.creator_bfl_account = Some(if value == "none" {
                None
            } else {
                Some(need_account(dotted, value)?)
            });
        }
        ["quota", "enabled"] => patch.quota_enabled = Some(need_bool(dotted, value)?),
        ["quota", "cooldown_ms"] => patch.quota_cooldown_ms = Some(need_u64(dotted, value)?),
        ["quota", "maximum_accounts"] => {
            patch.quota_maximum_accounts = Some(need_u32(dotted, value)?)
        }
        ["codex", "max_message_bytes"] => {
            patch.codex_max_message_bytes = Some(need_u64(dotted, value)?);
        }
        ["codex", "agent_model"] => {
            if value == "none" {
                patch.codex_agent_model = Some(None);
            } else {
                patch.codex_agent_model = Some(Some(need_model(dotted, value)?));
            }
        }
        ["codex", _] => {
            return Err(OverlayError::UnknownField {
                field: dotted.to_owned(),
            });
        }
        ["outputs", "default_profile"] => {
            patch.default_output_profile = Some(need_output_profile(dotted, value)?);
        }
        ["outputs", name, key] => {
            let profile = need_output_profile(dotted, name)?;
            let body_field = match *key {
                "generated_width_px"
                | "generated_height_px"
                | "width_px"
                | "height_px"
                | "quality" => {
                    if value == "none" {
                        BodyField::Text("none".to_owned())
                    } else {
                        BodyField::Count(need_u64(dotted, value)?)
                    }
                }
                "format" | "alpha" | "matte" => BodyField::Text(value.to_owned()),
                _ => {
                    return Err(OverlayError::UnknownField {
                        field: dotted.to_owned(),
                    });
                }
            };
            let position = patch
                .output_profile_edits
                .iter()
                .position(|edit| edit.name == profile);
            let edit = match position {
                Some(index) => &mut patch.output_profile_edits[index],
                None => {
                    patch.output_profile_edits.push(ProfileEdit {
                        name: profile.clone(),
                        source: OverlaySource::Runtime,
                        fields: std::collections::BTreeMap::new(),
                    });
                    let Some(edit) = patch.output_profile_edits.last_mut() else {
                        return Err(shared::invalid(dotted, "profile edit was lost"));
                    };
                    edit
                }
            };
            edit.fields.insert((*key).to_owned(), body_field);
        }
        ["retry", "max_attempts"] => {
            patch.retry_max_attempts = Some(need_u32(dotted, value)?);
        }
        ["retry", "retry_deadline_secs"] => {
            patch.retry_deadline_secs = Some(need_u64(dotted, value)?);
        }
        ["retry", "backoff_secs"] => {
            patch.retry_backoff_secs = Some(need_u64(dotted, value)?);
        }
        ["retry", _] => {
            return Err(OverlayError::UnknownField {
                field: dotted.to_owned(),
            });
        }
        ["accounting", "mode"] => {
            let Some(mode) = parse::parse_accounting_mode(value) else {
                return Err(shared::invalid(dotted, "unknown accounting mode"));
            };
            patch.accounting_mode = Some(mode);
        }
        ["accounting", "price_file"] => {
            if value == "none" {
                patch.accounting_price_file = Some(None);
            } else {
                patch.accounting_price_file = Some(Some(need_price_file(dotted, value)?));
            }
        }
        ["accounting", _] => {
            return Err(OverlayError::UnknownField {
                field: dotted.to_owned(),
            });
        }
        ["quality", "jpeg_quality"] => {
            let count = need_u64(dotted, value)?;
            let value = u8::try_from(count)
                .map_err(|_| shared::invalid(dotted, "integer outside range"))?;
            let lossy = LossyQuality::new(value)
                .map_err(|_| shared::invalid(dotted, "a quality is 1 to 100"))?;
            patch.quality_jpeg_quality = Some(lossy);
        }
        ["quality", "provider_quality"] => {
            if value == "none" {
                patch.quality_provider_quality = Some(None);
            } else {
                let quality = parse::parse_generation_quality(value).ok_or_else(|| {
                    shared::invalid(dotted, "expected low, medium, high, auto, or none")
                })?;
                patch.quality_provider_quality = Some(Some(quality));
            }
        }
        ["quality", _] => {
            return Err(OverlayError::UnknownField {
                field: dotted.to_owned(),
            });
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
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_model(dotted: &str, value: &str) -> Result<ModelName, OverlayError> {
    ModelName::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_endpoint(dotted: &str, value: &str) -> Result<ApiEndpoint, OverlayError> {
    ApiEndpoint::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_executable(dotted: &str, value: &str) -> Result<CodexExecutable, OverlayError> {
    CodexExecutable::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_workdir(dotted: &str, value: &str) -> Result<JobWorkDir, OverlayError> {
    JobWorkDir::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_cli_profile(dotted: &str, value: &str) -> Result<CliProfileRevision, OverlayError> {
    CliProfileRevision::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_output_profile(dotted: &str, value: &str) -> Result<OutputProfileName, OverlayError> {
    OutputProfileName::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}

fn need_price_file(dotted: &str, value: &str) -> Result<PriceFileRef, OverlayError> {
    PriceFileRef::new(value).map_err(|error| match error {
        crate::config::ConfigError::InvalidField { reason, .. } => shared::invalid(dotted, reason),
    })
}
