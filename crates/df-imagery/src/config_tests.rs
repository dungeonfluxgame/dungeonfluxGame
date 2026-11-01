use super::*;
use crate::config::{
    AccountingMode, ApiEndpoint, CliProfileRevision, CodexExecutable, JobWorkDir, ModelName,
    NamedOutputProfile, OutputProfileName, PriceFileRef,
};
use crate::request::Purpose;

#[test]
fn default_snapshot_is_valid() {
    ImageConfig::defaults().unwrap();
}

#[test]
fn zero_backend_concurrency_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.backends[BackendId::OpenAi.index()].max_concurrent_jobs = 0;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "backend.max_concurrent_jobs",
            reason: "a zero bound refuses all work",
        }
    );
}

#[test]
fn zero_pending_bytes_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.max_pending_bytes = 0;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "scheduler.max_pending_bytes",
            reason: "a zero bound refuses all work",
        }
    );
}

#[test]
fn zero_resource_bounds_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.max_decoded_bytes = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.max_output_bytes = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.queue_capacity = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.max_pending_jobs = 0;
    assert!(config.validate().is_err());
}

#[test]
fn larger_than_default_values_are_valid_without_a_code_change() {
    let mut config = ImageConfig::defaults().unwrap();
    config.backends[BackendId::OpenAi.index()].max_concurrent_jobs = 65;
    config.scheduler.foreground_limit = 65;
    config.scheduler.max_pending_bytes = u64::MAX;
    config.transforms.max_decoded_bytes = u64::MAX;
    config.validate().unwrap();
}

#[test]
fn stage_deadline_beyond_job_deadline_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.stage_deadline_secs = config.scheduler.job_deadline_secs + 1;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "scheduler.stage_deadline_secs",
            reason: "a stage deadline cannot outlive the job deadline",
        }
    );
}

#[test]
fn zero_request_bounds_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.request_limits.max_prompt_chars = 0;
    assert!(config.validate().is_err());
}

#[test]
fn default_request_limits_preserve_the_accepted_prompt_ceiling() {
    let config = ImageConfig::defaults().unwrap();
    assert_eq!(config.request_limits.max_prompt_chars, 32_000);
}

#[test]
fn blank_account_handle_is_refused() {
    assert!(AccountHandle::new("   ").is_err());
}

#[test]
fn account_handle_rejects_overlong_and_special_values() {
    let long: String = std::iter::repeat_n('a', 65).collect();
    assert!(AccountHandle::new(&long).is_err());
    assert!(AccountHandle::new("ops/key:1").is_err());
    assert_eq!(
        AccountHandle::new("clip-01_A").unwrap().as_str(),
        "clip-01_A"
    );
}

#[test]
fn every_purpose_indexes_into_its_own_gate() {
    let config = ImageConfig::defaults().unwrap();
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        assert_eq!(purpose.index(), index);
        assert!(config.purpose(purpose).enabled);
    }
    assert_eq!(Purpose::from_index(Purpose::COUNT), None);
}

#[test]
fn defaults_carry_typed_development_settings() {
    let config = ImageConfig::defaults().unwrap();
    assert_eq!(config.codex.executable.as_str(), "codex");
    assert_eq!(config.codex.workdir.as_str(), "artifacts/tmp/imagery-jobs");
    assert_eq!(config.codex.cli_profile.as_str(), "codex-images-v1");
    assert_eq!(config.codex.agent_model, None);
    assert_eq!(config.default_output_profile.as_str(), "default");
    assert_eq!(config.output_profiles.len(), 1);
    assert_eq!(config.retry.max_attempts, 2);
    assert_eq!(config.retry.retry_deadline_secs, 170);
    assert_eq!(config.accounting.mode, AccountingMode::RecordOnly);
    assert_eq!(config.accounting.price_file, None);
    assert_eq!(config.quality.jpeg_quality.value(), 85);
    assert_eq!(config.quality.provider_quality, None);
    assert!(config.backend(BackendId::OpenAi).model.is_none());
}

#[test]
fn resolver_returns_the_default_profile_for_every_purpose() {
    let config = ImageConfig::defaults().unwrap();
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        let resolved = config.output_profile_for(purpose).unwrap();
        assert_eq!(resolved, config.output_profiles[0].profile);
    }
}

#[test]
fn resolver_prefers_a_selected_purpose_profile() {
    use crate::output::{Dimensions, Encoding, RasterOutputSpec};

    let mut config = ImageConfig::defaults().unwrap();
    let delivered = RasterOutputSpec::new(
        Dimensions::new(96, 144).unwrap(),
        crate::output::AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    config.output_profiles.push(NamedOutputProfile::new(
        OutputProfileName::new("hero-thumbnail").unwrap(),
        OutputProfile::local(delivered),
    ));
    config.purposes[Purpose::CreatorPortrait.index()].output_profile =
        Some(OutputProfileName::new("hero-thumbnail").unwrap());
    config.validate().unwrap();
    let portrait = config.output_profile_for(Purpose::CreatorPortrait).unwrap();
    assert!(matches!(portrait, OutputProfile::Local { .. }));
    let plate = config.output_profile_for(Purpose::ScenePlate).unwrap();
    assert!(matches!(plate, OutputProfile::Provider { .. }));
}

#[test]
fn nonsecret_names_accept_their_documented_shapes() {
    assert_eq!(
        ModelName::new("gpt-image-1").unwrap().as_str(),
        "gpt-image-1"
    );
    assert_eq!(
        ModelName::new("flux-pro/v1.1-ultra").unwrap().as_str(),
        "flux-pro/v1.1-ultra"
    );
    assert!(ModelName::new("none").is_err());
    assert!(ModelName::new("has space").is_err());
    assert_eq!(
        ApiEndpoint::new("http://127.0.0.1:8080/v1")
            .unwrap()
            .as_str(),
        "http://127.0.0.1:8080/v1"
    );
    assert!(ApiEndpoint::new("https://user@example.com").is_err());
    assert!(ApiEndpoint::new("example.com/v1").is_err());
    assert_eq!(CodexExecutable::new("codex").unwrap().as_str(), "codex");
    assert_eq!(
        CodexExecutable::new("C:\\tools\\codex.exe")
            .unwrap()
            .as_str(),
        "C:\\tools\\codex.exe"
    );
    assert!(CodexExecutable::new("tools/codex").is_err());
    assert!(CodexExecutable::new("codex; rm -rf /").is_err());
    assert!(JobWorkDir::new("artifacts/tmp/imagery-jobs").is_ok());
    assert!(JobWorkDir::new("/absolute").is_ok());
    assert!(JobWorkDir::new("../escape").is_err());
    assert!(CliProfileRevision::new("codex-images-v1").is_ok());
    assert!(OutputProfileName::new("hero-thumbnail").is_ok());
    assert!(OutputProfileName::new("none").is_err());
    assert!(OutputProfileName::new("Hero").is_err());
    assert!(PriceFileRef::new("content/pricing.json").is_ok());
    assert!(PriceFileRef::new("/absolute/pricing.json").is_err());
    assert!(PriceFileRef::new("content/pricing.toml").is_err());
    assert!(AccountingMode::RecordOnly.label().contains("record"));
}

#[test]
fn endpoint_on_the_cli_backend_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.backends[BackendId::CodexCli.index()].endpoint =
        Some(ApiEndpoint::new("http://127.0.0.1:8080").unwrap());
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "backends.codex-cli.endpoint",
            reason: "the CLI backend uses an executable, not an endpoint",
        }
    );
}

#[test]
fn retry_deadline_beyond_job_deadline_is_capped_not_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.retry.retry_deadline_secs = config.scheduler.job_deadline_secs + 1;
    config.validate().unwrap();
    assert_eq!(
        config.effective_retry_deadline_secs(),
        config.scheduler.job_deadline_secs
    );
}

#[test]
fn short_job_deadlines_keep_history_valid_with_a_capped_retry_budget() {
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.job_deadline_secs = 60;
    config.scheduler.stage_deadline_secs = 45;
    config.validate().unwrap();
    assert_eq!(config.effective_retry_deadline_secs(), 60);
    config.retry.retry_deadline_secs = 30;
    config.validate().unwrap();
    assert_eq!(config.effective_retry_deadline_secs(), 30);
}

#[test]
fn zero_retry_attempts_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.retry.max_attempts = 0;
    assert!(config.validate().is_err());
}

#[test]
fn dangling_profile_selections_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.default_output_profile = OutputProfileName::new("missing").unwrap();
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "outputs.default_profile",
            reason: "the default profile names a listed profile",
        }
    );
    let mut config = ImageConfig::defaults().unwrap();
    config.purposes[Purpose::ScenePlate.index()].output_profile =
        Some(OutputProfileName::new("missing").unwrap());
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "purposes.output_profile",
            reason: "a purpose selection names a listed profile",
        }
    );
}

#[test]
fn duplicate_profile_names_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    let body = config.output_profiles[0].profile;
    config.output_profiles.push(NamedOutputProfile::new(
        OutputProfileName::new("default").unwrap(),
        body,
    ));
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "outputs",
            reason: "profile names stay unique",
        }
    );
}

#[test]
fn empty_profile_table_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.output_profiles.clear();
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "outputs",
            reason: "a snapshot lists at least one named profile",
        }
    );
}

#[test]
fn more_than_sixteen_profiles_are_valid_without_a_code_change() {
    use crate::output::{Dimensions, Encoding, RasterOutputSpec};

    let mut config = ImageConfig::defaults().unwrap();
    for index in 0..24 {
        let name = format!("extra-{index:02}");
        let delivered = RasterOutputSpec::new(
            Dimensions::new(32, 32).unwrap(),
            crate::output::AlphaHandling::Opaque,
            Encoding::Png,
        )
        .unwrap();
        config.output_profiles.push(NamedOutputProfile::new(
            OutputProfileName::new(&name).unwrap(),
            OutputProfile::local(delivered),
        ));
    }
    assert_eq!(config.output_profiles.len(), 25);
    config.validate().unwrap();
}

#[test]
fn executable_paths_with_spaces_and_apostrophes_are_valid_argv() {
    assert_eq!(
        CodexExecutable::new("/Applications/My Tools/codex's-bin/codex")
            .unwrap()
            .as_str(),
        "/Applications/My Tools/codex's-bin/codex"
    );
    assert_eq!(
        CodexExecutable::new("C:\\Program Files\\codex $bin\\codex.exe")
            .unwrap()
            .as_str(),
        "C:\\Program Files\\codex $bin\\codex.exe"
    );
    assert!(CodexExecutable::new("/tmp/codex\0evil").is_err());
    assert!(CodexExecutable::new("/tmp/codex\nevil").is_err());
    assert!(CodexExecutable::new("relative/path/codex").is_err());
    assert!(CodexExecutable::new("codex --evil").is_err());
}

#[test]
fn workdir_accepts_absolute_and_relative_bases_without_escape() {
    assert!(JobWorkDir::new("/var/lib/dungeonflux/jobs").is_ok());
    assert!(JobWorkDir::new("C:\\ProgramData\\DungeonFlux\\jobs").is_ok());
    assert!(JobWorkDir::new("C:/ProgramData/DungeonFlux/jobs").is_ok());
    assert!(JobWorkDir::new("\\\\host\\share\\jobs").is_ok());
    assert!(JobWorkDir::new("operator base/jobs'").is_ok());
    assert!(JobWorkDir::new("/").is_err());
    assert!(JobWorkDir::new("/jobs/../escape").is_err());
    assert!(JobWorkDir::new("jobs/./same").is_err());
    assert!(JobWorkDir::new("jobs//double").is_err());
    assert!(JobWorkDir::new("jobs\0evil").is_err());
    assert!(JobWorkDir::new("jobs\nevil").is_err());
}

#[test]
fn endpoints_reject_credentials_query_fragment_and_bad_hosts() {
    assert!(ApiEndpoint::new("http://127.0.0.1:8080/v1").is_ok());
    assert!(ApiEndpoint::new("https://fal.example.com/v1").is_ok());
    assert!(ApiEndpoint::new("http://localhost:3000").is_ok());
    assert!(ApiEndpoint::new("http://[::1]:8080/v1").is_ok());
    for bad in [
        "https://user@example.com",
        "https://user:pass@example.com/v1",
        "http://127.0.0.1:8080/v1?token=secret",
        "http://127.0.0.1:8080/v1#section",
        "http://127.0.0.1:8080/v1?x=1#y",
        "example.com/v1",
        "ftp://example.com/v1",
        "http://",
        "http:///v1",
        "http://bad host/v1",
        "http://host:99999/v1",
        "http://host:0/v1",
        "http://host:/v1",
        "http://host:abc/v1",
        "http://-bad.example.com/v1",
        "http://bad..example.com/v1",
        "http://[::1/v1",
        "http://1.2.3.4.5:abc/v1",
    ] {
        assert!(ApiEndpoint::new(bad).is_err(), "{bad}");
    }
    assert!(ApiEndpoint::new("http://127.0.0.1:8080/v1\0").is_err());
    assert!(ApiEndpoint::new("http://127.0.0.1:8080/v1\nevil").is_err());
    assert!(ApiEndpoint::new("http://127.0.0.1:\u{7f}8080/v1").is_err());
}

#[test]
fn endpoint_debug_stays_redacted() {
    let endpoint = ApiEndpoint::new("http://127.0.0.1:8080/v1").unwrap();
    let rendered = format!("{endpoint:?}");
    assert_eq!(rendered, "ApiEndpoint(..)");
    assert!(!rendered.contains("127.0.0.1"));
}

#[test]
fn provider_quality_defaults_to_unset_and_validates() {
    use crate::config::GenerationQuality;

    let mut config = ImageConfig::defaults().unwrap();
    assert_eq!(config.quality.provider_quality, None);
    config.quality.provider_quality = Some(GenerationQuality::High);
    config.validate().unwrap();
    assert_eq!(
        config.quality.provider_quality.unwrap(),
        GenerationQuality::High
    );
    config.quality.provider_quality = None;
    config.validate().unwrap();
}

#[test]
fn native_operator_paths_preserve_unicode_punctuation_and_long_segments() {
    for path in [
        "C:/Users/Jos\u{e9}/Imagery Jobs",
        "C:/Imagery $fixtures/(draft);&'",
        &format!("jobs/{}", "a".repeat(65)),
    ] {
        assert_eq!(JobWorkDir::new(path).unwrap().as_str(), path);
    }
    let executable = "C:/Users/Jos\u{e9}/Imagery $fixtures/(draft);&'/codex.exe";
    assert_eq!(
        CodexExecutable::new(executable).unwrap().as_str(),
        executable
    );
    assert!(JobWorkDir::new("jobs\0secret").is_err());
    assert!(CodexExecutable::new("C:/tools/codex\0.exe").is_err());
}

#[test]
fn endpoints_validate_uri_path_escapes_and_ipv6_literals() {
    for endpoint in [
        "https://example.test/\\evil",
        "https://example.test/%",
        "https://example.test/%2g",
        "https://example.test/[]",
        "https://[:::]/v1",
        "https://[example.test]/v1",
    ] {
        assert!(ApiEndpoint::new(endpoint).is_err(), "{endpoint}");
    }
    assert!(ApiEndpoint::new("http://[::ffff:192.0.2.1]:8080/v1").is_ok());
    assert!(ApiEndpoint::new("https://example.test/a%20b/route:$x&y").is_ok());
}

#[test]
fn codex_message_limit_defaults_to_24_mebibytes_and_only_requires_nonzero() {
    let mut config = ImageConfig::defaults().unwrap();
    assert_eq!(config.codex.max_message_bytes, 25_165_824);
    assert_eq!(config.codex.max_stdout_bytes, 33_554_432);
    assert_eq!(config.codex.max_stderr_bytes, 262_144);
    config.codex.max_message_bytes = 1;
    config.validate().unwrap();
    config.codex.max_message_bytes = u64::MAX;
    config.validate().unwrap();
    config.codex.max_message_bytes = 0;
    assert_eq!(
        config.validate(),
        Err(ConfigError::InvalidField {
            field: "codex.max_message_bytes",
            reason: "a zero bound refuses all work",
        })
    );
}

#[test]
fn trusted_analysis_routes_default_to_none_and_require_positive_token_limits() {
    use crate::request::AnalysisRoute;
    let mut config = ImageConfig::defaults().unwrap();
    assert_eq!(config.analysis, crate::config::AnalysisSettings::default());
    let mut route = AnalysisRoute {
        account: AccountHandle::new("analysis-fixture").unwrap(),
        model: ModelName::new("operator/custom-analysis").unwrap(),
        endpoint: ApiEndpoint::new("http://custom.analysis.test:8080/v1/responses").unwrap(),
        maximum_output_tokens: u64::MAX,
    };
    config.analysis.classification = Some(route.clone());
    config.analysis.inspection = Some(route.clone());
    config.validate().unwrap();
    route.maximum_output_tokens = 0;
    config.analysis.classification = Some(route.clone());
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidField {
            field: "analysis.classification.maximum_output_tokens",
            ..
        })
    ));
    config.analysis.classification = None;
    config.analysis.inspection = Some(route);
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidField {
            field: "analysis.inspection.maximum_output_tokens",
            ..
        })
    ));
}

#[test]
fn purpose_generation_overrides_are_optional_and_preserve_backend_selection() {
    use crate::config::GenerationQuality;
    let mut config = ImageConfig::defaults().unwrap();
    assert!(
        config
            .purposes
            .iter()
            .all(|purpose| purpose.quality_override.is_none() && purpose.model_override.is_none())
    );
    let gate = &mut config.purposes[Purpose::AnimationFrame.index()];
    gate.quality_override = Some(GenerationQuality::High);
    gate.model_override = Some(ModelName::new("custom/operator-frame-model").unwrap());
    gate.backend_override = Some(BackendId::Bfl);
    config.validate().unwrap();
    assert_eq!(
        config.purpose(Purpose::AnimationFrame).backend_override,
        Some(BackendId::Bfl)
    );
    assert_eq!(config.quality.provider_quality, None);
    assert_eq!(config.backend(BackendId::OpenAi).model, None);
}

#[test]
fn analysis_preparation_defaults_are_legacy_values_without_an_operator_ceiling() {
    let mut config = ImageConfig::defaults().unwrap();
    assert_eq!(config.analysis.preparation.max_pixels, 20_000_000);
    assert_eq!(config.analysis.preparation.max_picture_bytes, 1_048_576);
    assert_eq!(config.analysis.preparation.max_request_bytes, 16_777_216);
    config.analysis.preparation.max_pixels = u64::MAX;
    config.analysis.preparation.max_picture_bytes = u64::MAX;
    config.analysis.preparation.max_request_bytes = u64::MAX;
    config.validate().unwrap();
    for field in ["max_pixels", "max_picture_bytes", "max_request_bytes"] {
        let mut invalid = config.clone();
        match field {
            "max_pixels" => invalid.analysis.preparation.max_pixels = 0,
            "max_picture_bytes" => invalid.analysis.preparation.max_picture_bytes = 0,
            _ => invalid.analysis.preparation.max_request_bytes = 0,
        }
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn provider_moderation_override_is_independent_of_generation_quality() {
    let mut config = ImageConfig::defaults().unwrap();
    assert!(
        config
            .purposes
            .iter()
            .all(|purpose| purpose.moderation_override.is_none())
    );
    config.purposes[Purpose::AnimationFrame.index()].moderation_override =
        Some(crate::request::ProviderModeration::Low);
    config.validate().unwrap();
    assert_eq!(
        config.purpose(Purpose::AnimationFrame).quality_override,
        None
    );
    assert_eq!(config.quality.provider_quality, None);
    assert_eq!(
        config.purpose(Purpose::AnimationFrame).backend_override,
        None
    );
}
