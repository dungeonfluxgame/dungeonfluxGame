use crate::config::{BackendId, GlobalMode, ImageConfig};
use crate::config_overlay::{
    OverlayError, OverlaySource, RuntimePatch, apply_runtime, parse_runtime_value, resolve,
};
use crate::request::Purpose;

fn empty_patch() -> RuntimePatch {
    RuntimePatch::empty()
}

fn resolve_text(text: &str) -> crate::config_overlay::ResolvedConfig {
    resolve(text, &[], &empty_patch(), 7).unwrap()
}

#[test]
fn defaults_hold_when_all_layers_are_empty() {
    let resolved = resolve("", &[], &empty_patch(), 7).unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    assert_eq!(resolved.provenance.revision, 7);
    assert_eq!(
        resolved.provenance.source_of("mode"),
        Some(OverlaySource::Default)
    );
    assert!(resolved.provenance.ignored.is_empty());
}

#[test]
fn all_four_global_modes_parse_from_file() {
    for (text, mode) in [
        ("live", GlobalMode::Live),
        ("local-only", GlobalMode::LocalOnly),
        ("cache-only", GlobalMode::CacheOnly),
        ("off", GlobalMode::Off),
    ] {
        let file = format!("[imagery]\nmode = \"{text}\"\n");
        let resolved = resolve_text(&file);
        assert_eq!(resolved.config.mode, mode, "{text}");
        assert_eq!(
            resolved.provenance.source_of("mode"),
            Some(OverlaySource::FileImagery)
        );
    }
}

#[test]
fn unknown_mode_is_refused_in_file_and_env() {
    let error = resolve("[imagery]\nmode = \"paused\"\n", &[], &empty_patch(), 1).unwrap_err();
    assert_eq!(
        error,
        OverlayError::InvalidField {
            field: "imagery.mode".to_owned(),
            reason: "unknown mode",
        }
    );
    let error = resolve("", &[("DF_IMAGERY_MODE", "paused")], &empty_patch(), 1).unwrap_err();
    assert_eq!(
        error,
        OverlayError::InvalidField {
            field: "DF_IMAGERY_MODE".to_owned(),
            reason: "unknown mode",
        }
    );
}

#[test]
fn every_purpose_toggles_from_file_with_provenance() {
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        let file = format!(
            "[imagery.purposes.{}]\nenabled = false\n",
            purpose.label()
        );
        let resolved = resolve_text(&file);
        assert!(!resolved.config.purpose(purpose).enabled, "{}", purpose.label());
        let field = format!("purposes.{}.enabled", purpose.label());
        assert_eq!(
            resolved.provenance.source_of(&field),
            Some(OverlaySource::FileImagery),
            "{field}"
        );
    }
}

#[test]
fn every_purpose_toggles_from_new_env() {
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        let tag = purpose.label().to_ascii_uppercase();
        let name = format!("DF_IMAGERY_PURPOSE_{tag}_ENABLED");
        let pairs = vec![(name.as_str(), "off")];
        let resolved = resolve("", &pairs, &empty_patch(), 1).unwrap();
        assert!(!resolved.config.purpose(purpose).enabled, "{}", purpose.label());
    }
}

#[test]
fn every_purpose_backend_override_sets_and_clears() {
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        let tag = purpose.label().to_ascii_uppercase();
        let name = format!("DF_IMAGERY_PURPOSE_{tag}_BACKEND");
        let pairs = vec![(name.as_str(), "bfl")];
        let resolved = resolve("", &pairs, &empty_patch(), 1).unwrap();
        assert_eq!(
            resolved.config.purpose(purpose).backend_override,
            Some(BackendId::Bfl)
        );
        let pairs = vec![(name.as_str(), "none")];
        let resolved = resolve("", &pairs, &empty_patch(), 1).unwrap();
        assert_eq!(resolved.config.purpose(purpose).backend_override, None);
    }
}

#[test]
fn all_four_backend_gates_parse_from_file() {
    let file = "[imagery.backends.openai]\nenabled = true\naccount = \"ops-main\"\n\
        max_concurrent_jobs = 5\n[imagery.backends.fal]\nenabled = true\n\
        [imagery.backends.bfl]\nenabled = false\n[imagery.backends.codex-cli]\n\
        enabled = true\naccount = \"clip-01\"\n";
    let resolved = resolve_text(file);
    assert!(resolved.config.backend(BackendId::OpenAi).enabled);
    assert_eq!(resolved.config.backend(BackendId::OpenAi).account.as_str(), "ops-main");
    assert_eq!(resolved.config.backend(BackendId::OpenAi).max_concurrent_jobs, 5);
    assert!(resolved.config.backend(BackendId::Fal).enabled);
    assert!(!resolved.config.backend(BackendId::Bfl).enabled);
    assert_eq!(resolved.config.backend(BackendId::CodexCli).account.as_str(), "clip-01");
    assert_eq!(
        resolved.provenance.source_of("backends.openai.account"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn underscore_codex_section_spelling_is_accepted() {
    let resolved = resolve_text("[imagery.backends.codex_cli]\nenabled = true\n");
    assert!(resolved.config.backend(BackendId::CodexCli).enabled);
}

#[test]
fn unknown_backend_section_is_refused() {
    let error = resolve_text_err("[imagery.backends.midjourney]\nenabled = true\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "imagery.backends.midjourney.enabled".to_owned(),
        }
    );
}

#[test]
fn unknown_purpose_section_is_refused() {
    let error = resolve_text_err("[imagery.purposes.midjourney]\nenabled = true\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "imagery.purposes.midjourney.enabled".to_owned(),
        }
    );
}

fn resolve_text_err(text: &str) -> OverlayError {
    resolve(text, &[], &empty_patch(), 1).unwrap_err()
}

#[test]
fn invalid_account_handle_is_refused_without_echo() {
    let error = resolve_text_err("[imagery.backends.openai]\naccount = \"ops/key:1\"\n");
    match &error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "imagery.backends.openai.account");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(!error.to_string().contains("ops/key:1"));
}

#[test]
fn precedence_climbs_from_defaults_to_runtime() {
    let file = "[scheduler]\nqueue_age_secs = 111\n".replace("[scheduler]", "[imagery.scheduler]");
    let env = [
        ("DF_IMAGERY_SCHEDULER_QUEUE_AGE_SECS", "222"),
        ("DF_GAMEPLAY_IMAGERY", "on"),
    ];
    let mut patch = RuntimePatch::empty();
    patch.scheduler_queue_age_secs = Some(333);
    let resolved = resolve(&file, &env, &patch, 3).unwrap();
    assert_eq!(resolved.config.scheduler.queue_age_secs, 333);
    assert_eq!(
        resolved.provenance.source_of("scheduler.queue_age_secs"),
        Some(OverlaySource::Runtime)
    );
}

#[test]
fn file_layer_beats_legacy_env_but_loses_to_new_env() {
    let file = "[imagery.scheduler]\nqueue_age_secs = 111\n";
    let env = [("DF_IMAGERY_SCHEDULER_QUEUE_AGE_SECS", "222")];
    let resolved = resolve(file, &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.scheduler.queue_age_secs, 222);
    assert_eq!(
        resolved.provenance.source_of("scheduler.queue_age_secs"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_SCHEDULER_QUEUE_AGE_SECS".to_owned()
        ))
    );
}

#[test]
fn file_alias_conflict_names_both_spellings() {
    let file = "[imagery]\nbackend = \"openai\"\n[pictures]\npicture_provider = \"flux\"\n";
    let error = resolve_text_err(file);
    assert_eq!(
        error,
        OverlayError::AliasConflict {
            field: "default_backend".to_owned(),
            first: "imagery.backend".to_owned(),
            second: "pictures.picture_provider".to_owned(),
        }
    );
}

#[test]
fn new_env_wins_silently_over_legacy_env() {
    let env = [
        ("DF_IMAGERY_BACKEND", "openai"),
        ("DF_GAMEPLAY_IMAGERY_PROVIDER", "flux"),
    ];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.default_backend, BackendId::OpenAi);
    assert_eq!(
        resolved.provenance.source_of("default_backend"),
        Some(OverlaySource::NewEnv("DF_IMAGERY_BACKEND".to_owned()))
    );
}

#[test]
fn agreeing_aliases_merge_silently_on_the_new_spelling() {
    let file = "[imagery]\nbackend = \"openai\"\n[pictures]\npicture_provider = \"openai\"\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.default_backend, BackendId::OpenAi);
    assert_eq!(
        resolved.provenance.source_of("default_backend"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn unknown_file_key_is_refused() {
    let error = resolve_text_err("[imagery]\nfrobnicate = true\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "imagery.frobnicate".to_owned(),
        }
    );
}

#[test]
fn unknown_purpose_key_is_refused() {
    let error = resolve_text_err("[imagery.purposes.scene_plate]\nfrobnicate = true\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "imagery.purposes.scene_plate.frobnicate".to_owned(),
        }
    );
}

#[test]
fn unknown_pictures_key_is_refused() {
    let error = resolve_text_err("[pictures]\nscene_pictuers = true\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "pictures.scene_pictuers".to_owned(),
        }
    );
}

#[test]
fn unknown_new_env_key_is_refused() {
    let error = resolve("", &[("DF_IMAGERY_NOPE", "1")], &empty_patch(), 1).unwrap_err();
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "DF_IMAGERY_NOPE".to_owned(),
        }
    );
}

#[test]
fn unknown_legacy_imagery_env_key_is_refused() {
    let error = resolve("", &[("DF_GAMEPLAY_IMAGERY_MODLE", "x")], &empty_patch(), 1)
        .unwrap_err();
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "DF_GAMEPLAY_IMAGERY_MODLE".to_owned(),
        }
    );
}

#[test]
fn unrelated_env_names_pass_through_untouched() {
    let resolved = resolve(
        "",
        &[("DF_GAMEPLAY_LISTEN", "127.0.0.1:63309")],
        &empty_patch(),
        1,
    )
    .unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    assert!(resolved.provenance.ignored.is_empty());
}

#[test]
fn zero_bounds_are_refused() {
    let error = resolve_text_err("[imagery.transforms]\nmax_concurrency = 0\n");
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "transforms.max_concurrency");
        }
        other => panic!("unexpected {other:?}"),
    }
    let error = resolve("", &[("DF_IMAGERY_SCHEDULER_FOREGROUND_LIMIT", "0")], &empty_patch(), 1)
        .unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "scheduler.foreground_limit");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn overflow_counts_are_refused() {
    let error = resolve(
        "",
        &[("DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES", "99999999999999999999999")],
        &empty_patch(),
        1,
    )
    .unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES");
        }
        other => panic!("unexpected {other:?}"),
    }
    let error = resolve_text_err("[imagery.transforms]\nmax_concurrency = 4294967296\n");
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "imagery.transforms.max_concurrency");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn negative_file_counts_are_refused() {
    let error = resolve_text_err("[imagery.scheduler]\nqueue_age_secs = -5\n");
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "imagery.scheduler.queue_age_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn disabling_the_default_backend_keeps_the_selection_without_reroute() {
    let resolved = resolve_text("[imagery.backends.openai]\nenabled = false\n");
    assert!(!resolved.config.backend(BackendId::OpenAi).enabled);
    assert_eq!(resolved.config.default_backend, BackendId::OpenAi);
}

#[test]
fn legacy_scene_pictures_fans_out_to_scene_art_only() {
    let resolved = resolve_text("[pictures]\nscene_pictures = false\n");
    assert!(!resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert!(!resolved.config.purpose(Purpose::SceneEdit).enabled);
    assert!(!resolved.config.purpose(Purpose::CanonicalPerson).enabled);
    assert!(!resolved.config.purpose(Purpose::CanonicalRoom).enabled);
    assert!(!resolved.config.purpose(Purpose::Expression).enabled);
    assert!(!resolved.config.purpose(Purpose::ItemCloseUp).enabled);
    assert!(!resolved.config.purpose(Purpose::PartyMural).enabled);
    assert!(!resolved.config.purpose(Purpose::CombatStill).enabled);
    assert!(resolved.config.purpose(Purpose::CreatorPortrait).enabled);
    assert_eq!(
        resolved.provenance.source_of("purposes.scene_plate.enabled"),
        Some(OverlaySource::FilePictures)
    );
    assert_eq!(
        resolved.provenance.source_of("purposes.canonical_person.enabled"),
        Some(OverlaySource::FilePictures)
    );
}

#[test]
fn legacy_scene_plates_only_gates_plates() {
    let resolved = resolve_text("[pictures]\nscene_plates = false\n");
    assert!(!resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert!(resolved.config.purpose(Purpose::SceneEdit).enabled);
}

#[test]
fn legacy_creator_portraits_enable_their_purpose() {
    let file = "[pictures]\ncreator_portraits = true\n";
    let resolved = resolve_text(file);
    assert!(resolved.config.purpose(Purpose::CreatorPortrait).enabled);
    let env = [("DF_GAMEPLAY_CREATOR_PORTRAITS", "off")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert!(!resolved.config.purpose(Purpose::CreatorPortrait).enabled);
}

#[test]
fn legacy_alternate_route_is_acknowledged_without_backend_substitution() {
    let resolved = resolve_text("[pictures]\ncreator_alternate_route = true\n");
    assert_eq!(
        resolved.config.purpose(Purpose::CreatorPortrait).backend_override,
        None
    );
    assert!(resolved.provenance.ignored.iter().any(|entry| entry
        .contains("creator_alternate_route")));
}

#[test]
fn alternate_route_does_not_conflict_with_an_explicit_override() {
    let file = "[pictures]\ncreator_alternate_route = true\n\
        [imagery.purposes.creator_portrait]\nbackend = \"openai\"\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved.config.purpose(Purpose::CreatorPortrait).backend_override,
        Some(BackendId::OpenAi)
    );
    assert!(resolved.provenance.ignored.iter().any(|entry| entry
        .contains("creator_alternate_route")));
}

#[test]
fn character_video_off_disables_animation_frames() {
    let env = [("DF_GAMEPLAY_CHARACTER_VIDEO", "off")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert!(!resolved.config.purpose(Purpose::AnimationFrame).enabled);
}

#[test]
fn disable_openai_api_disables_only_the_openai_backend() {
    let env = [("DF_GAMEPLAY_DISABLE_OPENAI_API", "on")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert!(!resolved.config.backend(BackendId::OpenAi).enabled);
    assert_eq!(resolved.config.default_backend, BackendId::OpenAi);
}

#[test]
fn legacy_flux_provider_maps_to_fal_while_new_flux_is_refused() {
    let env = [("DF_GAMEPLAY_IMAGERY_PROVIDER", "flux")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.default_backend, BackendId::Fal);
    let error = resolve("", &[("DF_IMAGERY_BACKEND", "flux")], &empty_patch(), 1).unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => assert_eq!(field, "DF_IMAGERY_BACKEND"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn unrelated_credentials_are_ignored_without_cloning_values() {
    for name in ["OPENAI_API_KEY", "FAL_KEY", "DF_GAMEPLAY_LISTEN"] {
        let resolved = resolve("", &[(name, "private-value")], &empty_patch(), 1).unwrap();
        assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
        assert!(resolved.provenance.ignored.is_empty());
    }
}

#[test]
fn imagery_owned_secret_names_are_refused_without_echo() {
    let error = resolve(
        "",
        &[("DF_IMAGERY_PURPOSE_SCENE_PLATE_KEY", "private-value")],
        &empty_patch(),
        1,
    )
    .unwrap_err();
    assert_eq!(
        error,
        OverlayError::SecretRefused {
            name: "DF_IMAGERY_PURPOSE_SCENE_PLATE_KEY".to_owned(),
        }
    );
    assert!(!error.to_string().contains("private-value"));
}

#[test]
fn invalid_runtime_update_preserves_the_original() {
    let base = ImageConfig::defaults().unwrap();
    let mut patch = RuntimePatch::empty();
    patch.scheduler_job_deadline_secs = Some(100);
    patch.scheduler_stage_deadline_secs = Some(170);
    let error = apply_runtime(&base, &patch).unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "scheduler.stage_deadline_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(base, ImageConfig::defaults().unwrap());
}

#[test]
fn invalid_layer_leaves_no_partial_result() {
    let before = ImageConfig::defaults().unwrap();
    let file = "[imagery.scheduler]\njob_deadline_secs = 100\nstage_deadline_secs = 170\n";
    let error = resolve(file, &[], &empty_patch(), 1).unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "scheduler.stage_deadline_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(before, ImageConfig::defaults().unwrap());
}

#[test]
fn runtime_patch_applies_atomically_through_resolve() {
    let mut patch = RuntimePatch::empty();
    patch.mode = Some(GlobalMode::Off);
    patch.backend_concurrency[BackendId::CodexCli.index()] = Some(4);
    let resolved = resolve("", &[], &patch, 9).unwrap();
    assert_eq!(resolved.config.mode, GlobalMode::Off);
    assert_eq!(resolved.config.backend(BackendId::CodexCli).max_concurrent_jobs, 4);
    assert_eq!(resolved.provenance.revision, 9);
    assert_eq!(
        resolved.provenance.source_of("mode"),
        Some(OverlaySource::Runtime)
    );
}

#[test]
fn runtime_dotted_paths_set_every_scalar_group() {
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "mode", "cache-only").unwrap();
    parse_runtime_value(&mut patch, "default_backend", "codex-cli").unwrap();
    parse_runtime_value(&mut patch, "purposes.item_close_up.enabled", "off").unwrap();
    parse_runtime_value(&mut patch, "purposes.item_close_up.backend", "fal").unwrap();
    parse_runtime_value(&mut patch, "backends.bfl.enabled", "off").unwrap();
    parse_runtime_value(&mut patch, "backends.bfl.account", "bfl-acct").unwrap();
    parse_runtime_value(&mut patch, "backends.bfl.max_concurrent_jobs", "2").unwrap();
    parse_runtime_value(&mut patch, "transforms.enabled", "off").unwrap();
    parse_runtime_value(&mut patch, "transforms.max_decoded_bytes", "1024").unwrap();
    parse_runtime_value(&mut patch, "scheduler.job_deadline_secs", "300").unwrap();
    parse_runtime_value(&mut patch, "scheduler.stage_deadline_secs", "200").unwrap();
    parse_runtime_value(&mut patch, "request_limits.max_prompt_chars", "1000").unwrap();
    let config = apply_runtime(&ImageConfig::defaults().unwrap(), &patch).unwrap();
    assert_eq!(config.mode, GlobalMode::CacheOnly);
    assert_eq!(config.default_backend, BackendId::CodexCli);
    assert!(!config.purpose(Purpose::ItemCloseUp).enabled);
    assert_eq!(
        config.purpose(Purpose::ItemCloseUp).backend_override,
        Some(BackendId::Fal)
    );
    assert!(!config.backend(BackendId::Bfl).enabled);
    assert_eq!(config.backend(BackendId::Bfl).account.as_str(), "bfl-acct");
    assert!(!config.transforms.enabled);
    assert_eq!(config.transforms.max_decoded_bytes, 1024);
    assert_eq!(config.scheduler.job_deadline_secs, 300);
    assert_eq!(config.request_limits.max_prompt_chars, 1000);
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "purposes.item_close_up.backend", "none").unwrap();
    let config = apply_runtime(&config, &clear).unwrap();
    assert_eq!(config.purpose(Purpose::ItemCloseUp).backend_override, None);
}

#[test]
fn runtime_dotted_paths_reject_unknowns_and_bad_values() {
    let mut patch = RuntimePatch::empty();
    let error = parse_runtime_value(&mut patch, "purposes.nope.enabled", "off").unwrap_err();
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "purposes.nope.enabled".to_owned(),
        }
    );
    let error = parse_runtime_value(&mut patch, "scheduler.bogus", "1").unwrap_err();
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "scheduler.bogus".to_owned(),
        }
    );
    let error = parse_runtime_value(&mut patch, "mode", "paused").unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => assert_eq!(field, "mode"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(patch, RuntimePatch::empty());
}

#[test]
fn acknowledged_keys_are_reported_as_ignored_not_applied() {
    let file = "[pictures]\npicture_quality = \"high\"\n[prefetch]\nblah = 1\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    assert!(resolved.provenance.ignored.iter().any(|entry| entry.contains("picture_quality")));
    let resolved = resolve("", &[("DF_GAMEPLAY_IMAGERY_MODEL", "gpt-x")], &empty_patch(), 1)
        .unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    let ignored_model = resolved
        .provenance
        .ignored
        .iter()
        .any(|entry| entry.contains("DF_GAMEPLAY_IMAGERY_MODEL"));
    assert!(ignored_model);
}

#[test]
fn prefetch_costs_and_transform_profile_sections_are_ignored() {
    let file = "[imagery.prefetch]\nenabled = false\n[imagery.costs]\nprice_file = \"x\"\n\
        [imagery.transform_profiles.hero_thumbnail]\nwidth = 96\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    assert_eq!(resolved.provenance.ignored.len(), 3);
}

#[test]
fn duplicate_file_keys_are_refused() {
    let error = resolve_text_err("[imagery]\nmode = \"live\"\nmode = \"off\"\n");
    assert_eq!(
        error,
        OverlayError::DuplicateField {
            field: "imagery.mode".to_owned(),
        }
    );
}

#[test]
fn duplicate_env_names_are_refused() {
    let error = resolve(
        "",
        &[("DF_IMAGERY_MODE", "live"), ("DF_IMAGERY_MODE", "off")],
        &empty_patch(),
        1,
    )
    .unwrap_err();
    assert_eq!(
        error,
        OverlayError::DuplicateField {
            field: "DF_IMAGERY_MODE".to_owned(),
        }
    );
}

#[test]
fn larger_than_default_bounds_are_valid_without_a_code_change() {
    let file = "[imagery.scheduler]\nmax_pending_bytes = 18446744073709551615\n\
        [imagery.transforms]\nmax_decoded_bytes = 18446744073709551615\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.scheduler.max_pending_bytes, u64::MAX);
    assert_eq!(resolved.config.transforms.max_decoded_bytes, u64::MAX);
}

#[test]
fn empty_values_count_as_unset() {
    let resolved = resolve("", &[("DF_IMAGERY_MODE", "")], &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.mode, GlobalMode::Live);
    assert_eq!(
        resolved.provenance.source_of("mode"),
        Some(OverlaySource::Default)
    );
}

#[test]
fn full_example_file_parses_with_expected_winners() {
    let file = "# operator file\n[network]\ngame_port = 63309\n[imagery]\nmode = \"local-only\"\n\
        backend = \"codex-cli\"\n[imagery.backends.codex_cli]\nenabled = true\n\
        max_concurrent_jobs = 2\n[imagery.purposes.creator_portrait]\nenabled = true\n\
        [imagery.purposes.animation_frame]\nenabled = false\n[pictures]\n\
        scene_pictures = true\nscene_plates = true\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.mode, GlobalMode::LocalOnly);
    assert_eq!(resolved.config.default_backend, BackendId::CodexCli);
    assert!(!resolved.config.purpose(Purpose::AnimationFrame).enabled);
    assert!(resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert_eq!(
        resolved.provenance.source_of("default_backend"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn file_alias_conflict_still_errors_within_the_same_layer() {
    let file = "[imagery]\nbackend = \"openai\"\n[pictures]\npicture_provider = \"flux\"\n";
    let error = resolve_text_err(file);
    assert_eq!(
        error,
        OverlayError::AliasConflict {
            field: "default_backend".to_owned(),
            first: "imagery.backend".to_owned(),
            second: "pictures.picture_provider".to_owned(),
        }
    );
}

#[test]
fn legacy_master_env_off_gates_canonical_items_mural_and_expression() {
    let env = [("DF_GAMEPLAY_IMAGERY", "off")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    for purpose in [
        Purpose::CanonicalPerson,
        Purpose::CanonicalRoom,
        Purpose::ScenePlate,
        Purpose::SceneEdit,
        Purpose::Expression,
        Purpose::ItemCloseUp,
        Purpose::PartyMural,
        Purpose::CombatStill,
    ] {
        assert!(!resolved.config.purpose(purpose).enabled, "{}", purpose.label());
    }
    assert!(resolved.config.purpose(Purpose::CreatorPortrait).enabled);
    assert_eq!(
        resolved.provenance.source_of("purposes.item_close_up.enabled"),
        Some(OverlaySource::LegacyEnv("DF_GAMEPLAY_IMAGERY".to_owned()))
    );
}

#[test]
fn new_per_purpose_env_wins_over_legacy_master_off() {
    let env = [
        ("DF_GAMEPLAY_IMAGERY", "off"),
        ("DF_IMAGERY_PURPOSE_ITEM_CLOSE_UP_ENABLED", "on"),
    ];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert!(resolved.config.purpose(Purpose::ItemCloseUp).enabled);
    assert!(!resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert_eq!(
        resolved.provenance.source_of("purposes.item_close_up.enabled"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_PURPOSE_ITEM_CLOSE_UP_ENABLED".to_owned()
        ))
    );
}

#[test]
fn file_job_deadline_maps_to_scheduler_job_deadline() {
    let resolved = resolve_text("[imagery]\njob_deadline_seconds = 200\n");
    assert_eq!(resolved.config.scheduler.job_deadline_secs, 200);
    assert_eq!(
        resolved.provenance.source_of("scheduler.job_deadline_secs"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn legacy_deadline_env_maps_to_stage_deadline() {
    let env = [("DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS", "45")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.scheduler.stage_deadline_secs, 45);
    assert_eq!(
        resolved.provenance.source_of("scheduler.stage_deadline_secs"),
        Some(OverlaySource::LegacyEnv(
            "DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS".to_owned()
        ))
    );
}

#[test]
fn empty_legacy_deadline_counts_as_unset() {
    let env = [("DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS", "")];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
}

#[test]
fn generic_key_substring_does_not_trigger_secret_refusal() {
    let resolved = resolve("", &[("MONKEY_BUSINESS", "bananas")], &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
}

#[test]
fn overlong_file_keys_are_refused_before_allocation() {
    let long = "a".repeat(200);
    let file = format!("[imagery]\n{long} = true\n");
    let error = resolve_text_err(&file);
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert!(field.contains("file:imagery"), "{field}");
        }
        other => panic!("unexpected {other:?}"),
    }
}
