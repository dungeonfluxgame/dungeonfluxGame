//! Local image-configuration state: validated base, launch patch, runtime patch.
//!
//! One owner holds the three layers separately and recomputes the effective
//! [`ImageConfig`] from them on every committed change. Precedence is the
//! [`crate::config_overlay`] order with one insertion: built-in or trusted host defaults,
//! then the supplied file layer, then legacy launcher environment, then new
//! `DF_IMAGERY_*` environment, then the explicit launch patch (recorded as
//! [`OverlaySource::Launch`](crate::config_overlay::OverlaySource::Launch)),
//! then the cumulative runtime patch (recorded as
//! [`OverlaySource::Runtime`](crate::config_overlay::OverlaySource::Runtime)).
//! Higher layers win silently and record their
//! source; within the file layer contradictory new and legacy spellings for
//! one field conflict, while new environment wins silently over legacy
//! environment as the higher of two distinct layers.
//!
//! This owner never reads ambient environment, the filesystem, or the
//! network; every input is supplied by the caller. It never stores, logs, or
//! echoes secrets: [`ImageConfig`] carries none by construction and every
//! error names fields only. Application is local only: [`AppliedLocal`]
//! reports a committed local revision, never a global dispatch
//! acknowledgement. The scheduler and hosted egress owners have not consumed
//! the revision yet, so global propagation stays pending until they do.
//! Persistence belongs to the launcher through
//! [`ConfigControl::save_prepared`]; this crate never rewrites an operator
//! TOML file.
//!
//! How it fails: an invalid patch, a stale expected revision, or a revision
//! counter at its ceiling leaves the complete state unchanged and returns a
//! [`ConfigApplyError`] the caller can branch on.

use crate::config::{ConfigError, ImageConfig};
use crate::config_overlay::{
    OverlayError, Provenance, ResolvedConfig, RuntimePatch, apply_launch_patch, apply_patch,
};

/// One committed local application. Local only: the new revision is visible
/// to later [`ConfigControl::read_effective`] calls, but no scheduler or
/// egress owner has acknowledged it, so a disable must not be treated as
/// globally propagated on this receipt alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppliedLocal {
    pub revision: u64,
}

/// Revision-consistent capture of the effective configuration for the
/// launcher to serialize or save. The triple travels together so a later
/// local update cannot tear the saved snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedExport {
    revision: u64,
    config: ImageConfig,
    provenance: Provenance,
}

impl PreparedExport {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn config(&self) -> &ImageConfig {
        &self.config
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

/// Receipt for a launcher-owned save of one [`PreparedExport`]. The revision
/// is the exact captured one, never a reread of newer state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedExport {
    pub revision: u64,
    pub path: String,
}

/// Why a local configuration change or save was refused. Field names are
/// echoed; values never are, so errors stay redacted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigApplyError {
    StaleRevision { expected: u64, current: u64 },
    Invalid(OverlayError),
    RevisionOverflow,
    PersistFailed,
}

impl std::fmt::Display for ConfigApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleRevision { expected, current } => write!(
                formatter,
                "stale revision: expected {expected}, current {current}"
            ),
            Self::Invalid(error) => write!(formatter, "invalid configuration: {error}"),
            Self::RevisionOverflow => write!(formatter, "revision counter at its ceiling"),
            Self::PersistFailed => write!(formatter, "launcher persistence failed"),
        }
    }
}

impl std::error::Error for ConfigApplyError {}

/// The one crate-native configuration-state owner. Base layers, the explicit
/// launch patch, and cumulative runtime state stay separate; the effective
/// snapshot is recomputed from all three on every committed change.
#[derive(Clone, Debug)]
pub struct ConfigControl {
    base: ImageConfig,
    base_provenance: Provenance,
    launch: RuntimePatch,
    runtime: RuntimePatch,
    revision: u64,
    effective: ResolvedConfig,
}

impl ConfigControl {
    /// Build the owner from supplied file text, supplied environment pairs,
    /// and an explicit launch patch at an initial revision. Reads nothing
    /// ambient. An invalid layer or launch patch yields no owner.
    pub fn build(
        config_text: &str,
        env: &[(&str, &str)],
        launch: &RuntimePatch,
        initial_revision: u64,
    ) -> Result<Self, OverlayError> {
        let base = crate::config_overlay::resolve(
            config_text,
            env,
            &RuntimePatch::empty(),
            initial_revision,
        )?;
        Self::from_resolved(base, launch, initial_revision)
    }

    /// Applies operator layers after trusted host defaults; seeded fields retain Default provenance.
    pub fn build_with_defaults(
        defaults: ImageConfig,
        config_text: &str,
        env: &[(&str, &str)],
        launch: &RuntimePatch,
        initial_revision: u64,
    ) -> Result<Self, OverlayError> {
        let base_resolved = crate::config_overlay::resolve_with_defaults(
            defaults,
            config_text,
            env,
            &RuntimePatch::empty(),
            initial_revision,
        )?;
        Self::from_resolved(base_resolved, launch, initial_revision)
    }

    fn from_resolved(
        base_resolved: ResolvedConfig,
        launch: &RuntimePatch,
        initial_revision: u64,
    ) -> Result<Self, OverlayError> {
        let effective = recompute(
            &base_resolved.config,
            &base_resolved.provenance,
            launch,
            &RuntimePatch::empty(),
            initial_revision,
        )?;
        Ok(Self {
            base: base_resolved.config,
            base_provenance: base_resolved.provenance,
            launch: launch.clone(),
            runtime: RuntimePatch::empty(),
            revision: initial_revision,
            effective,
        })
    }

    /// The live effective snapshot with its winning source per field. The
    /// returned value is an independent copy; later updates never mutate it.
    pub fn read_effective(&self) -> ResolvedConfig {
        self.effective.clone()
    }

    /// Before mounting the service, adopts the revision actually acknowledged by the durable
    /// execution-gate CAS. This does not change policy, provenance sources or runtime overlays.
    /// The host must not call this on an already published service or treat a number as an ack.
    pub fn adopt_startup_revision(
        &mut self,
        expected_local: u64,
        acknowledged_revision: u64,
    ) -> Result<AppliedLocal, ConfigApplyError> {
        if expected_local != self.revision {
            return Err(ConfigApplyError::StaleRevision {
                expected: expected_local,
                current: self.revision,
            });
        }
        if acknowledged_revision == 0 || acknowledged_revision < self.revision {
            return Err(ConfigApplyError::Invalid(OverlayError::InvalidField {
                field: "revision".to_owned(),
                reason: "an acknowledged revision is positive and cannot regress",
            }));
        }
        self.revision = acknowledged_revision;
        self.effective.provenance.revision = acknowledged_revision;
        self.base_provenance.revision = acknowledged_revision;
        Ok(AppliedLocal {
            revision: acknowledged_revision,
        })
    }

    /// The current local revision. Pass it back as the expected revision of
    /// the next `apply` or `reset`.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Pure validation of a candidate snapshot. Mutates no state.
    pub fn validate(&self, candidate: &ImageConfig) -> Result<(), ConfigError> {
        candidate.validate()
    }

    /// Pure preview of a runtime patch against current state: the post-patch
    /// snapshot at the current revision, committed nothing. An invalid patch
    /// leaves the owner untouched.
    pub fn preview(&self, patch: &RuntimePatch) -> Result<ResolvedConfig, ConfigApplyError> {
        let mut runtime = self.runtime.clone();
        merge_runtime(&mut runtime, patch);
        recompute(
            &self.base,
            &self.base_provenance,
            &self.launch,
            &runtime,
            self.revision,
        )
        .map_err(ConfigApplyError::Invalid)
    }

    /// Apply a partial typed runtime patch under an optimistic fence. `None`
    /// leaves a prior runtime value in place; `Some(None)` on a purpose
    /// backend override, the provider quality default, and the other
    /// two-level selections clears it. A stale fence, a full revision
    /// counter, or an invalid result leaves the complete state unchanged.
    pub fn apply(
        &mut self,
        patch: RuntimePatch,
        expected_revision: u64,
    ) -> Result<AppliedLocal, ConfigApplyError> {
        if expected_revision != self.revision {
            return Err(ConfigApplyError::StaleRevision {
                expected: expected_revision,
                current: self.revision,
            });
        }
        let next = self
            .revision
            .checked_add(1)
            .ok_or(ConfigApplyError::RevisionOverflow)?;
        let mut runtime = self.runtime.clone();
        merge_runtime(&mut runtime, &patch);
        let effective = recompute(
            &self.base,
            &self.base_provenance,
            &self.launch,
            &runtime,
            next,
        )
        .map_err(ConfigApplyError::Invalid)?;
        self.runtime = runtime;
        self.effective = effective;
        self.revision = next;
        Ok(AppliedLocal { revision: next })
    }

    /// Clear runtime overrides back to base plus launch at a fresh revision.
    /// Launch overrides and their launch provenance stay.
    /// A stale fence or a full counter leaves the state unchanged.
    pub fn reset(&mut self, expected_revision: u64) -> Result<AppliedLocal, ConfigApplyError> {
        if expected_revision != self.revision {
            return Err(ConfigApplyError::StaleRevision {
                expected: expected_revision,
                current: self.revision,
            });
        }
        let next = self
            .revision
            .checked_add(1)
            .ok_or(ConfigApplyError::RevisionOverflow)?;
        let effective = recompute(
            &self.base,
            &self.base_provenance,
            &self.launch,
            &RuntimePatch::empty(),
            next,
        )
        .map_err(ConfigApplyError::Invalid)?;
        self.runtime = RuntimePatch::empty();
        self.effective = effective;
        self.revision = next;
        Ok(AppliedLocal { revision: next })
    }

    /// Capture the effective configuration with its exact revision for a
    /// launcher-owned save. Later updates never mutate the capture.
    pub fn prepare_export(&self) -> PreparedExport {
        PreparedExport {
            revision: self.revision,
            config: self.effective.config.clone(),
            provenance: self.effective.provenance.clone(),
        }
    }

    /// Persist one [`PreparedExport`] through an injected launcher callback.
    /// The callback receives the exact captured revision, never a reread of
    /// newer state. A callback failure surfaces [`ConfigApplyError::PersistFailed`];
    /// only a callback success reports a [`PersistedExport`].
    pub fn save_prepared<E>(
        prepared: &PreparedExport,
        persist: impl FnOnce(&ImageConfig, &Provenance, u64) -> Result<String, E>,
    ) -> Result<PersistedExport, ConfigApplyError> {
        match persist(&prepared.config, &prepared.provenance, prepared.revision) {
            Ok(path) => Ok(PersistedExport {
                revision: prepared.revision,
                path,
            }),
            Err(_) => Err(ConfigApplyError::PersistFailed),
        }
    }
}

fn recompute(
    base: &ImageConfig,
    base_provenance: &Provenance,
    launch: &RuntimePatch,
    runtime: &RuntimePatch,
    revision: u64,
) -> Result<ResolvedConfig, OverlayError> {
    let mut config = base.clone();
    let mut provenance = base_provenance.clone();
    provenance.revision = revision;
    apply_launch_patch(launch, &mut config, &mut provenance)?;
    apply_patch(runtime, &mut config, &mut provenance)?;
    config.validate().map_err(|error| match error {
        ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
    })?;
    config.validate_codex_lines().map_err(|error| match error {
        ConfigError::InvalidField { field, reason } => OverlayError::InvalidField {
            field: field.to_owned(),
            reason,
        },
    })?;
    Ok(ResolvedConfig { config, provenance })
}

/// Merge a partial patch into cumulative runtime state: `None` keeps the
/// prior value, any `Some` (including `Some(None)` for a purpose backend
/// override) replaces it.
fn merge_runtime(into: &mut RuntimePatch, patch: &RuntimePatch) {
    if patch.mode.is_some() {
        into.mode = patch.mode;
    }
    for (slot, edit) in [
        (
            &mut into.analysis_classification,
            &patch.analysis_classification,
        ),
        (&mut into.analysis_inspection, &patch.analysis_inspection),
        (
            &mut into.analysis_head_locations,
            &patch.analysis_head_locations,
        ),
        (
            &mut into.analysis_staging_identity,
            &patch.analysis_staging_identity,
        ),
        (
            &mut into.analysis_staging_facing,
            &patch.analysis_staging_facing,
        ),
    ] {
        if let Some(edit) = edit {
            if let Some(prior) = slot {
                prior.merge(edit);
            } else {
                *slot = Some(edit.clone());
            }
        }
    }
    if let Some(edit) = &patch.analysis_segmentation {
        if let Some(prior) = &mut into.analysis_segmentation {
            prior.merge(edit);
        } else {
            into.analysis_segmentation = Some(edit.clone());
        }
    }
    if patch.analysis_segmentation_preparation_max_pixels.is_some() {
        into.analysis_segmentation_preparation_max_pixels =
            patch.analysis_segmentation_preparation_max_pixels;
    }
    if patch
        .analysis_segmentation_preparation_max_picture_bytes
        .is_some()
    {
        into.analysis_segmentation_preparation_max_picture_bytes =
            patch.analysis_segmentation_preparation_max_picture_bytes;
    }
    if patch
        .analysis_segmentation_preparation_max_request_bytes
        .is_some()
    {
        into.analysis_segmentation_preparation_max_request_bytes =
            patch.analysis_segmentation_preparation_max_request_bytes;
    }
    if patch.analysis_preparation_max_pixels.is_some() {
        into.analysis_preparation_max_pixels = patch.analysis_preparation_max_pixels;
    }
    if patch.analysis_preparation_max_picture_bytes.is_some() {
        into.analysis_preparation_max_picture_bytes = patch.analysis_preparation_max_picture_bytes;
    }
    if patch.analysis_preparation_max_request_bytes.is_some() {
        into.analysis_preparation_max_request_bytes = patch.analysis_preparation_max_request_bytes;
    }
    if patch.default_backend.is_some() {
        into.default_backend = patch.default_backend;
    }
    for index in 0..crate::request::Purpose::COUNT {
        if patch.purpose_enabled[index].is_some() {
            into.purpose_enabled[index] = patch.purpose_enabled[index];
        }
        if patch.purpose_backend[index].is_some() {
            into.purpose_backend[index] = patch.purpose_backend[index];
        }
    }
    for index in 0..crate::request::Purpose::COUNT {
        if patch.purpose_moderation_override[index].is_some() {
            into.purpose_moderation_override[index] = patch.purpose_moderation_override[index];
        }
        if patch.purpose_account_override[index].is_some() {
            into.purpose_account_override[index] = patch.purpose_account_override[index].clone();
        }
        if patch.purpose_quality_override[index].is_some() {
            into.purpose_quality_override[index] = patch.purpose_quality_override[index];
        }
        if patch.purpose_model_override[index].is_some() {
            into.purpose_model_override[index] = patch.purpose_model_override[index].clone();
        }
        if patch.purpose_output_profile[index].is_some() {
            into.purpose_output_profile[index] = patch.purpose_output_profile[index].clone();
        }
    }
    for index in 0..crate::config::BackendId::COUNT {
        if patch.backend_enabled[index].is_some() {
            into.backend_enabled[index] = patch.backend_enabled[index];
        }
        if patch.backend_account[index].is_some() {
            into.backend_account[index] = patch.backend_account[index].clone();
        }
        if patch.backend_concurrency[index].is_some() {
            into.backend_concurrency[index] = patch.backend_concurrency[index];
        }
        if patch.backend_model[index].is_some() {
            into.backend_model[index] = patch.backend_model[index].clone();
        }
        if patch.backend_generation_model[index].is_some() {
            into.backend_generation_model[index] = patch.backend_generation_model[index].clone();
        }
        if patch.backend_edit_model[index].is_some() {
            into.backend_edit_model[index] = patch.backend_edit_model[index].clone();
        }
        if patch.backend_alpha_model[index].is_some() {
            into.backend_alpha_model[index] = patch.backend_alpha_model[index].clone();
        }
        if patch.backend_endpoint[index].is_some() {
            into.backend_endpoint[index] = patch.backend_endpoint[index].clone();
        }
    }
    if patch.codex_executable.is_some() {
        into.codex_executable = patch.codex_executable.clone();
    }
    if patch.codex_workdir.is_some() {
        into.codex_workdir = patch.codex_workdir.clone();
    }
    if patch.codex_cli_profile.is_some() {
        into.codex_cli_profile = patch.codex_cli_profile.clone();
    }
    if patch.codex_max_stdout_bytes.is_some() {
        into.codex_max_stdout_bytes = patch.codex_max_stdout_bytes;
    }
    if patch.codex_max_stderr_bytes.is_some() {
        into.codex_max_stderr_bytes = patch.codex_max_stderr_bytes;
    }
    if let Some(value) = &patch.creator_bfl_account {
        into.creator_bfl_account = Some(value.clone());
    }
    if patch.quota_enabled.is_some() {
        into.quota_enabled = patch.quota_enabled;
    }
    if patch.quota_cooldown_ms.is_some() {
        into.quota_cooldown_ms = patch.quota_cooldown_ms;
    }
    if patch.quota_maximum_accounts.is_some() {
        into.quota_maximum_accounts = patch.quota_maximum_accounts;
    }
    if patch.codex_max_message_bytes.is_some() {
        into.codex_max_message_bytes = patch.codex_max_message_bytes;
    }
    if patch.codex_agent_model.is_some() {
        into.codex_agent_model = patch.codex_agent_model.clone();
    }
    if patch.default_output_profile.is_some() {
        into.default_output_profile = patch.default_output_profile.clone();
    }
    for edit in &patch.output_profile_edits {
        let position = into
            .output_profile_edits
            .iter()
            .position(|prior| prior.name == edit.name);
        match position {
            Some(index) => {
                for (key, value) in &edit.fields {
                    into.output_profile_edits[index]
                        .fields
                        .insert(key.clone(), value.clone());
                }
            }
            None => into.output_profile_edits.push(edit.clone()),
        }
    }
    if patch.output_profiles.is_some() {
        into.output_profiles = patch.output_profiles.clone();
    }
    if patch.retry_max_attempts.is_some() {
        into.retry_max_attempts = patch.retry_max_attempts;
    }
    if patch.retry_deadline_secs.is_some() {
        into.retry_deadline_secs = patch.retry_deadline_secs;
    }
    if patch.retry_backoff_secs.is_some() {
        into.retry_backoff_secs = patch.retry_backoff_secs;
    }
    if patch.accounting_mode.is_some() {
        into.accounting_mode = patch.accounting_mode;
    }
    if patch.accounting_price_file.is_some() {
        into.accounting_price_file = patch.accounting_price_file.clone();
    }
    if patch.quality_jpeg_quality.is_some() {
        into.quality_jpeg_quality = patch.quality_jpeg_quality;
    }
    if patch.quality_provider_quality.is_some() {
        into.quality_provider_quality = patch.quality_provider_quality;
    }
    if patch.transforms_enabled.is_some() {
        into.transforms_enabled = patch.transforms_enabled;
    }
    if patch.transforms_max_concurrency.is_some() {
        into.transforms_max_concurrency = patch.transforms_max_concurrency;
    }
    if patch.transforms_max_decoded_bytes.is_some() {
        into.transforms_max_decoded_bytes = patch.transforms_max_decoded_bytes;
    }
    if patch.transforms_max_output_bytes.is_some() {
        into.transforms_max_output_bytes = patch.transforms_max_output_bytes;
    }
    if patch.transforms_queue_capacity.is_some() {
        into.transforms_queue_capacity = patch.transforms_queue_capacity;
    }
    if patch.transforms_deadline_secs.is_some() {
        into.transforms_deadline_secs = patch.transforms_deadline_secs;
    }
    if patch.scheduler_foreground_limit.is_some() {
        into.scheduler_foreground_limit = patch.scheduler_foreground_limit;
    }
    if patch.scheduler_presence_limit.is_some() {
        into.scheduler_presence_limit = patch.scheduler_presence_limit;
    }
    if patch.scheduler_background_limit.is_some() {
        into.scheduler_background_limit = patch.scheduler_background_limit;
    }
    if patch.scheduler_max_pending_jobs.is_some() {
        into.scheduler_max_pending_jobs = patch.scheduler_max_pending_jobs;
    }
    if patch.scheduler_max_pending_bytes.is_some() {
        into.scheduler_max_pending_bytes = patch.scheduler_max_pending_bytes;
    }
    if patch.scheduler_queue_age_secs.is_some() {
        into.scheduler_queue_age_secs = patch.scheduler_queue_age_secs;
    }
    if patch.scheduler_job_deadline_secs.is_some() {
        into.scheduler_job_deadline_secs = patch.scheduler_job_deadline_secs;
    }
    if patch.scheduler_stage_deadline_secs.is_some() {
        into.scheduler_stage_deadline_secs = patch.scheduler_stage_deadline_secs;
    }
    if patch.request_max_prompt_chars.is_some() {
        into.request_max_prompt_chars = patch.request_max_prompt_chars;
    }
    if patch.request_max_references.is_some() {
        into.request_max_references = patch.request_max_references;
    }
    if patch.request_max_transform_steps.is_some() {
        into.request_max_transform_steps = patch.request_max_transform_steps;
    }
    if patch.request_max_composite_layers.is_some() {
        into.request_max_composite_layers = patch.request_max_composite_layers;
    }
}

#[cfg(test)]
mod startup_adoption_tests {
    use super::*;
    #[test]
    fn startup_adoption_preserves_policy_sources_and_future_runtime_overlays() {
        let mut control = ConfigControl::build("", &[], &RuntimePatch::empty(), 1).unwrap();
        let mut patch = RuntimePatch::empty();
        patch.mode = Some(crate::GlobalMode::LocalOnly);
        control.apply(patch, 1).unwrap();
        let before = control.read_effective();
        control.adopt_startup_revision(2, 41).unwrap();
        let adopted = control.read_effective();
        assert_eq!(adopted.config, before.config);
        assert_eq!(adopted.provenance.fields, before.provenance.fields);
        assert_eq!(adopted.provenance.ignored, before.provenance.ignored);
        assert_eq!(adopted.provenance.revision, 41);
        control.apply(RuntimePatch::empty(), 41).unwrap();
        assert_eq!(
            control.read_effective().config.mode,
            crate::GlobalMode::LocalOnly
        );
        assert_eq!(control.revision(), 42);
    }
    #[test]
    fn stale_or_zero_startup_adoption_leaves_the_complete_state_unchanged() {
        let mut control = ConfigControl::build("", &[], &RuntimePatch::empty(), 1).unwrap();
        let state = |owner: &ConfigControl| {
            (
                owner.read_effective(),
                owner.revision(),
                owner.launch.clone(),
                owner.runtime.clone(),
                owner.base.clone(),
                owner.base_provenance.clone(),
            )
        };
        let before = state(&control);
        assert!(control.adopt_startup_revision(2, 41).is_err());
        assert_eq!(state(&control), before);
        assert!(control.adopt_startup_revision(1, 0).is_err());
        assert_eq!(state(&control), before);
        control.adopt_startup_revision(1, 41).unwrap();
        let before = state(&control);
        assert!(control.adopt_startup_revision(41, 40).is_err());
        assert_eq!(state(&control), before);
    }
}
