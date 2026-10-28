//! Validated image configuration supplied by composition.
//!
//! The crate reads a supplied snapshot, never ambient environment. The later
//! launcher maps files, environment aliases and runtime overrides onto this
//! model; per-field precedence and file mapping belong to that owner.
//!
//! Resource bounds are operator-tunable with sane defaults and no hardcoded
//! ceilings: validation requires nonzero values where zero would refuse all
//! work, keeps the stage deadline inside the job deadline, and uses widening
//! integer arithmetic so no bound can overflow in a check.

use crate::request::{Purpose, RequestError, RequestLimits};

/// Global execution posture of the image service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalMode {
    Live,
    LocalOnly,
    CacheOnly,
    Off,
}

/// Selectable image backends; Codex CLI is a normal backend here.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BackendId {
    OpenAi,
    Fal,
    Bfl,
    CodexCli,
}

impl BackendId {
    pub const COUNT: usize = 4;

    pub fn index(self) -> usize {
        match self {
            Self::OpenAi => 0,
            Self::Fal => 1,
            Self::Bfl => 2,
            Self::CodexCli => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Fal => "fal",
            Self::Bfl => "bfl",
            Self::CodexCli => "codex-cli",
        }
    }
}

/// Nonsecret account handle for scheduling, cache identity and telemetry.
/// Credentials are never carried here; composition supplies them at egress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountHandle(String);

impl AccountHandle {
    pub fn new(handle: &str) -> Result<Self, ConfigError> {
        let trimmed = handle.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(ConfigError::InvalidField {
                field: "account",
                reason: "handle is empty or over 64 characters",
            });
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ConfigError::InvalidField {
                field: "account",
                reason: "handle keeps to ASCII letters, digits, - and _",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-purpose gate: enablement plus an optional backend override. Disabling
/// a purpose also stops its transform demand; it never revokes sharing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PurposeGate {
    pub enabled: bool,
    pub backend_override: Option<BackendId>,
}

/// Per-backend gate: enablement, nonsecret account binding, concurrency.
/// Disabling one backend never reroutes to another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendGate {
    pub enabled: bool,
    pub account: AccountHandle,
    pub max_concurrent_jobs: u32,
}

/// Local transform resource bounds; no supplier calls use these permits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformLimits {
    pub enabled: bool,
    pub max_concurrency: u32,
    pub max_decoded_bytes: u64,
    pub max_output_bytes: u64,
    pub queue_capacity: u32,
    pub deadline_secs: u64,
}

/// Finite scheduler bounds shared by provider and local stages. The stage
/// deadline is absolute and never extends the job deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerBounds {
    pub foreground_limit: u32,
    pub presence_limit: u32,
    pub background_limit: u32,
    pub max_pending_jobs: u32,
    pub max_pending_bytes: u64,
    pub queue_age_secs: u64,
    pub job_deadline_secs: u64,
    pub stage_deadline_secs: u64,
}

/// The one validated image configuration snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageConfig {
    pub mode: GlobalMode,
    pub default_backend: BackendId,
    pub purposes: [PurposeGate; Purpose::COUNT],
    pub backends: [BackendGate; BackendId::COUNT],
    pub transforms: TransformLimits,
    pub scheduler: SchedulerBounds,
    pub request_limits: RequestLimits,
}

impl ImageConfig {
    /// Built-in defaults; the operator file and overrides replace them. The
    /// enabled OpenAI backend and per-purpose defaults are placeholders the
    /// coordinator must confirm before any dispatch is enabled.
    pub fn defaults() -> Result<Self, ConfigError> {
        let purposes = [PurposeGate {
            enabled: true,
            backend_override: None,
        }; Purpose::COUNT];
        let account = AccountHandle::new("default")?;
        let backends = [
            BackendGate {
                enabled: true,
                account: account.clone(),
                max_concurrent_jobs: 3,
            },
            BackendGate {
                enabled: false,
                account: account.clone(),
                max_concurrent_jobs: 3,
            },
            BackendGate {
                enabled: false,
                account: account.clone(),
                max_concurrent_jobs: 2,
            },
            BackendGate {
                enabled: false,
                account,
                max_concurrent_jobs: 2,
            },
        ];
        let config = Self {
            mode: GlobalMode::Live,
            default_backend: BackendId::OpenAi,
            purposes,
            backends,
            transforms: TransformLimits {
                enabled: true,
                max_concurrency: 2,
                max_decoded_bytes: 268_435_456,
                max_output_bytes: 16_777_216,
                queue_capacity: 64,
                deadline_secs: 30,
            },
            scheduler: SchedulerBounds {
                foreground_limit: 3,
                presence_limit: 3,
                background_limit: 6,
                max_pending_jobs: 512,
                max_pending_bytes: 536_870_912,
                queue_age_secs: 300,
                job_deadline_secs: 240,
                stage_deadline_secs: 170,
            },
            request_limits: RequestLimits::default(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn purpose(&self, purpose: Purpose) -> &PurposeGate {
        &self.purposes[purpose.index()]
    }

    pub fn backend(&self, backend: BackendId) -> &BackendGate {
        &self.backends[backend.index()]
    }

    /// Finite values and resource bounds only; capability policy stays with
    /// the service, and secrets never appear here. No upper bounds are
    /// imposed: larger operator values are valid without a code change.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for backend in &self.backends {
            nonzero(
                "backend.max_concurrent_jobs",
                u64::from(backend.max_concurrent_jobs),
            )?;
        }
        let transforms = &self.transforms;
        nonzero(
            "transforms.max_concurrency",
            u64::from(transforms.max_concurrency),
        )?;
        nonzero("transforms.max_decoded_bytes", transforms.max_decoded_bytes)?;
        nonzero("transforms.max_output_bytes", transforms.max_output_bytes)?;
        nonzero(
            "transforms.queue_capacity",
            u64::from(transforms.queue_capacity),
        )?;
        nonzero("transforms.deadline_secs", transforms.deadline_secs)?;
        let scheduler = &self.scheduler;
        nonzero(
            "scheduler.foreground_limit",
            u64::from(scheduler.foreground_limit),
        )?;
        nonzero(
            "scheduler.presence_limit",
            u64::from(scheduler.presence_limit),
        )?;
        nonzero(
            "scheduler.background_limit",
            u64::from(scheduler.background_limit),
        )?;
        nonzero(
            "scheduler.max_pending_jobs",
            u64::from(scheduler.max_pending_jobs),
        )?;
        nonzero("scheduler.max_pending_bytes", scheduler.max_pending_bytes)?;
        nonzero("scheduler.queue_age_secs", scheduler.queue_age_secs)?;
        nonzero("scheduler.job_deadline_secs", scheduler.job_deadline_secs)?;
        nonzero(
            "scheduler.stage_deadline_secs",
            scheduler.stage_deadline_secs,
        )?;
        if scheduler.stage_deadline_secs > scheduler.job_deadline_secs {
            return Err(ConfigError::InvalidField {
                field: "scheduler.stage_deadline_secs",
                reason: "a stage deadline cannot outlive the job deadline",
            });
        }
        match self.request_limits.validate() {
            Ok(()) => Ok(()),
            Err(RequestError::InvalidLimit { field }) => Err(ConfigError::InvalidField {
                field,
                reason: "a request bound must admit at least one item",
            }),
            Err(_) => Err(ConfigError::InvalidField {
                field: "request_limits",
                reason: "request limits are invalid",
            }),
        }
    }
}

fn nonzero(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidField {
            field,
            reason: "a zero bound refuses all work",
        });
    }
    Ok(())
}

/// Why a configuration snapshot was refused; the prior snapshot stays live.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
}
