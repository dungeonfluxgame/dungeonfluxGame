//! Validated image configuration supplied by composition.
//!
//! The crate reads a supplied snapshot, never ambient environment. The later
//! launcher maps files, environment aliases and runtime overrides onto this
//! model; per-field precedence and file mapping belong to that owner.
//!
//! Resource bounds are operator-tunable with sane defaults and no hardcoded
//! ceilings: validation requires nonzero values where zero would refuse all
//! work, keeps the stage deadline inside the job deadline, and uses widening
//! integer arithmetic so no bound can overflow in a check. The retry deadline
//! is a finite budget capped by the job deadline at use
//! (`effective_retry_deadline_secs`), never a cross-field refusal, so
//! historical snapshots with short job deadlines stay valid. Private
//! executors consume the frozen snapshot through these typed nonsecret
//! fields; credentials never appear here and stay at deployment composition.

use crate::output::{DEFAULT_JPEG_QUALITY_0_100, LossyQuality, OutputProfile};
use crate::request::{Purpose, RequestError, RequestLimits};

/// Largest Codex protocol line: a base64 PNG for the 16 MiB artifact bound, plus the envelope.
pub const CODEX_MAX_MESSAGE_BYTES_DEFAULT: u64 = 24 * 1024 * 1024;
/// Stdout must hold the largest single protocol line plus the other turn events.
pub const CODEX_MAX_STDOUT_BYTES_DEFAULT: u64 = 32 * 1024 * 1024;
/// JSON envelope allowance around one base64 image line.
const CODEX_MESSAGE_ENVELOPE_BYTES: u64 = 64 * 1024;

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

/// Nonsecret provider model selection (for example `gpt-image-1`). The
/// agent model on a Codex invocation is not an image-model guarantee.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelName(String);

impl ModelName {
    pub fn new(name: &str) -> Result<Self, ConfigError> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err(ConfigError::InvalidField {
                field: "model",
                reason: "a model name is 1 to 128 characters",
            });
        }
        if trimmed == "none" {
            return Err(ConfigError::InvalidField {
                field: "model",
                reason: "a model name avoids none, which clears a selection",
            });
        }
        let valid = trimmed.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')
        });
        if !valid {
            return Err(ConfigError::InvalidField {
                field: "model",
                reason: "a model name keeps to letters, digits and - _ . / :",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Nonsecret API base endpoint for development transport (for example a
/// loopback fixture server). The value is a structural `http(s)` base URL
/// with no userinfo, query, or fragment; credentials stay at deployment
/// composition and are never parsed out of this value.
#[derive(Clone, Eq, PartialEq)]
pub struct ApiEndpoint(String);

impl std::fmt::Debug for ApiEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ApiEndpoint(..)")
    }
}

impl ApiEndpoint {
    pub fn new(endpoint: &str) -> Result<Self, ConfigError> {
        let trimmed = endpoint.trim();
        if trimmed.is_empty() || trimmed.len() > 256 {
            return Err(ConfigError::InvalidField {
                field: "endpoint",
                reason: "an endpoint is 1 to 256 characters",
            });
        }
        if trimmed.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(ConfigError::InvalidField {
                field: "endpoint",
                reason: "an endpoint carries no control characters",
            });
        }
        let Some((scheme, rest)) = trimmed.split_once("://") else {
            return Err(ConfigError::InvalidField {
                field: "endpoint",
                reason: "an endpoint keeps to an http(s) base URL",
            });
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let path = rest.split_once('/').map_or("", |(_, path)| path);
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            || authority.is_empty()
            || authority.contains('@')
            || trimmed.contains('?')
            || trimmed.contains('#')
            || trimmed.contains('\\')
            || !is_valid_endpoint_path(path)
        {
            return Err(ConfigError::InvalidField {
                field: "endpoint",
                reason: "an endpoint carries a valid http(s) authority and URI path without credentials, query, or fragment",
            });
        }
        let parsed = url::Url::parse(trimmed).map_err(|_| ConfigError::InvalidField {
            field: "endpoint",
            reason: "an endpoint is a valid http(s) base URL",
        })?;
        let valid_host = match parsed.host() {
            Some(url::Host::Domain(host)) => host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }),
            Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => true,
            None => false,
        };
        if !valid_host || parsed.port() == Some(0) || authority.ends_with(':') {
            return Err(ConfigError::InvalidField {
                field: "endpoint",
                reason: "an endpoint names a valid host and port",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_endpoint_path(path: &str) -> bool {
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            if !bytes.next().is_some_and(|value| value.is_ascii_hexdigit())
                || !bytes.next().is_some_and(|value| value.is_ascii_hexdigit())
            {
                return false;
            }
        } else if !byte.is_ascii_alphanumeric()
            && !matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
                    | b'/'
            )
        {
            return false;
        }
    }
    true
}

/// Codex executable selection: a bare command resolved through `PATH`
/// (for example `codex`) or an absolute filesystem path. This names
/// `argv[0]` only, never shell text: the backend uses native argument
/// arrays, so an absolute path may carry spaces, apostrophes, or other
/// shell punctuation without any shell meaning. There is no executable
/// allowlist, no version pin and no hash check; an unsupported capability
/// reports its typed outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexExecutable(String);

impl CodexExecutable {
    pub fn new(executable: &str) -> Result<Self, ConfigError> {
        validate_native_path(executable, "executable")?;
        if (executable.contains('/') || executable.contains('\\')) && !is_absolute_path(executable)
        {
            return Err(ConfigError::InvalidField {
                field: "executable",
                reason: "a path executable is absolute",
            });
        }
        if !is_absolute_path(executable) && executable.chars().any(char::is_whitespace) {
            return Err(ConfigError::InvalidField {
                field: "executable",
                reason: "a bare command contains no whitespace",
            });
        }
        Ok(Self(executable.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_native_path(path: &str, field: &'static str) -> Result<(), ConfigError> {
    if path.is_empty()
        || path.len() > 4096
        || path
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(ConfigError::InvalidField {
            field,
            reason: "a native path is nonempty, control-free and at most 4096 bytes",
        });
    }
    Ok(())
}

fn is_absolute_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with("\\\\") {
        return true;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    false
}

/// Operator base directory for job work. Either an absolute path or a path
/// relative to the service working directory; exact per-job input/output
/// directories are staged beneath it by the worker at runtime and stay
/// worker-owned, never operator-addressed per job. The worker resolves and
/// confines individual job paths beneath this operator-selected base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobWorkDir(String);

impl JobWorkDir {
    pub fn new(workdir: &str) -> Result<Self, ConfigError> {
        validate_native_path(workdir, "workdir")?;
        let unrooted = workdir.trim_start_matches(['/', '\\']);
        let unrooted = if unrooted.as_bytes().get(1) == Some(&b':') {
            unrooted.get(3..).unwrap_or_default()
        } else {
            unrooted
        };
        if unrooted.is_empty()
            || unrooted
                .split(['/', '\\'])
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(ConfigError::InvalidField {
                field: "workdir",
                reason: "a workdir names a directory without traversal segments",
            });
        }
        Ok(Self(workdir.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Supported CLI image-profile revision (for example `codex-images-v1`).
/// A newer CLI is never refused merely because its version changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliProfileRevision(String);

impl CliProfileRevision {
    pub fn new(revision: &str) -> Result<Self, ConfigError> {
        let trimmed = revision.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(ConfigError::InvalidField {
                field: "cli_profile",
                reason: "a CLI profile revision is 1 to 64 characters",
            });
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ConfigError::InvalidField {
                field: "cli_profile",
                reason: "a CLI profile revision keeps to letters, digits and - _ .",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Named output profile key. Lowercase letters, digits and hyphens only so
/// environment spellings round-trip back to the same file spelling.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OutputProfileName(String);

impl OutputProfileName {
    pub fn new(name: &str) -> Result<Self, ConfigError> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(ConfigError::InvalidField {
                field: "output_profile",
                reason: "a profile name is 1 to 64 characters",
            });
        }
        if trimmed == "none" {
            return Err(ConfigError::InvalidField {
                field: "output_profile",
                reason: "a profile name avoids none, which clears a selection",
            });
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ConfigError::InvalidField {
                field: "output_profile",
                reason: "a profile name keeps to lowercase letters, digits and -",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Nonsecret price-file reference for accounting projections (for example
/// `content/pricing.json`). Money policy stays with the finance owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceFileRef(String);

impl PriceFileRef {
    pub fn new(reference: &str) -> Result<Self, ConfigError> {
        let trimmed = reference.trim();
        if trimmed.is_empty() || trimmed.len() > 256 {
            return Err(ConfigError::InvalidField {
                field: "price_file",
                reason: "a price-file reference is 1 to 256 characters",
            });
        }
        if !trimmed.ends_with(".json")
            || trimmed.contains('\\')
            || trimmed.contains(':')
            || trimmed.starts_with('/')
        {
            return Err(ConfigError::InvalidField {
                field: "price_file",
                reason: "a price-file reference is a relative .json path",
            });
        }
        for segment in trimmed.split('/') {
            if segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.len() > 64
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            {
                return Err(ConfigError::InvalidField {
                    field: "price_file",
                    reason: "a price-file segment keeps to letters, digits and - _ .",
                });
            }
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-purpose gate: enablement, an optional backend override and an
/// optional named output profile selection. Model and generation-quality
/// overrides apply to generation/edit only. Disabling a purpose also stops
/// its transform demand; it never revokes sharing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurposeGate {
    pub enabled: bool,
    pub backend_override: Option<BackendId>,
    pub account_override: Option<AccountHandle>,
    pub output_profile: Option<OutputProfileName>,
    pub quality_override: Option<GenerationQuality>,
    pub moderation_override: Option<crate::request::ProviderModeration>,
    pub model_override: Option<ModelName>,
}

/// Per-backend gate: enablement, nonsecret account binding, concurrency,
/// plus optional nonsecret model and base-endpoint selection. Disabling one
/// backend never reroutes to another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendGate {
    pub enabled: bool,
    pub account: AccountHandle,
    pub max_concurrent_jobs: u32,
    pub model: Option<ModelName>,
    pub generation_model: Option<ModelName>,
    pub edit_model: Option<ModelName>,
    pub alpha_model: Option<ModelName>,
    pub endpoint: Option<ApiEndpoint>,
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

/// Codex CLI composition settings: executable selection, job workdir, the
/// supported CLI image-profile revision and an optional agent model. The
/// agent model never promises which image model serves a turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexSettings {
    pub executable: CodexExecutable,
    pub workdir: JobWorkDir,
    pub cli_profile: CliProfileRevision,
    pub agent_model: Option<ModelName>,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    /// Maximum bytes in one CLI protocol message; independent of total stdout.
    pub max_message_bytes: u64,
}

/// Safe retry knobs for provider and CLI dispatch. These bound additional
/// attempts only; a refusal is never retried with other wording or model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub retry_deadline_secs: u64,
    pub backoff_secs: u64,
}

/// How usage is accounted. `RecordOnly` keeps local record-only projections
/// with no hosted spend; `Hosted` admits through the finance authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountingMode {
    RecordOnly,
    Hosted,
}

impl AccountingMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::RecordOnly => "record-only",
            Self::Hosted => "hosted",
        }
    }
}

/// Accounting settings: the mode plus an optional nonsecret price-file
/// reference for projections. Tracking never imposes a spend cap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Accounting {
    pub mode: AccountingMode,
    pub price_file: Option<PriceFileRef>,
}

/// Provider generation quality, separate from delivered encoding quality.
/// Legacy numeric selections remain lossless historical data for migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationQuality {
    Low,
    Medium,
    High,
    Auto,
    /// Preserves historical version 2 snapshots without inventing a named quality.
    LegacyNumeric(LossyQuality),
}

/// Defaults for delivered JPEG encoding and provider generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QualityDefaults {
    pub jpeg_quality: LossyQuality,
    pub provider_quality: Option<GenerationQuality>,
}

/// One named output profile: a caller-visible key plus the concrete
/// provider or local output contract it selects. Requests keep their
/// explicit `OutputProfile`; the resolver below maps a purpose to one of
/// these at the caller/composition seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedOutputProfile {
    pub name: OutputProfileName,
    pub profile: OutputProfile,
}

impl NamedOutputProfile {
    pub fn new(name: OutputProfileName, profile: OutputProfile) -> Self {
        Self { name, profile }
    }
}

/// Operator limits for private analysis preparation, separate from published asset limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisPreparationLimits {
    pub max_pixels: u64,
    pub max_picture_bytes: u64,
    pub max_request_bytes: u64,
}
impl Default for AnalysisPreparationLimits {
    fn default() -> Self {
        Self {
            max_pixels: crate::output::MAXIMUM_OUTPUT_PIXELS,
            max_picture_bytes: 1024 * 1024,
            max_request_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Provider identity for the configured SAM RLE segmentation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentationProvider {
    Fal,
}

/// Trusted segmentation account, model slug and queue-base endpoint; no text-token settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentationRoute {
    pub provider: SegmentationProvider,
    pub account: AccountHandle,
    pub model: ModelName,
    pub endpoint: ApiEndpoint,
}

/// Trusted, operator-configured analysis routes; request data cannot choose credential destinations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisSettings {
    pub preparation: AnalysisPreparationLimits,
    pub classification: Option<crate::request::AnalysisRoute>,
    pub inspection: Option<crate::request::AnalysisRoute>,
    pub head_locations: Option<crate::request::AnalysisRoute>,
    pub staging_identity: Option<crate::request::AnalysisRoute>,
    pub staging_facing: Option<crate::request::AnalysisRoute>,
    pub segmentation: Option<SegmentationRoute>,
    pub segmentation_preparation: AnalysisPreparationLimits,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            preparation: AnalysisPreparationLimits::default(),
            classification: None,
            inspection: None,
            head_locations: None,
            staging_identity: None,
            staging_facing: None,
            segmentation: None,
            segmentation_preparation: AnalysisPreparationLimits {
                max_pixels: crate::output::MAXIMUM_OUTPUT_PIXELS,
                max_picture_bytes: 3 * 1024 * 1024,
                max_request_bytes: 6 * 1024 * 1024,
            },
        }
    }
}

/// Shared supplier-account quota policy; disabling bypasses admission without erasing state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaSettings {
    pub enabled: bool,
    pub cooldown_ms: u64,
    /// Closed entries without live admissions are evicted first when this bound is reached.
    pub maximum_accounts: u32,
}
impl Default for QuotaSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            cooldown_ms: 900_000,
            maximum_accounts: 1_024,
        }
    }
}

/// The one validated image configuration snapshot. The named profile table
/// is bounded by the supplying document (file text, environment, and codec
/// byte bounds at each parser), never by a fixed profile count: any
/// non-empty table with unique names is valid.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageConfig {
    pub mode: GlobalMode,
    pub default_backend: BackendId,
    /// The creator alternate BFL route never inherits the primary OpenAI account override.
    pub creator_bfl_account: Option<AccountHandle>,
    pub purposes: [PurposeGate; Purpose::COUNT],
    pub backends: [BackendGate; BackendId::COUNT],
    pub transforms: TransformLimits,
    pub scheduler: SchedulerBounds,
    pub request_limits: RequestLimits,
    pub codex: CodexSettings,
    pub analysis: AnalysisSettings,
    pub quota: QuotaSettings,
    pub default_output_profile: OutputProfileName,
    pub output_profiles: Vec<NamedOutputProfile>,
    pub retry: RetryPolicy,
    pub accounting: Accounting,
    pub quality: QualityDefaults,
}

impl ImageConfig {
    /// Built-in defaults; the operator file and overrides replace them. The
    /// enabled OpenAI backend and per-purpose defaults are placeholders the
    /// coordinator must confirm before any dispatch is enabled.
    pub fn defaults() -> Result<Self, ConfigError> {
        let purposes = std::array::from_fn(|_| PurposeGate {
            enabled: true,
            backend_override: None,
            account_override: None,
            output_profile: None,
            quality_override: None,
            moderation_override: None,
            model_override: None,
        });
        let account = AccountHandle::new("default")?;
        let backends = [
            BackendGate {
                enabled: false,
                account: account.clone(),
                max_concurrent_jobs: 3,
                model: None,
                generation_model: None,
                edit_model: None,
                alpha_model: None,
                endpoint: None,
            },
            BackendGate {
                enabled: false,
                account: account.clone(),
                max_concurrent_jobs: 3,
                model: None,
                generation_model: None,
                edit_model: None,
                alpha_model: None,
                endpoint: None,
            },
            BackendGate {
                enabled: false,
                account: account.clone(),
                max_concurrent_jobs: 2,
                model: None,
                generation_model: None,
                edit_model: None,
                alpha_model: None,
                endpoint: None,
            },
            BackendGate {
                enabled: true,
                account,
                max_concurrent_jobs: 2,
                model: None,
                generation_model: None,
                edit_model: None,
                alpha_model: None,
                endpoint: None,
            },
        ];
        let config = Self {
            mode: GlobalMode::Live,
            default_backend: BackendId::CodexCli,
            creator_bfl_account: None,
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
            analysis: AnalysisSettings::default(),
            quota: QuotaSettings::default(),
            codex: CodexSettings {
                executable: CodexExecutable::new("codex")?,
                workdir: JobWorkDir::new("artifacts/tmp/imagery-jobs")?,
                cli_profile: CliProfileRevision::new("codex-images-v1")?,
                agent_model: None,
                max_stdout_bytes: CODEX_MAX_STDOUT_BYTES_DEFAULT,
                max_stderr_bytes: 256 * 1024,
                max_message_bytes: CODEX_MAX_MESSAGE_BYTES_DEFAULT,
            },
            default_output_profile: OutputProfileName::new("default").map_err(|_| {
                ConfigError::InvalidField {
                    field: "outputs.default_profile",
                    reason: "the default profile names a listed profile",
                }
            })?,
            output_profiles: vec![NamedOutputProfile::new(
                OutputProfileName::new("default").map_err(|_| ConfigError::InvalidField {
                    field: "outputs.default_profile",
                    reason: "the default profile names a listed profile",
                })?,
                default_profile_body()?,
            )],
            retry: RetryPolicy {
                max_attempts: 2,
                retry_deadline_secs: 170,
                backoff_secs: 2,
            },
            accounting: Accounting {
                mode: AccountingMode::RecordOnly,
                price_file: None,
            },
            quality: QualityDefaults {
                jpeg_quality: LossyQuality::new(DEFAULT_JPEG_QUALITY_0_100).map_err(|_| {
                    ConfigError::InvalidField {
                        field: "quality.jpeg_quality",
                        reason: "a JPEG quality is 1 to 100",
                    }
                })?,
                provider_quality: None,
            },
        };
        config.validate()?;
        Ok(config)
    }

    /// Operator-set Codex line limits, checked where operators set values rather than on every
    /// snapshot decode, so retained snapshots from earlier builds stay readable. One base64 image
    /// line must hold the largest artifact the transform bound admits, and stdout must hold it.
    pub fn validate_codex_lines(&self) -> Result<(), ConfigError> {
        let largest_line = self
            .transforms
            .max_output_bytes
            .div_ceil(3)
            .saturating_mul(4)
            .saturating_add(CODEX_MESSAGE_ENVELOPE_BYTES);
        if self.codex.max_message_bytes < largest_line {
            return Err(ConfigError::InvalidField {
                field: "codex.max_message_bytes",
                reason: "must hold a base64 image line for transforms.max_output_bytes",
            });
        }
        if self.codex.max_stdout_bytes < self.codex.max_message_bytes {
            return Err(ConfigError::InvalidField {
                field: "codex.max_stdout_bytes",
                reason: "must be at least codex.max_message_bytes",
            });
        }
        Ok(())
    }

    pub fn purpose(&self, purpose: Purpose) -> &PurposeGate {
        &self.purposes[purpose.index()]
    }

    pub fn backend(&self, backend: BackendId) -> &BackendGate {
        &self.backends[backend.index()]
    }

    /// Concrete output contract for a purpose at the caller/composition
    /// seam. The purpose selection wins; the snapshot default covers the
    /// rest. Requests keep their explicit `OutputProfile`; this maps a
    /// purpose to one of the named profiles so the caller can build it.
    /// Wiring it into dispatch belongs to the later executor owner.
    pub fn output_profile_for(&self, purpose: Purpose) -> Option<OutputProfile> {
        let selected = self.purposes[purpose.index()]
            .output_profile
            .as_ref()
            .unwrap_or(&self.default_output_profile);
        self.output_profiles
            .iter()
            .find(|entry| &entry.name == selected)
            .map(|entry| entry.profile)
    }

    fn find_output_profile(&self, name: &OutputProfileName) -> bool {
        self.output_profiles.iter().any(|entry| &entry.name == name)
    }

    /// Finite values and resource bounds only; capability policy stays with
    /// the service, and secrets never appear here. No upper bounds are
    /// imposed: larger operator values are valid without a code change.
    pub fn validate(&self) -> Result<(), ConfigError> {
        nonzero("quota.cooldown_ms", self.quota.cooldown_ms)?;
        nonzero(
            "quota.maximum_accounts",
            u64::from(self.quota.maximum_accounts),
        )?;
        for backend in &self.backends {
            nonzero(
                "backend.max_concurrent_jobs",
                u64::from(backend.max_concurrent_jobs),
            )?;
        }
        nonzero("codex.max_stdout_bytes", self.codex.max_stdout_bytes)?;
        nonzero("codex.max_stderr_bytes", self.codex.max_stderr_bytes)?;
        nonzero("codex.max_message_bytes", self.codex.max_message_bytes)?;
        nonzero(
            "analysis.preparation.max_pixels",
            self.analysis.preparation.max_pixels,
        )?;
        nonzero(
            "analysis.preparation.max_picture_bytes",
            self.analysis.preparation.max_picture_bytes,
        )?;
        nonzero(
            "analysis.preparation.max_request_bytes",
            self.analysis.preparation.max_request_bytes,
        )?;

        nonzero(
            "analysis.segmentation_preparation.max_pixels",
            self.analysis.segmentation_preparation.max_pixels,
        )?;
        nonzero(
            "analysis.segmentation_preparation.max_picture_bytes",
            self.analysis.segmentation_preparation.max_picture_bytes,
        )?;
        nonzero(
            "analysis.segmentation_preparation.max_request_bytes",
            self.analysis.segmentation_preparation.max_request_bytes,
        )?;
        for (field, route) in [
            (
                "analysis.classification.maximum_output_tokens",
                &self.analysis.classification,
            ),
            (
                "analysis.inspection.maximum_output_tokens",
                &self.analysis.inspection,
            ),
            (
                "analysis.head_locations.maximum_output_tokens",
                &self.analysis.head_locations,
            ),
            (
                "analysis.staging_identity.maximum_output_tokens",
                &self.analysis.staging_identity,
            ),
            (
                "analysis.staging_facing.maximum_output_tokens",
                &self.analysis.staging_facing,
            ),
        ] {
            if let Some(route) = route {
                nonzero(field, route.maximum_output_tokens)?;
            }
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
            Ok(()) => (),
            Err(RequestError::InvalidLimit { field }) => {
                return Err(ConfigError::InvalidField {
                    field,
                    reason: "a request bound must admit at least one item",
                });
            }
            Err(_) => {
                return Err(ConfigError::InvalidField {
                    field: "request_limits",
                    reason: "request limits are invalid",
                });
            }
        }
        for (index, gate) in self.backends.iter().enumerate() {
            if gate.endpoint.is_some() && index == BackendId::CodexCli.index() {
                return Err(ConfigError::InvalidField {
                    field: "backends.codex-cli.endpoint",
                    reason: "the CLI backend uses an executable, not an endpoint",
                });
            }
        }
        if self.output_profiles.is_empty() {
            return Err(ConfigError::InvalidField {
                field: "outputs",
                reason: "a snapshot lists at least one named profile",
            });
        }
        for (position, entry) in self.output_profiles.iter().enumerate() {
            let duplicate = self.output_profiles[..position]
                .iter()
                .any(|prior| prior.name == entry.name);
            if duplicate {
                return Err(ConfigError::InvalidField {
                    field: "outputs",
                    reason: "profile names stay unique",
                });
            }
        }
        if !self.find_output_profile(&self.default_output_profile) {
            return Err(ConfigError::InvalidField {
                field: "outputs.default_profile",
                reason: "the default profile names a listed profile",
            });
        }
        for index in 0..Purpose::COUNT {
            if let Some(selected) = &self.purposes[index].output_profile
                && !self.find_output_profile(selected)
            {
                return Err(ConfigError::InvalidField {
                    field: "purposes.output_profile",
                    reason: "a purpose selection names a listed profile",
                });
            }
        }
        nonzero("retry.max_attempts", u64::from(self.retry.max_attempts))?;
        nonzero("retry.retry_deadline_secs", self.retry.retry_deadline_secs)?;
        Ok(())
    }

    /// Finite retry budget actually available to the scheduler and worker:
    /// the configured retry deadline capped by the job deadline. A retry
    /// deadline beyond the job deadline is valid configuration and simply
    /// retries within the job; historical snapshots with short job
    /// deadlines stay valid.
    pub fn effective_retry_deadline_secs(&self) -> u64 {
        self.retry
            .retry_deadline_secs
            .min(self.scheduler.job_deadline_secs)
    }
}

/// Built-in default profile body: provider-native 1024 square generation
/// with a delivered opaque JPEG at the neutral quality. File, environment
/// and runtime edits seed missing profiles from this body.
pub(crate) fn default_profile_body() -> Result<OutputProfile, ConfigError> {
    use crate::output::{AlphaHandling, Dimensions, Encoding, RasterOutputSpec};
    let map = |field: &'static str| ConfigError::InvalidField {
        field,
        reason: "a built-in dimension is valid",
    };
    let generated = Dimensions::new(1024, 1024).map_err(|_| map("outputs"))?;
    let delivered_size = Dimensions::new(1024, 1024).map_err(|_| map("outputs"))?;
    let quality =
        LossyQuality::new(DEFAULT_JPEG_QUALITY_0_100).map_err(|_| map("quality.jpeg_quality"))?;
    let delivered = RasterOutputSpec::new(
        delivered_size,
        AlphaHandling::Opaque,
        Encoding::Jpeg { quality },
    )
    .map_err(|_| map("outputs"))?;
    Ok(OutputProfile::provider(generated, delivered))
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
