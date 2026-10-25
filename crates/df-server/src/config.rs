//! The hosted server's TOML configuration contract: one file per role, every section typed and
//! closed (unknown keys are refused), secrets named by `*_file` path and never inline, every
//! tunable defaulted to the value the hosted runtime design proposes and validated at load.
//! Staging and production reject every local-only key. Loading never reads the environment.

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::deployment::{InstanceId, InstanceIdRefusal};

mod listing;
mod validation;

pub use listing::flatten;
#[cfg(test)]
pub(crate) use listing::redact_owned_sections;
pub use validation::ConfigError;

/// Largest secret or CA file the loader accepts; a bigger file is a mistake, not a secret.
const MAXIMUM_REFERENCED_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    ControlPlane,
    CampaignRuntime,
    Egress,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Environment {
    Local,
    Staging,
    Production,
}

impl Environment {
    /// Staging and production are the hosted environments that reject local-only keys.
    pub const fn is_hosted(self) -> bool {
        matches!(self, Self::Staging | Self::Production)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSection {
    pub role: Role,
    pub environment: Environment,
    pub instance_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenSection {
    /// The edge-facing listener of a control plane; refused for the other roles.
    pub public_address: Option<SocketAddr>,
    /// The internal mTLS listener (game, media gateway, dispatcher or finance endpoint).
    pub internal_address: SocketAddr,
    pub metrics_address: SocketAddr,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    VerifyFull,
    /// Local only, and only to a literal loopback address or a socket path.
    LoopbackPlaintext,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSection {
    pub mode: TlsMode,
    pub ca_file: Option<PathBuf>,
    pub server_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSection {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub user: String,
    pub password_file: PathBuf,
    #[serde(default = "DatabaseSection::default_pool_size")]
    pub pool_size: u32,
    #[serde(default = "DatabaseSection::default_statement_timeout_ms")]
    pub statement_timeout_ms: u32,
    pub tls: TlsSection,
}

impl DatabaseSection {
    fn default_pool_size() -> u32 {
        4
    }

    fn default_statement_timeout_ms() -> u32 {
        30_000
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathsSection {
    /// Explicit scratch directory, so nothing falls back to the process environment.
    pub temp_directory: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetrySection {
    pub otlp_endpoint: String,
    pub service_name: String,
    pub spool_directory: PathBuf,
    #[serde(default = "TelemetrySection::default_spool_bytes_maximum")]
    pub spool_bytes_maximum: u64,
    #[serde(default = "TelemetrySection::default_push_interval_seconds")]
    pub push_interval_seconds: u32,
}

impl TelemetrySection {
    fn default_spool_bytes_maximum() -> u64 {
        256 * 1024 * 1024
    }

    fn default_push_interval_seconds() -> u32 {
        10
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct LeaseSection {
    pub seconds: u32,
    pub renew_every_seconds: u32,
    pub safety_margin_seconds: u32,
    /// How long a placement may wait for its runtime to claim before another may supersede it.
    pub placing_seconds: u32,
}

impl Default for LeaseSection {
    fn default() -> Self {
        Self {
            seconds: 30,
            renew_every_seconds: 10,
            safety_margin_seconds: 3,
            placing_seconds: 60,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct MeterSection {
    pub segment_roll_seconds: u32,
    pub backdate_window_seconds: u32,
}

impl Default for MeterSection {
    fn default() -> Self {
        Self {
            segment_roll_seconds: 60,
            backdate_window_seconds: 45,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct PresenceSection {
    pub window_ms: u32,
    pub idle_release_seconds: u32,
    pub away_end_hours: u32,
}

impl Default for PresenceSection {
    fn default() -> Self {
        Self {
            window_ms: 30_000,
            idle_release_seconds: 900,
            away_end_hours: 12,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct BlockingWaitSection {
    pub gated_grace_ms: u32,
    pub outage_grace_ms: u32,
    pub blocked_interval_free: bool,
}

impl Default for BlockingWaitSection {
    fn default() -> Self {
        Self {
            gated_grace_ms: 10_000,
            outage_grace_ms: 0,
            blocked_interval_free: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct OwnerSection {
    pub stall_seconds: u32,
    pub stall_exit_seconds: u32,
}

impl Default for OwnerSection {
    fn default() -> Self {
        Self {
            stall_seconds: 15,
            stall_exit_seconds: 60,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct MoneySection {
    pub reconstruction_deadline_seconds: u32,
}

impl Default for MoneySection {
    fn default() -> Self {
        Self {
            reconstruction_deadline_seconds: 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct DrainSection {
    pub effect_wait_seconds: u32,
    pub maximum_seconds: u32,
    pub edge_seconds: u32,
}

impl Default for DrainSection {
    fn default() -> Self {
        Self {
            effect_wait_seconds: 30,
            maximum_seconds: 90,
            edge_seconds: 20,
        }
    }
}

/// Table lifecycle and ownership settings (hosted runtime design 5, 6.2, 6.5).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct RuntimeSection {
    pub maximum_seats: u8,
    pub lease: LeaseSection,
    pub meter: MeterSection,
    pub presence: PresenceSection,
    pub blocking_wait: BlockingWaitSection,
    pub owner: OwnerSection,
    pub money: MoneySection,
    pub drain: DrainSection,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            maximum_seats: 6,
            lease: LeaseSection::default(),
            meter: MeterSection::default(),
            presence: PresenceSection::default(),
            blocking_wait: BlockingWaitSection::default(),
            owner: OwnerSection::default(),
            money: MoneySection::default(),
            drain: DrainSection::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProvidersMode {
    Fake,
    Live,
}

/// The dispatcher's provider wiring; only the egress role has one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProvidersSection {
    pub mode: ProvidersMode,
    /// Staging refuses live providers unless this is set.
    #[serde(default)]
    pub allow_live: bool,
}

/// Keys that exist only for local development; a hosted environment refuses the section.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalDevSection {
    /// Mounts the `/dev/*` control surface.
    #[serde(default)]
    pub dev_control: bool,
}

/// A section another owner defines and validates; carried through unread.
pub type OwnedSection = toml::Table;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub instance: InstanceSection,
    pub listen: ListenSection,
    pub database: DatabaseSection,
    pub paths: PathsSection,
    pub telemetry: TelemetrySection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    pub providers: Option<ProvidersSection>,
    pub local_dev: Option<LocalDevSection>,
    /// TODO(O2): the object-store section; O2's loader validates it.
    pub objects: Option<OwnedSection>,
    /// TODO(B): the identity-provider section; B's loader validates it.
    pub identity: Option<OwnedSection>,
    /// TODO(B): the payment-processor section; B's loader validates it.
    pub payments: Option<OwnedSection>,
}

impl ServerConfig {
    /// Parses and validates configuration text.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text).map_err(|error| ConfigError::Parse {
            message: error.message().to_string(),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Reads, parses and validates a configuration file, then checks that every file it names
    /// (password, CA certificate) exists and is a plausible size.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
            path: path.to_path_buf(),
            kind: error.kind(),
        })?;
        let config = Self::from_toml_str(&text)?;
        config.check_referenced_files()?;
        Ok(config)
    }

    pub(crate) fn instance_id(&self) -> Result<InstanceId, InstanceIdRefusal> {
        InstanceId::new(self.instance.instance_id.clone())
    }

    fn check_referenced_files(&self) -> Result<(), ConfigError> {
        check_file("database.password_file", &self.database.password_file)?;
        if let Some(ca_file) = &self.database.tls.ca_file {
            check_file("database.tls.ca_file", ca_file)?;
        }
        Ok(())
    }
}

fn check_file(key: &'static str, path: &Path) -> Result<(), ConfigError> {
    let metadata = fs::metadata(path).map_err(|_| ConfigError::ReferencedFileUnreadable { key })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_REFERENCED_FILE_BYTES
    {
        return Err(ConfigError::ReferencedFileUnreadable { key });
    }
    Ok(())
}
