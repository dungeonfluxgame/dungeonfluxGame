//! Where the host finds the owner's local Forge studio, for the concerns that can use it instead
//! of a paid supplier: voices and speech recognition (`DF_SPEECH_BACKEND=forge`), the SAM scan
//! (`DF_SCAN_BACKEND=forge`) and picture generation (`DF_IMAGERY_BACKEND=forge`). Each concern
//! keeps its own port; this module only reads Forge's address and project, once per use, through
//! `environment`. The client itself is `df_providers::ForgeClient`.

use std::fmt;

use df_providers::{FORGE_DEFAULT_ORIGIN, ForgeClient, ForgeConfig};
use serde::{Deserialize, Serialize};

use crate::environment;

/// The value of a concern's backend variable that selects Forge.
pub const FORGE_BACKEND: &str = "forge";
/// Forge's origin; unset is Forge's own default address, `FORGE_DEFAULT_ORIGIN`.
pub const URL_VARIABLE: &str = "DF_FORGE_URL";
/// The id of the Forge project the renders go to (`forge_projects` lists them).
pub const PROJECT_VARIABLE: &str = "DF_FORGE_PROJECT";

/// Why the host has no Forge client.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeSetupCause {
    /// `DF_FORGE_PROJECT` is not set.
    ProjectMissing,
    /// `DF_FORGE_URL` is not a loopback `http://` origin, or `DF_FORGE_PROJECT` not a project id.
    InvalidSetting,
}

impl ForgeSetupCause {
    pub const fn key(self) -> &'static str {
        match self {
            Self::ProjectMissing => "project_missing",
            Self::InvalidSetting => "invalid_setting",
        }
    }
}

/// Why the host has no Forge client, and what to set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForgeSetupError {
    pub cause: ForgeSetupCause,
}

impl fmt::Display for ForgeSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.cause {
            ForgeSetupCause::ProjectMissing => write!(
                formatter,
                "forge has no project: set {PROJECT_VARIABLE} to the id of a forge project"
            ),
            ForgeSetupCause::InvalidSetting => write!(
                formatter,
                "{URL_VARIABLE} must be a loopback http:// origin and {PROJECT_VARIABLE} a \
                 forge project id"
            ),
        }
    }
}

impl std::error::Error for ForgeSetupError {}

/// The Forge client this process's environment describes.
pub fn client_from_environment() -> Result<ForgeClient, ForgeSetupError> {
    client_with(None, None)
}

/// The Forge client of `origin` and `project`, each the environment's when not given.
pub fn client_with(
    origin: Option<&str>,
    project: Option<&str>,
) -> Result<ForgeClient, ForgeSetupError> {
    let project = project
        .map(str::to_owned)
        .or_else(|| environment::text(PROJECT_VARIABLE))
        .ok_or(ForgeSetupError {
            cause: ForgeSetupCause::ProjectMissing,
        })?;
    let origin = origin
        .map(str::to_owned)
        .or_else(|| environment::text(URL_VARIABLE))
        .unwrap_or_else(|| FORGE_DEFAULT_ORIGIN.to_owned());
    let invalid = ForgeSetupError {
        cause: ForgeSetupCause::InvalidSetting,
    };
    let config = ForgeConfig::new(&origin, &project).map_err(|_| invalid)?;
    ForgeClient::new(config).map_err(|_| invalid)
}
