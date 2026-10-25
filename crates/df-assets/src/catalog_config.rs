//! The catalog's tunable limits (planning/asset-storage-research.md 11, 17): plan values as
//! defaults, validated when loaded. A knob that is out of range falls back to its default and is
//! named in an audit line; nothing here panics or changes behaviour silently.

use crate::QuotaPolicy;
use crate::catalog_record::AssetKind;
use std::time::Duration;

const MIB: u64 = 1024 * 1024;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
/// Byte counts are stored as signed 64-bit integers.
const MAXIMUM_STORED_BYTES: u64 = i64::MAX as u64;

/// The hard byte limit of one staged object, per kind (a limit on storage, not a spend cap, D10).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaximumBytes {
    pub image: u64,
    pub audio: u64,
    pub video: u64,
    pub character_json: u64,
    pub checkpoint_bundle: u64,
    pub export_archive: u64,
}

impl Default for MaximumBytes {
    fn default() -> Self {
        Self {
            image: 10 * MIB,
            audio: 25 * MIB,
            video: 100 * MIB,
            character_json: MIB,
            checkpoint_bundle: 64 * MIB,
            export_archive: 512 * MIB,
        }
    }
}

impl MaximumBytes {
    /// Replaces each zero or database-unrepresentable limit with its default, naming it.
    fn validated(self) -> Self {
        let defaults = Self::default();
        let usable = |knob: &str, value: u64, default: u64| {
            if (1..=MAXIMUM_STORED_BYTES).contains(&value) {
                value
            } else {
                fall_back(knob, "zero_or_beyond_the_database_integer");
                default
            }
        };
        Self {
            image: usable("maximum_bytes.image", self.image, defaults.image),
            audio: usable("maximum_bytes.audio", self.audio, defaults.audio),
            video: usable("maximum_bytes.video", self.video, defaults.video),
            character_json: usable(
                "maximum_bytes.character_json",
                self.character_json,
                defaults.character_json,
            ),
            checkpoint_bundle: usable(
                "maximum_bytes.checkpoint_bundle",
                self.checkpoint_bundle,
                defaults.checkpoint_bundle,
            ),
            export_archive: usable(
                "maximum_bytes.export_archive",
                self.export_archive,
                defaults.export_archive,
            ),
        }
    }

    pub fn for_kind(&self, kind: AssetKind) -> u64 {
        match kind {
            AssetKind::Image => self.image,
            AssetKind::Audio => self.audio,
            AssetKind::Video => self.video,
            AssetKind::CharacterJson => self.character_json,
            AssetKind::CheckpointBundle => self.checkpoint_bundle,
            AssetKind::ExportArchive => self.export_archive,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogConfig {
    pub maximum_bytes: MaximumBytes,
    pub quota: QuotaPolicy,
    /// How long a step claim lasts before another worker may take the row.
    pub step_lease_ms: i64,
    /// How long after the supplier's answer the sweeper waits for the output before asking the
    /// dispatcher to fetch it again.
    pub staging_deadline_ms: i64,
    /// Only paces the orphan sweep: the deletion rule never depends on age.
    pub cleanup_grace_ms: i64,
    pub refetch_backoff_ms: i64,
    pub refetch_backoff_cap_ms: i64,
    /// Counts from the staged object's `staged_at_ms`; strictly below the lifecycle expiry.
    pub quarantine_max_age_ms: i64,
    /// The value S2's lifecycle rule for `Quarantine` is generated from.
    pub quarantine_lifecycle_expiry_ms: i64,
    pub sweeper_batch_size: usize,
    /// The deadline given to every object-store call.
    pub store_deadline: Duration,
    /// The longest the staging endpoint waits for one chunk of provider output.
    pub staging_chunk_timeout: Duration,
    /// The longest one staging transfer may take in total.
    pub staging_total_timeout: Duration,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            maximum_bytes: MaximumBytes::default(),
            quota: QuotaPolicy::default(),
            step_lease_ms: 5 * MINUTE_MS,
            staging_deadline_ms: 10 * MINUTE_MS,
            cleanup_grace_ms: 10 * MINUTE_MS,
            refetch_backoff_ms: MINUTE_MS,
            refetch_backoff_cap_ms: 30 * MINUTE_MS,
            quarantine_max_age_ms: 20 * HOUR_MS,
            quarantine_lifecycle_expiry_ms: 24 * HOUR_MS,
            sweeper_batch_size: 64,
            store_deadline: Duration::from_secs(30),
            staging_chunk_timeout: Duration::from_secs(30),
            staging_total_timeout: Duration::from_secs(300),
        }
    }
}

fn fall_back(knob: &str, reason: &str) {
    df_observe::record_audit_line(
        "ASSET",
        format!("ASSET config_fallback knob={knob} reason={reason}"),
    );
}

/// Replaces each zero or database-unrepresentable quota knob with its default, naming it.
fn validated_quota(quota: QuotaPolicy) -> QuotaPolicy {
    let defaults = QuotaPolicy::default();
    let bytes = |knob: &str, value: u64, default: u64| {
        if (1..=MAXIMUM_STORED_BYTES).contains(&value) {
            value
        } else {
            fall_back(knob, "zero_or_beyond_the_database_integer");
            default
        }
    };
    let count = |knob: &str, value: u32, default: u32| {
        if (1..=i32::MAX as u32).contains(&value) {
            value
        } else {
            fall_back(knob, "zero_or_beyond_the_database_integer");
            default
        }
    };
    QuotaPolicy {
        account_bytes: bytes(
            "quota.account_bytes",
            quota.account_bytes,
            defaults.account_bytes,
        ),
        account_templates: count(
            "quota.account_templates",
            quota.account_templates,
            defaults.account_templates,
        ),
        account_in_flight_uploads: count(
            "quota.account_in_flight_uploads",
            quota.account_in_flight_uploads,
            defaults.account_in_flight_uploads,
        ),
        campaign_bytes: bytes(
            "quota.campaign_bytes",
            quota.campaign_bytes,
            defaults.campaign_bytes,
        ),
    }
}

impl CatalogConfig {
    /// Replaces every knob that is not usable with its default, one audit line per knob.
    pub fn validated(self) -> Self {
        let defaults = Self::default();
        let mut config = self;
        if config.step_lease_ms <= 0 {
            fall_back("step_lease_ms", "not_positive");
            config.step_lease_ms = defaults.step_lease_ms;
        }
        if config.staging_deadline_ms <= 0 {
            fall_back("staging_deadline_ms", "not_positive");
            config.staging_deadline_ms = defaults.staging_deadline_ms;
        }
        if config.cleanup_grace_ms < 0 {
            fall_back("cleanup_grace_ms", "negative");
            config.cleanup_grace_ms = defaults.cleanup_grace_ms;
        }
        if config.refetch_backoff_ms <= 0
            || config.refetch_backoff_cap_ms < config.refetch_backoff_ms
        {
            fall_back("refetch_backoff_ms", "not_positive_or_above_cap");
            config.refetch_backoff_ms = defaults.refetch_backoff_ms;
            config.refetch_backoff_cap_ms = defaults.refetch_backoff_cap_ms;
        }
        if config.quarantine_lifecycle_expiry_ms <= 0 {
            fall_back("quarantine_lifecycle_expiry_ms", "not_positive");
            config.quarantine_lifecycle_expiry_ms = defaults.quarantine_lifecycle_expiry_ms;
        }
        if config.quarantine_max_age_ms <= 0
            || config.quarantine_max_age_ms >= config.quarantine_lifecycle_expiry_ms
        {
            fall_back("quarantine_max_age_ms", "not_below_lifecycle_expiry");
            config.quarantine_max_age_ms = defaults.quarantine_max_age_ms;
            config.quarantine_lifecycle_expiry_ms = defaults.quarantine_lifecycle_expiry_ms;
        }
        if config.sweeper_batch_size == 0 {
            fall_back("sweeper_batch_size", "zero");
            config.sweeper_batch_size = defaults.sweeper_batch_size;
        }
        if config.store_deadline.is_zero() {
            fall_back("store_deadline", "zero");
            config.store_deadline = defaults.store_deadline;
        }
        if config.staging_chunk_timeout.is_zero() {
            fall_back("staging_chunk_timeout", "zero");
            config.staging_chunk_timeout = defaults.staging_chunk_timeout;
        }
        if config.staging_total_timeout.is_zero() {
            fall_back("staging_total_timeout", "zero");
            config.staging_total_timeout = defaults.staging_total_timeout;
        }
        config.maximum_bytes = config.maximum_bytes.validated();
        config.quota = validated_quota(config.quota);
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarantine_age_not_below_the_lifecycle_expiry_falls_back_to_the_defaults() {
        let config = CatalogConfig {
            quarantine_max_age_ms: 25 * HOUR_MS,
            ..CatalogConfig::default()
        }
        .validated();
        assert_eq!(config, CatalogConfig::default());
    }

    #[test]
    fn a_usable_configuration_is_kept_exactly() {
        let config = CatalogConfig {
            step_lease_ms: 1_000,
            quarantine_max_age_ms: 2 * HOUR_MS,
            ..CatalogConfig::default()
        };
        assert_eq!(config.validated(), config);
    }

    #[test]
    fn a_zero_image_maximum_or_a_zero_quota_falls_back_instead_of_refusing_everything() {
        let mut config = CatalogConfig::default();
        config.maximum_bytes.image = 0;
        config.maximum_bytes.video = u64::MAX;
        config.quota.campaign_bytes = 0;
        config.quota.account_in_flight_uploads = 0;
        config.quota.account_bytes = 5;
        let config = config.validated();
        let defaults = CatalogConfig::default();
        assert_eq!(config.maximum_bytes.image, defaults.maximum_bytes.image);
        assert_eq!(config.maximum_bytes.video, defaults.maximum_bytes.video);
        assert_eq!(config.quota.campaign_bytes, defaults.quota.campaign_bytes);
        assert_eq!(
            config.quota.account_in_flight_uploads,
            defaults.quota.account_in_flight_uploads
        );
        assert_eq!(config.quota.account_bytes, 5, "a usable knob is kept");
    }

    #[test]
    fn a_zero_staging_timeout_falls_back() {
        let config = CatalogConfig {
            staging_chunk_timeout: Duration::ZERO,
            staging_total_timeout: Duration::ZERO,
            ..CatalogConfig::default()
        }
        .validated();
        assert_eq!(config, CatalogConfig::default());
    }

    #[test]
    fn each_unusable_knob_is_replaced_independently() {
        let config = CatalogConfig {
            step_lease_ms: 0,
            sweeper_batch_size: 0,
            cleanup_grace_ms: 7,
            ..CatalogConfig::default()
        }
        .validated();
        assert_eq!(config.step_lease_ms, CatalogConfig::default().step_lease_ms);
        assert_eq!(config.sweeper_batch_size, 64);
        assert_eq!(config.cleanup_grace_ms, 7);
    }
}
