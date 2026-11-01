use crate::config::{AccountHandle, BackendId, GlobalMode, ImageConfig};
use crate::config_control::{ConfigApplyError, ConfigControl};
use crate::config_overlay::{OverlayError, OverlaySource, RuntimePatch, resolve};
use crate::request::Purpose;

fn launch_mode(mode: GlobalMode) -> RuntimePatch {
    let mut launch = RuntimePatch::empty();
    launch.mode = Some(mode);
    launch
}

fn build(file: &str, env: &[(&str, &str)], launch: &RuntimePatch, revision: u64) -> ConfigControl {
    ConfigControl::build(file, env, launch, revision).unwrap()
}

#[test]
fn resolve_contract_stays_source_compatible() {
    let resolved = resolve("", &[], &RuntimePatch::empty(), 7).unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    assert_eq!(resolved.provenance.revision, 7);
    assert_eq!(
        resolved.provenance.source_of("mode"),
        Some(OverlaySource::Default)
    );
}

#[test]
fn base_launch_runtime_precedence_records_winning_sources() {
    let file = "[imagery]\nmode = \"local-only\"\n";
    let env = [("DF_IMAGERY_MODE", "off")];
    let mut owner = build(file, &env, &launch_mode(GlobalMode::CacheOnly), 5);
    let effective = owner.read_effective();
    assert_eq!(effective.config.mode, GlobalMode::CacheOnly);
    assert_eq!(
        effective.provenance.source_of("mode"),
        Some(OverlaySource::Launch)
    );
    assert_eq!(effective.provenance.revision, 5);

    let mut patch = RuntimePatch::empty();
    patch.mode = Some(GlobalMode::Off);
    let applied = owner.apply(patch, 5).unwrap();
    assert_eq!(applied.revision, 6);
    let effective = owner.read_effective();
    assert_eq!(effective.config.mode, GlobalMode::Off);
    assert_eq!(
        effective.provenance.source_of("mode"),
        Some(OverlaySource::Runtime)
    );
    assert_eq!(effective.provenance.revision, 6);
}

#[test]
fn new_env_wins_over_legacy_env_and_launch_wins_over_env() {
    let env = [("DF_GAMEPLAY_IMAGERY_PROVIDER", "flux")];
    let owner = build("", &env, &RuntimePatch::empty(), 1);
    let effective = owner.read_effective();
    assert_eq!(effective.config.default_backend, BackendId::Fal);
    assert_eq!(
        effective.provenance.source_of("default_backend"),
        Some(OverlaySource::LegacyEnv(
            "DF_GAMEPLAY_IMAGERY_PROVIDER".to_owned()
        ))
    );

    let env = [
        ("DF_GAMEPLAY_IMAGERY_PROVIDER", "flux"),
        ("DF_IMAGERY_BACKEND", "openai"),
    ];
    let owner = build("", &env, &RuntimePatch::empty(), 1);
    assert_eq!(
        owner.read_effective().config.default_backend,
        BackendId::OpenAi
    );
    assert_eq!(
        owner
            .read_effective()
            .provenance
            .source_of("default_backend"),
        Some(OverlaySource::NewEnv("DF_IMAGERY_BACKEND".to_owned()))
    );

    let mut launch = RuntimePatch::empty();
    launch.default_backend = Some(BackendId::CodexCli);
    let owner = build("", &env, &launch, 1);
    let effective = owner.read_effective();
    assert_eq!(effective.config.default_backend, BackendId::CodexCli);
    assert_eq!(
        effective.provenance.source_of("default_backend"),
        Some(OverlaySource::Launch)
    );
}

#[test]
fn partial_applies_accumulate_without_clobbering() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 10);
    let mut first = RuntimePatch::empty();
    first.mode = Some(GlobalMode::Off);
    owner.apply(first, 10).unwrap();
    let mut second = RuntimePatch::empty();
    second.transforms_max_concurrency = Some(7);
    second.scheduler_queue_age_secs = Some(91);
    owner.apply(second, 11).unwrap();

    let effective = owner.read_effective();
    assert_eq!(effective.config.mode, GlobalMode::Off);
    assert_eq!(effective.config.transforms.max_concurrency, 7);
    assert_eq!(effective.config.scheduler.queue_age_secs, 91);
    assert_eq!(effective.provenance.revision, 12);
    assert_eq!(
        effective.provenance.source_of("mode"),
        Some(OverlaySource::Runtime)
    );
}

#[test]
fn none_in_a_patch_keeps_prior_runtime_values() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let mut first = RuntimePatch::empty();
    first.scheduler_foreground_limit = Some(9);
    owner.apply(first, 1).unwrap();
    let mut second = RuntimePatch::empty();
    second.mode = Some(GlobalMode::LocalOnly);
    owner.apply(second, 2).unwrap();

    let effective = owner.read_effective();
    assert_eq!(effective.config.scheduler.foreground_limit, 9);
    assert_eq!(effective.config.mode, GlobalMode::LocalOnly);
}

#[test]
fn some_none_clears_the_optional_purpose_backend_override() {
    let mut launch = RuntimePatch::empty();
    launch.purpose_backend[Purpose::CreatorPortrait.index()] = Some(Some(BackendId::OpenAi));
    let mut owner = build("", &[], &launch, 1);
    assert_eq!(
        owner
            .read_effective()
            .config
            .purpose(Purpose::CreatorPortrait)
            .backend_override,
        Some(BackendId::OpenAi)
    );

    let untouched = RuntimePatch::empty();
    owner.apply(untouched, 1).unwrap();
    assert_eq!(
        owner
            .read_effective()
            .config
            .purpose(Purpose::CreatorPortrait)
            .backend_override,
        Some(BackendId::OpenAi)
    );

    let mut clear = RuntimePatch::empty();
    clear.purpose_backend[Purpose::CreatorPortrait.index()] = Some(None);
    owner.apply(clear, 2).unwrap();
    let effective = owner.read_effective();
    assert_eq!(
        effective
            .config
            .purpose(Purpose::CreatorPortrait)
            .backend_override,
        None
    );
    assert_eq!(
        effective
            .provenance
            .source_of("purposes.creator_portrait.backend_override"),
        Some(OverlaySource::Runtime)
    );
}

#[test]
fn reset_clears_runtime_and_keeps_launch() {
    let mut launch = RuntimePatch::empty();
    launch.default_backend = Some(BackendId::CodexCli);
    launch.purpose_enabled[Purpose::AnimationFrame.index()] = Some(false);
    let mut owner = build("", &[], &launch, 3);
    let mut runtime = RuntimePatch::empty();
    runtime.default_backend = Some(BackendId::Bfl);
    runtime.mode = Some(GlobalMode::Off);
    owner.apply(runtime, 3).unwrap();

    let reset = owner.reset(4).unwrap();
    assert_eq!(reset.revision, 5);
    let effective = owner.read_effective();
    assert_eq!(effective.config.default_backend, BackendId::CodexCli);
    assert!(!effective.config.purpose(Purpose::AnimationFrame).enabled);
    assert_eq!(effective.config.mode, GlobalMode::Live);
    assert_eq!(
        effective.provenance.source_of("default_backend"),
        Some(OverlaySource::Launch)
    );
    assert_eq!(effective.provenance.revision, 5);
}

#[test]
fn all_global_modes_apply() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let mut revision = 1;
    for mode in [
        GlobalMode::Live,
        GlobalMode::LocalOnly,
        GlobalMode::CacheOnly,
        GlobalMode::Off,
    ] {
        owner.apply(launch_mode(mode), revision).unwrap();
        revision += 1;
        assert_eq!(owner.read_effective().config.mode, mode);
    }
    assert_eq!(owner.revision(), revision);
}

#[test]
fn backend_selection_and_bounds_update_with_runtime_sources() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let mut patch = RuntimePatch::empty();
    patch.backend_enabled[BackendId::Fal.index()] = Some(true);
    patch.backend_account[BackendId::OpenAi.index()] =
        Some(AccountHandle::new("ops-main").unwrap());
    patch.backend_concurrency[BackendId::OpenAi.index()] = Some(5);
    patch.transforms_max_decoded_bytes = Some(1024);
    patch.transforms_queue_capacity = Some(9);
    patch.transforms_deadline_secs = Some(45);
    patch.scheduler_job_deadline_secs = Some(300);
    patch.scheduler_stage_deadline_secs = Some(200);
    patch.request_max_prompt_chars = Some(1000);
    patch.request_max_composite_layers = Some(4);
    owner.apply(patch, 1).unwrap();

    let effective = owner.read_effective();
    assert!(effective.config.backend(BackendId::Fal).enabled);
    assert_eq!(
        effective.config.backend(BackendId::OpenAi).account.as_str(),
        "ops-main"
    );
    assert_eq!(
        effective
            .config
            .backend(BackendId::OpenAi)
            .max_concurrent_jobs,
        5
    );
    assert_eq!(effective.config.transforms.max_decoded_bytes, 1024);
    assert_eq!(effective.config.transforms.queue_capacity, 9);
    assert_eq!(effective.config.transforms.deadline_secs, 45);
    assert_eq!(effective.config.scheduler.job_deadline_secs, 300);
    assert_eq!(effective.config.scheduler.stage_deadline_secs, 200);
    assert_eq!(effective.config.request_limits.max_prompt_chars, 1000);
    assert_eq!(effective.config.request_limits.max_composite_layers, 4);
    for field in [
        "backends.fal.enabled",
        "backends.openai.account",
        "transforms.max_decoded_bytes",
        "scheduler.job_deadline_secs",
        "request_limits.max_prompt_chars",
    ] {
        assert_eq!(
            effective.provenance.source_of(field),
            Some(OverlaySource::Runtime),
            "{field}"
        );
    }
}

#[test]
fn invalid_apply_and_preview_leave_complete_state_unchanged() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 4);
    let before = owner.read_effective();
    let mut bad = RuntimePatch::empty();
    bad.scheduler_job_deadline_secs = Some(100);
    bad.scheduler_stage_deadline_secs = Some(170);
    let error = owner.apply(bad.clone(), 4).unwrap_err();
    match &error {
        ConfigApplyError::Invalid(OverlayError::InvalidField { field, .. }) => {
            assert_eq!(field, "scheduler.stage_deadline_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(owner.revision(), 4);
    assert_eq!(owner.read_effective(), before);

    let preview_error = owner.preview(&bad).unwrap_err();
    assert_eq!(preview_error, error);
    assert_eq!(owner.read_effective(), before);

    let mut good = RuntimePatch::empty();
    good.scheduler_queue_age_secs = Some(77);
    let previewed = owner.preview(&good).unwrap();
    assert_eq!(previewed.config.scheduler.queue_age_secs, 77);
    assert_eq!(previewed.provenance.revision, 4);
    assert_eq!(owner.read_effective(), before);
}

#[test]
fn stale_expected_revision_rejects_the_second_writer() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 5);
    owner.apply(launch_mode(GlobalMode::Off), 5).unwrap();
    let error = owner.apply(launch_mode(GlobalMode::Live), 5).unwrap_err();
    assert_eq!(
        error,
        ConfigApplyError::StaleRevision {
            expected: 5,
            current: 6
        }
    );
    let effective = owner.read_effective();
    assert_eq!(effective.config.mode, GlobalMode::Off);
    assert_eq!(effective.provenance.revision, 6);

    let reset_error = owner.reset(5).unwrap_err();
    assert_eq!(
        reset_error,
        ConfigApplyError::StaleRevision {
            expected: 5,
            current: 6
        }
    );
    assert_eq!(owner.read_effective(), effective);
}

#[test]
fn revision_overflow_leaves_state_unchanged() {
    let mut owner = build("", &[], &RuntimePatch::empty(), u64::MAX);
    let before = owner.read_effective();
    let error = owner.apply(RuntimePatch::empty(), u64::MAX).unwrap_err();
    assert_eq!(error, ConfigApplyError::RevisionOverflow);
    assert_eq!(owner.reset(u64::MAX).unwrap_err(), error);
    assert_eq!(owner.revision(), u64::MAX);
    assert_eq!(owner.read_effective(), before);
}

#[test]
fn snapshots_stay_independent_of_later_updates() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let snapshot = owner.read_effective();
    owner.apply(launch_mode(GlobalMode::Off), 1).unwrap();
    assert_eq!(snapshot.config.mode, GlobalMode::Live);
    assert_eq!(snapshot.provenance.revision, 1);
    assert_eq!(owner.read_effective().config.mode, GlobalMode::Off);
}

#[test]
fn prepared_export_matches_revision_and_save_uses_the_capture() {
    let mut owner = build("", &[], &launch_mode(GlobalMode::CacheOnly), 8);
    owner.apply(launch_mode(GlobalMode::Off), 8).unwrap();
    let prepared = owner.prepare_export();
    assert_eq!(prepared.revision(), 9);
    let effective = owner.read_effective();
    assert_eq!(prepared.config(), &effective.config);
    assert_eq!(prepared.provenance(), &effective.provenance);

    owner.apply(launch_mode(GlobalMode::Live), 9).unwrap();
    let receipt = ConfigControl::save_prepared(&prepared, |config, provenance, revision| {
        assert_eq!(revision, 9);
        assert_eq!(config.mode, GlobalMode::Off);
        assert_eq!(provenance.revision, 9);
        Ok::<String, ()>("launcher/db.toml".to_owned())
    })
    .unwrap();
    assert_eq!(receipt.revision, 9);
    assert_eq!(receipt.path, "launcher/db.toml");
}

#[test]
fn save_failure_surfaces_without_false_success() {
    let owner = build("", &[], &RuntimePatch::empty(), 2);
    let prepared = owner.prepare_export();
    let error =
        ConfigControl::save_prepared(&prepared, |_, _, _| Err::<String, ()>(())).unwrap_err();
    assert_eq!(error, ConfigApplyError::PersistFailed);
}

#[test]
fn pure_validate_accepts_and_refuses_without_mutation() {
    let owner = build("", &[], &RuntimePatch::empty(), 1);
    owner.validate(&ImageConfig::defaults().unwrap()).unwrap();
    let mut broken = ImageConfig::defaults().unwrap();
    broken.transforms.max_concurrency = 0;
    let error = owner.validate(&broken).unwrap_err();
    assert_eq!(
        error,
        crate::config::ConfigError::InvalidField {
            field: "transforms.max_concurrency",
            reason: "a zero bound refuses all work",
        }
    );
    assert_eq!(
        owner.read_effective().config,
        ImageConfig::defaults().unwrap()
    );
}

#[test]
fn invalid_launch_leaves_no_owner_behind() {
    let mut launch = RuntimePatch::empty();
    launch.scheduler_job_deadline_secs = Some(100);
    launch.scheduler_stage_deadline_secs = Some(170);
    let error = ConfigControl::build("", &[], &launch, 1).unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "scheduler.stage_deadline_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn launch_and_runtime_cover_the_new_setting_groups() {
    use crate::config::{
        ApiEndpoint, CliProfileRevision, CodexExecutable, JobWorkDir, ModelName, OutputProfileName,
    };

    let mut launch = RuntimePatch::empty();
    launch.codex_executable = Some(CodexExecutable::new("/usr/local/bin/codex").unwrap());
    launch.codex_workdir = Some(JobWorkDir::new("artifacts/tmp/jobs").unwrap());
    launch.codex_cli_profile = Some(CliProfileRevision::new("codex-images-v2").unwrap());
    launch.backend_model[BackendId::OpenAi.index()] =
        Some(Some(ModelName::new("gpt-image-1").unwrap()));
    launch.retry_max_attempts = Some(3);
    launch.accounting_mode = Some(crate::config::AccountingMode::Hosted);
    launch.quality_jpeg_quality = Some(crate::output::LossyQuality::new(70).unwrap());
    let mut owner = build("", &[], &launch, 1);
    let effective = owner.read_effective();
    assert_eq!(
        effective.config.codex.executable.as_str(),
        "/usr/local/bin/codex"
    );
    assert_eq!(
        effective
            .config
            .backend(BackendId::OpenAi)
            .model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-image-1"
    );
    assert_eq!(
        effective.provenance.source_of("codex.executable"),
        Some(OverlaySource::Launch)
    );
    assert_eq!(
        effective.provenance.source_of("backends.openai.model"),
        Some(OverlaySource::Launch)
    );

    let mut runtime = RuntimePatch::empty();
    runtime.codex_agent_model = Some(Some(ModelName::new("gpt-5").unwrap()));
    runtime.backend_endpoint[BackendId::Fal.index()] =
        Some(Some(ApiEndpoint::new("http://127.0.0.1:8080/v1").unwrap()));
    runtime.default_output_profile = Some(OutputProfileName::new("default").unwrap());
    runtime.purpose_output_profile[Purpose::ScenePlate.index()] =
        Some(Some(OutputProfileName::new("default").unwrap()));
    runtime.retry_deadline_secs = Some(120);
    owner.apply(runtime, 1).unwrap();
    let effective = owner.read_effective();
    assert_eq!(
        effective
            .config
            .codex
            .agent_model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-5"
    );
    assert_eq!(effective.config.retry.retry_deadline_secs, 120);
    assert_eq!(
        effective.config.codex.executable.as_str(),
        "/usr/local/bin/codex"
    );
    assert_eq!(
        effective.provenance.source_of("codex.agent_model"),
        Some(OverlaySource::Runtime)
    );
    assert_eq!(
        effective.provenance.source_of("codex.executable"),
        Some(OverlaySource::Launch)
    );
}

#[test]
fn invalid_new_settings_preserve_the_live_snapshot() {
    use crate::config::OutputProfileName;

    let mut owner = build("", &[], &RuntimePatch::empty(), 4);
    let before = owner.read_effective();
    let mut bad = RuntimePatch::empty();
    bad.retry_max_attempts = Some(0);
    bad.retry_deadline_secs = Some(300);
    let error = owner.apply(bad, 4).unwrap_err();
    match &error {
        ConfigApplyError::Invalid(OverlayError::InvalidField { field, .. }) => {
            assert_eq!(field, "retry.max_attempts");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(owner.revision(), 4);
    assert_eq!(owner.read_effective(), before);

    let mut dangling = RuntimePatch::empty();
    dangling.default_output_profile = Some(OutputProfileName::new("missing").unwrap());
    let error = owner.apply(dangling, 4).unwrap_err();
    match &error {
        ConfigApplyError::Invalid(OverlayError::InvalidField { field, .. }) => {
            assert_eq!(field, "outputs.default_profile");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(owner.read_effective(), before);

    let mut preview = RuntimePatch::empty();
    preview.codex_executable =
        Some(crate::config::CodexExecutable::new("/usr/local/bin/codex").unwrap());
    let previewed = owner.preview(&preview).unwrap();
    assert_eq!(
        previewed.config.codex.executable.as_str(),
        "/usr/local/bin/codex"
    );
    assert_eq!(owner.read_effective(), before);
}

#[test]
fn reset_keeps_launch_selections_for_the_new_groups() {
    use crate::config::{ModelName, OutputProfileName};

    let mut launch = RuntimePatch::empty();
    launch.codex_agent_model = Some(Some(ModelName::new("gpt-5").unwrap()));
    launch.purpose_output_profile[Purpose::ScenePlate.index()] =
        Some(Some(OutputProfileName::new("default").unwrap()));
    let mut owner = build("", &[], &launch, 3);
    let mut runtime = RuntimePatch::empty();
    runtime.codex_agent_model = Some(None);
    runtime.retry_max_attempts = Some(5);
    owner.apply(runtime, 3).unwrap();
    assert_eq!(owner.read_effective().config.retry.max_attempts, 5);
    owner.reset(4).unwrap();
    let effective = owner.read_effective();
    assert_eq!(
        effective
            .config
            .codex
            .agent_model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-5"
    );
    assert_eq!(
        effective
            .config
            .purpose(Purpose::ScenePlate)
            .output_profile
            .as_ref()
            .unwrap()
            .as_str(),
        "default"
    );
    assert_eq!(effective.config.retry.max_attempts, 2);
    assert_eq!(
        effective.provenance.source_of("codex.agent_model"),
        Some(OverlaySource::Launch)
    );
}

#[test]
fn provider_quality_applies_accumulates_and_resets() {
    use crate::config::GenerationQuality;
    use crate::config_overlay::parse_runtime_value;

    let mut launch = RuntimePatch::empty();
    parse_runtime_value(&mut launch, "quality.provider_quality", "high").unwrap();
    let mut owner = build("", &[], &launch, 1);
    assert_eq!(
        owner
            .read_effective()
            .config
            .quality
            .provider_quality
            .unwrap(),
        GenerationQuality::High
    );
    assert_eq!(
        owner
            .read_effective()
            .provenance
            .source_of("quality.provider_quality"),
        Some(OverlaySource::Launch)
    );

    let mut runtime = RuntimePatch::empty();
    parse_runtime_value(&mut runtime, "quality.provider_quality", "none").unwrap();
    owner.apply(runtime, 1).unwrap();
    let effective = owner.read_effective();
    assert_eq!(effective.config.quality.provider_quality, None);
    assert_eq!(
        effective.provenance.source_of("quality.provider_quality"),
        Some(OverlaySource::Runtime)
    );

    owner.reset(2).unwrap();
    let effective = owner.read_effective();
    assert_eq!(
        effective.config.quality.provider_quality,
        Some(GenerationQuality::High)
    );
    assert_eq!(
        effective.provenance.source_of("quality.provider_quality"),
        Some(OverlaySource::Launch)
    );
}

#[test]
fn partial_profile_edits_accumulate_with_per_field_provenance() {
    use crate::config_overlay::parse_runtime_value;

    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let mut first = RuntimePatch::empty();
    parse_runtime_value(&mut first, "outputs.hero-thumbnail.width_px", "96").unwrap();
    owner.apply(first, 1).unwrap();
    let mut second = RuntimePatch::empty();
    parse_runtime_value(&mut second, "outputs.hero-thumbnail.quality", "65").unwrap();
    parse_runtime_value(&mut second, "quality.provider_quality", "medium").unwrap();
    owner.apply(second, 2).unwrap();

    let effective = owner.read_effective();
    let edited = effective
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .unwrap();
    assert_eq!(edited.profile.delivered().dimensions().width_px(), 96);
    assert_eq!(
        effective.config.quality.provider_quality.unwrap(),
        crate::config::GenerationQuality::Medium
    );
    assert_eq!(
        effective
            .provenance
            .source_of("outputs.hero-thumbnail.width_px"),
        Some(OverlaySource::Runtime)
    );
    assert_eq!(
        effective
            .provenance
            .source_of("outputs.hero-thumbnail.quality"),
        Some(OverlaySource::Runtime)
    );

    owner.reset(3).unwrap();
    let effective = owner.read_effective();
    assert_eq!(effective.config, ImageConfig::defaults().unwrap());
}

#[test]
fn clearing_matte_removes_flattening_and_reset_restores_it() {
    use crate::config_overlay::parse_runtime_value;
    use crate::output::AlphaHandling;
    let text = "[imagery.outputs.default]\nalpha = \"matte\"\nmatte = \"#123456\"\n";
    let mut owner = build(text, &[], &RuntimePatch::empty(), 1);
    let before = owner.read_effective();
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "outputs.default.matte", "none").unwrap();
    owner.apply(patch, 1).unwrap();
    assert_eq!(
        owner
            .read_effective()
            .config
            .output_profile_for(Purpose::ScenePlate)
            .unwrap()
            .delivered()
            .alpha(),
        AlphaHandling::Opaque
    );
    owner.reset(2).unwrap();
    assert_eq!(owner.read_effective().config, before.config);
}

#[test]
fn operation_model_patches_accumulate_clear_and_reset() {
    use crate::config_overlay::parse_runtime_value;
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(
        &mut launch,
        "backends.fal.generation_model",
        "launch-generation",
    )
    .unwrap();
    let mut owner = build("", &[], &launch, 1);
    let mut first = RuntimePatch::empty();
    parse_runtime_value(&mut first, "backends.fal.edit_model", "runtime-edit").unwrap();
    owner.apply(first, 1).unwrap();
    let mut second = RuntimePatch::empty();
    parse_runtime_value(&mut second, "backends.fal.alpha_model", "runtime-alpha").unwrap();
    owner.apply(second, 2).unwrap();
    let config = owner.read_effective().config;
    assert_eq!(
        config
            .backend(BackendId::Fal)
            .edit_model
            .as_ref()
            .unwrap()
            .as_str(),
        "runtime-edit"
    );
    assert_eq!(
        config
            .backend(BackendId::Fal)
            .alpha_model
            .as_ref()
            .unwrap()
            .as_str(),
        "runtime-alpha"
    );
    owner.reset(3).unwrap();
    let config = owner.read_effective().config;
    assert_eq!(config.backend(BackendId::Fal).edit_model, None);
    assert_eq!(config.backend(BackendId::Fal).alpha_model, None);
    assert_eq!(
        config
            .backend(BackendId::Fal)
            .generation_model
            .as_ref()
            .unwrap()
            .as_str(),
        "launch-generation"
    );
}

#[test]
fn codex_stream_tuning_survives_unrelated_runtime_edits() {
    let mut owner = build("", &[], &RuntimePatch::empty(), 1);
    let mut limits = RuntimePatch::empty();
    limits.codex_max_stdout_bytes = Some(33554432);
    limits.codex_max_stderr_bytes = Some(2097152);
    owner.apply(limits, 1).unwrap();
    let mut mode = RuntimePatch::empty();
    mode.mode = Some(GlobalMode::CacheOnly);
    owner.apply(mode, 2).unwrap();
    let effective = owner.read_effective();
    assert_eq!(effective.config.codex.max_stdout_bytes, 33554432);
    assert_eq!(effective.config.codex.max_stderr_bytes, 2097152);
    assert_eq!(
        effective.provenance.source_of("codex.max_stdout_bytes"),
        Some(OverlaySource::Runtime)
    );
}

#[test]
fn codex_message_limit_accumulates_atomically_and_reset_restores_launch() {
    use crate::config_overlay::parse_runtime_value;
    let text = "[imagery.codex]\nmax_message_bytes = 25165824\n";
    let env = [("DF_IMAGERY_CODEX_MAX_MESSAGE_BYTES", "26214400")];
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(&mut launch, "codex.max_message_bytes", "27262976").unwrap();
    let mut owner = build(text, &env, &launch, 1);
    let initial = owner.read_effective();
    assert_eq!(initial.config.codex.max_message_bytes, 27_262_976);
    assert_eq!(
        initial.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::Launch)
    );
    let mut runtime = RuntimePatch::empty();
    parse_runtime_value(&mut runtime, "codex.max_message_bytes", "31457280").unwrap();
    owner.apply(runtime, 1).unwrap();
    let mut unrelated = RuntimePatch::empty();
    unrelated.mode = Some(GlobalMode::CacheOnly);
    owner.apply(unrelated, 2).unwrap();
    let effective = owner.read_effective();
    assert_eq!(effective.config.codex.max_message_bytes, 31_457_280);
    assert_eq!(
        effective.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::Runtime)
    );
    let saved = owner.prepare_export();
    assert_eq!(saved.config().codex.max_message_bytes, 31_457_280);
    let mut invalid = RuntimePatch::empty();
    invalid.codex_max_message_bytes = Some(0);
    assert!(owner.apply(invalid, 3).is_err());
    assert_eq!(owner.read_effective(), effective);
    owner.reset(3).unwrap();
    let reset = owner.read_effective();
    assert_eq!(reset.config.codex.max_message_bytes, 27_262_976);
    assert_eq!(
        reset.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::Launch)
    );
    assert_eq!(
        reset.config.codex.max_stdout_bytes,
        initial.config.codex.max_stdout_bytes
    );
    assert_eq!(
        reset.config.codex.max_stderr_bytes,
        initial.config.codex.max_stderr_bytes
    );
    assert_eq!(saved.config().codex.max_message_bytes, 31_457_280);
}

#[test]
fn trusted_analysis_route_edits_accumulate_clear_and_reset_to_launch() {
    use crate::config_overlay::parse_runtime_value;
    let text = "[imagery.analysis.inspection]\naccount = \"file-analysis\"\nmodel = \"custom/file-model\"\nendpoint = \"http://localhost:9080/responses\"\nmaximum_output_tokens = 1024\n";
    let env = [(
        "DF_IMAGERY_ANALYSIS_INSPECTION_ACCOUNT",
        "environment-analysis",
    )];
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(
        &mut launch,
        "analysis.inspection.model",
        "custom/launch-model",
    )
    .unwrap();
    let mut owner = build(text, &env, &launch, 1);
    let initial = owner.read_effective();
    assert_eq!(
        initial
            .config
            .analysis
            .inspection
            .as_ref()
            .unwrap()
            .account
            .as_str(),
        "environment-analysis"
    );
    assert_eq!(
        initial.provenance.source_of("analysis.inspection.model"),
        Some(OverlaySource::Launch)
    );
    let mut endpoint = RuntimePatch::empty();
    parse_runtime_value(
        &mut endpoint,
        "analysis.inspection.endpoint",
        "http://localhost:9081/custom",
    )
    .unwrap();
    owner.apply(endpoint, 1).unwrap();
    let mut tokens = RuntimePatch::empty();
    parse_runtime_value(
        &mut tokens,
        "analysis.inspection.maximum_output_tokens",
        "8192",
    )
    .unwrap();
    owner.apply(tokens, 2).unwrap();
    let effective = owner.read_effective();
    let route = effective.config.analysis.inspection.as_ref().unwrap();
    assert_eq!(route.endpoint.as_str(), "http://localhost:9081/custom");
    assert_eq!(route.maximum_output_tokens, 8192);
    assert_eq!(route.model.as_str(), "custom/launch-model");
    assert_eq!(
        effective
            .provenance
            .source_of("analysis.inspection.endpoint"),
        Some(OverlaySource::Runtime)
    );
    let saved = owner.prepare_export();
    let mut invalid = RuntimePatch::empty();
    parse_runtime_value(
        &mut invalid,
        "analysis.inspection.maximum_output_tokens",
        "0",
    )
    .unwrap();
    assert!(owner.apply(invalid, 3).is_err());
    assert_eq!(owner.read_effective(), effective);
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "analysis.inspection", "none").unwrap();
    owner.apply(clear, 3).unwrap();
    assert!(owner.read_effective().config.analysis.inspection.is_none());
    owner.reset(4).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(
        saved
            .config()
            .analysis
            .inspection
            .as_ref()
            .unwrap()
            .endpoint
            .as_str(),
        "http://localhost:9081/custom"
    );
}

#[test]
fn purpose_model_and_quality_overrides_accumulate_then_reset_to_launch() {
    use crate::config::GenerationQuality;
    use crate::config_overlay::parse_runtime_value;
    let text = "[imagery.purposes.animation_frame]\nbackend = \"bfl\"\nquality_override = \"low\"\nmodel_override = \"custom/file-frame\"\n";
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(
        &mut launch,
        "purposes.animation_frame.model_override",
        "gpt-image-2.5-sunburst",
    )
    .unwrap();
    parse_runtime_value(
        &mut launch,
        "purposes.animation_frame.quality_override",
        "medium",
    )
    .unwrap();
    let mut owner = build(text, &[], &launch, 1);
    let initial = owner.read_effective();
    assert_eq!(
        initial
            .provenance
            .source_of("purposes.animation_frame.model_override"),
        Some(OverlaySource::Launch)
    );
    let mut quality = RuntimePatch::empty();
    parse_runtime_value(
        &mut quality,
        "purposes.animation_frame.quality_override",
        "high",
    )
    .unwrap();
    owner.apply(quality, 1).unwrap();
    let mut unrelated = RuntimePatch::empty();
    unrelated.mode = Some(GlobalMode::CacheOnly);
    owner.apply(unrelated, 2).unwrap();
    let effective = owner.read_effective();
    let frame = effective.config.purpose(Purpose::AnimationFrame);
    assert_eq!(
        frame.model_override.as_ref().unwrap().as_str(),
        "gpt-image-2.5-sunburst"
    );
    assert_eq!(frame.quality_override, Some(GenerationQuality::High));
    assert_eq!(frame.backend_override, Some(BackendId::Bfl));
    let captured = owner.prepare_export();
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.model_override",
        "none",
    )
    .unwrap();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.quality_override",
        "none",
    )
    .unwrap();
    owner.apply(clear, 3).unwrap();
    let cleared = owner.read_effective();
    assert!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .model_override
            .is_none()
    );
    assert!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .quality_override
            .is_none()
    );
    owner.reset(4).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(
        captured
            .config()
            .purpose(Purpose::AnimationFrame)
            .quality_override,
        Some(GenerationQuality::High)
    );
}

#[test]
fn moderation_and_analysis_preparation_edits_accumulate_reset_and_preserve_exports() {
    use crate::config_overlay::parse_runtime_value;
    use crate::request::ProviderModeration;
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(
        &mut launch,
        "purposes.animation_frame.moderation_override",
        "low",
    )
    .unwrap();
    parse_runtime_value(
        &mut launch,
        "analysis.preparation.max_request_bytes",
        "33554432",
    )
    .unwrap();
    let mut owner = build("", &[], &launch, 1);
    let initial = owner.read_effective();
    let mut first = RuntimePatch::empty();
    parse_runtime_value(
        &mut first,
        "purposes.animation_frame.moderation_override",
        "auto",
    )
    .unwrap();
    parse_runtime_value(&mut first, "analysis.preparation.max_pixels", "40000000").unwrap();
    owner.apply(first, 1).unwrap();
    let mut second = RuntimePatch::empty();
    parse_runtime_value(
        &mut second,
        "analysis.preparation.max_picture_bytes",
        "4194304",
    )
    .unwrap();
    owner.apply(second, 2).unwrap();
    let effective = owner.read_effective();
    assert_eq!(
        effective
            .config
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        Some(ProviderModeration::Auto)
    );
    assert_eq!(effective.config.analysis.preparation.max_pixels, 40_000_000);
    assert_eq!(
        effective.config.analysis.preparation.max_picture_bytes,
        4_194_304
    );
    assert_eq!(
        effective.config.analysis.preparation.max_request_bytes,
        33_554_432
    );
    let saved = owner.prepare_export();
    let mut zero = RuntimePatch::empty();
    zero.analysis_preparation_max_pixels = Some(0);
    assert!(owner.apply(zero, 3).is_err());
    assert_eq!(owner.read_effective(), effective);
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.moderation_override",
        "none",
    )
    .unwrap();
    owner.apply(clear, 3).unwrap();
    assert!(
        owner
            .read_effective()
            .config
            .purpose(Purpose::AnimationFrame)
            .moderation_override
            .is_none()
    );
    owner.reset(4).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(saved.config().analysis.preparation.max_pixels, 40_000_000);
    assert_eq!(
        saved
            .config()
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        Some(ProviderModeration::Auto)
    );
}

#[test]
fn trusted_defaults_precede_partial_routes_and_explicit_clears() {
    use crate::config::{ApiEndpoint, ModelName};
    use crate::config_overlay::parse_runtime_value;
    use crate::request::AnalysisRoute;
    let mut defaults = ImageConfig::defaults().unwrap();
    defaults.analysis.head_locations = Some(AnalysisRoute {
        account: AccountHandle::new("analysis-private").unwrap(),
        model: ModelName::new("original-model").unwrap(),
        endpoint: ApiEndpoint::new("https://analysis.example/v1").unwrap(),
        maximum_output_tokens: 2048,
    });
    defaults.purposes[Purpose::AnimationFrame.index()].account_override =
        Some(AccountHandle::new("creator-openai").unwrap());
    let file = "[imagery.analysis.head_locations]\nmodel = \"file-model\"\n";
    let env = [(
        "DF_IMAGERY_ANALYSIS_HEAD_LOCATIONS_ENDPOINT",
        "https://operator.example/v2",
    )];
    let mut owner =
        ConfigControl::build_with_defaults(defaults.clone(), file, &env, &RuntimePatch::empty(), 1)
            .unwrap();
    let initial = owner.read_effective();
    let route = initial.config.analysis.head_locations.as_ref().unwrap();
    assert_eq!(route.account.as_str(), "analysis-private");
    assert_eq!(route.model.as_str(), "file-model");
    assert_eq!(route.endpoint.as_str(), env[0].1);
    assert_eq!(
        initial
            .provenance
            .source_of("analysis.head_locations.account"),
        Some(OverlaySource::Default)
    );
    assert_eq!(
        initial
            .provenance
            .source_of("analysis.head_locations.model"),
        Some(OverlaySource::FileImagery)
    );
    let mut edit = RuntimePatch::empty();
    parse_runtime_value(
        &mut edit,
        "analysis.head_locations.maximum_output_tokens",
        "4096",
    )
    .unwrap();
    parse_runtime_value(
        &mut edit,
        "purposes.animation_frame.account_override",
        "operator-creator",
    )
    .unwrap();
    owner.apply(edit, 1).unwrap();
    let saved = owner.prepare_export();
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "analysis.head_locations", "none").unwrap();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.account_override",
        "none",
    )
    .unwrap();
    owner.apply(clear, 2).unwrap();
    assert!(
        owner
            .read_effective()
            .config
            .analysis
            .head_locations
            .is_none()
    );
    assert!(
        owner
            .read_effective()
            .config
            .purpose(Purpose::AnimationFrame)
            .account_override
            .is_none()
    );
    owner.reset(3).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(
        saved
            .config()
            .analysis
            .head_locations
            .as_ref()
            .unwrap()
            .maximum_output_tokens,
        4096
    );
    assert_eq!(
        saved
            .config()
            .purpose(Purpose::AnimationFrame)
            .account_override
            .as_ref()
            .unwrap()
            .as_str(),
        "operator-creator"
    );
    let explicit = ConfigControl::build_with_defaults(defaults.clone(),
        "[imagery.analysis]\nhead_locations = \"none\"\n[imagery.purposes.animation_frame]\naccount_override = \"none\"\n",
        &[], &RuntimePatch::empty(), 1).unwrap();
    assert!(
        explicit
            .read_effective()
            .config
            .analysis
            .head_locations
            .is_none()
    );
    assert!(
        explicit
            .read_effective()
            .config
            .purpose(Purpose::AnimationFrame)
            .account_override
            .is_none()
    );
    defaults.analysis.preparation.max_pixels = 0;
    assert!(
        ConfigControl::build_with_defaults(defaults, "", &[], &RuntimePatch::empty(), 1).is_err()
    );
}

#[test]
fn standalone_route_runtime_edits_merge_export_and_reset() {
    use crate::config_overlay::parse_runtime_value;
    let file = concat!(
        "[imagery.analysis.segmentation]\nprovider = \"fal\"\naccount = \"sam-account\"\n",
        "model = \"fal-ai/sam-3-1/image-rle\"\nendpoint = \"https://queue.fal.run\"\n",
    );
    let mut owner = build(file, &[], &RuntimePatch::empty(), 1);
    let initial = owner.read_effective();
    let mut edit = RuntimePatch::empty();
    parse_runtime_value(
        &mut edit,
        "analysis.segmentation.model",
        "private/sam-contract",
    )
    .unwrap();
    parse_runtime_value(
        &mut edit,
        "analysis.segmentation_preparation.max_pixels",
        "80000000",
    )
    .unwrap();
    owner.apply(edit, 1).unwrap();
    let mut second = RuntimePatch::empty();
    parse_runtime_value(
        &mut second,
        "analysis.segmentation.endpoint",
        "https://private.example/queue",
    )
    .unwrap();
    owner.apply(second, 2).unwrap();
    let export = owner.prepare_export();
    let route = export.config().analysis.segmentation.as_ref().unwrap();
    assert_eq!(route.model.as_str(), "private/sam-contract");
    assert_eq!(route.account.as_str(), "sam-account");
    assert_eq!(route.endpoint.as_str(), "https://private.example/queue");
    assert_eq!(
        export.config().analysis.segmentation_preparation.max_pixels,
        80000000
    );
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "analysis.segmentation", "none").unwrap();
    owner.apply(clear, 3).unwrap();
    assert!(
        owner
            .read_effective()
            .config
            .analysis
            .segmentation
            .is_none()
    );
    owner.reset(4).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert!(export.config().analysis.segmentation.is_some());
}

#[test]
fn quota_edits_accumulate_export_and_reset_without_committing_invalid_limits() {
    use crate::config_overlay::parse_runtime_value;
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(&mut launch, "quota.cooldown_ms", "25").unwrap();
    let mut owner = build("[imagery.quota]\nmaximum_accounts=7\n", &[], &launch, 1);
    let initial = owner.read_effective();
    let mut disable = RuntimePatch::empty();
    parse_runtime_value(&mut disable, "quota.enabled", "off").unwrap();
    owner.apply(disable, 1).unwrap();
    let mut limit = RuntimePatch::empty();
    parse_runtime_value(&mut limit, "quota.maximum_accounts", "19").unwrap();
    owner.apply(limit, 2).unwrap();
    let effective = owner.read_effective();
    assert!(!effective.config.quota.enabled);
    assert_eq!(effective.config.quota.cooldown_ms, 25);
    assert_eq!(effective.config.quota.maximum_accounts, 19);
    assert_eq!(
        effective.provenance.source_of("quota.enabled"),
        Some(OverlaySource::Runtime)
    );
    assert_eq!(
        effective.provenance.source_of("quota.cooldown_ms"),
        Some(OverlaySource::Launch)
    );
    assert_eq!(
        effective.provenance.source_of("quota.maximum_accounts"),
        Some(OverlaySource::Runtime)
    );
    let saved = owner.prepare_export();
    let mut invalid = RuntimePatch::empty();
    invalid.quota_cooldown_ms = Some(0);
    assert!(owner.apply(invalid, 3).is_err());
    assert_eq!(owner.read_effective(), effective);
    assert_eq!(owner.revision(), 3);
    owner.reset(3).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(saved.config().quota, effective.config.quota);
}

#[test]
fn creator_alternate_account_clear_survives_later_edits_export_and_reset() {
    use crate::config_overlay::parse_runtime_value;
    let mut defaults = ImageConfig::defaults().unwrap();
    defaults.creator_bfl_account = Some(AccountHandle::new("seeded-bfl").unwrap());
    let mut launch = RuntimePatch::empty();
    parse_runtime_value(&mut launch, "creator.bfl_account", "launch-bfl").unwrap();
    let mut owner = ConfigControl::build_with_defaults(defaults, "", &[], &launch, 1).unwrap();
    let initial = owner.read_effective();
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "creator.bfl_account", "none").unwrap();
    owner.apply(clear, 1).unwrap();
    let mut unrelated = RuntimePatch::empty();
    unrelated.quota_enabled = Some(false);
    owner.apply(unrelated, 2).unwrap();
    let saved = owner.prepare_export();
    assert!(owner.read_effective().config.creator_bfl_account.is_none());
    assert_eq!(
        owner
            .read_effective()
            .provenance
            .source_of("creator.bfl_account"),
        Some(OverlaySource::Runtime)
    );
    owner.reset(3).unwrap();
    assert_eq!(owner.read_effective().config, initial.config);
    assert_eq!(
        owner
            .read_effective()
            .provenance
            .source_of("creator.bfl_account"),
        Some(OverlaySource::Launch)
    );
    assert!(saved.config().creator_bfl_account.is_none());
}
