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
        let file = format!("[imagery.purposes.{}]\nenabled = false\n", purpose.label());
        let resolved = resolve_text(&file);
        assert!(
            !resolved.config.purpose(purpose).enabled,
            "{}",
            purpose.label()
        );
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
        assert!(
            !resolved.config.purpose(purpose).enabled,
            "{}",
            purpose.label()
        );
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
    assert_eq!(
        resolved.config.backend(BackendId::OpenAi).account.as_str(),
        "ops-main"
    );
    assert_eq!(
        resolved
            .config
            .backend(BackendId::OpenAi)
            .max_concurrent_jobs,
        5
    );
    assert!(resolved.config.backend(BackendId::Fal).enabled);
    assert!(!resolved.config.backend(BackendId::Bfl).enabled);
    assert_eq!(
        resolved
            .config
            .backend(BackendId::CodexCli)
            .account
            .as_str(),
        "clip-01"
    );
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
    let error = resolve("", &[("DF_GAMEPLAY_IMAGERY_MODLE", "x")], &empty_patch(), 1).unwrap_err();
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
    let error = resolve(
        "",
        &[("DF_IMAGERY_SCHEDULER_FOREGROUND_LIMIT", "0")],
        &empty_patch(),
        1,
    )
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
        &[(
            "DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES",
            "99999999999999999999999",
        )],
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
    let resolved = resolve_text("[imagery.backends.codex-cli]\nenabled = false\n");
    assert!(!resolved.config.backend(BackendId::CodexCli).enabled);
    assert_eq!(resolved.config.default_backend, BackendId::CodexCli);
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
        resolved
            .provenance
            .source_of("purposes.scene_plate.enabled"),
        Some(OverlaySource::FilePictures)
    );
    assert_eq!(
        resolved
            .provenance
            .source_of("purposes.canonical_person.enabled"),
        Some(OverlaySource::FilePictures)
    );
}

#[test]
fn legacy_scene_plates_gates_plates_and_edits() {
    let resolved = resolve_text("[pictures]\nscene_plates = false\n");
    assert!(!resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert!(!resolved.config.purpose(Purpose::SceneEdit).enabled);
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
        resolved
            .config
            .purpose(Purpose::CreatorPortrait)
            .backend_override,
        None
    );
    assert!(
        resolved
            .provenance
            .ignored
            .iter()
            .any(|entry| entry.contains("creator_alternate_route"))
    );
}

#[test]
fn alternate_route_does_not_conflict_with_an_explicit_override() {
    let file = "[pictures]\ncreator_alternate_route = true\n\
        [imagery.purposes.creator_portrait]\nbackend = \"openai\"\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved
            .config
            .purpose(Purpose::CreatorPortrait)
            .backend_override,
        Some(BackendId::OpenAi)
    );
    assert!(
        resolved
            .provenance
            .ignored
            .iter()
            .any(|entry| entry.contains("creator_alternate_route"))
    );
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
    assert_eq!(resolved.config.default_backend, BackendId::CodexCli);
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
    assert_eq!(
        resolved
            .config
            .backend(BackendId::CodexCli)
            .max_concurrent_jobs,
        4
    );
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
fn unsupported_owned_quality_keys_are_refused_not_ignored() {
    for file in [
        "[pictures]\npicture_quality = \"high\"\n",
        "[pictures]\npicture_profile = \"high\"\n",
        "[pictures]\nsplat_worlds = true\n",
        "[imagery]\nprofile = \"high\"\n",
        "[imagery.backends.codex_cli]\nexecutable = \"/bin/codex\"\n",
        "[imagery.purposes.scene_plate]\nquality = \"high\"\n",
        "[imagery.purposes.scene_plate]\nminimum_refresh_interval_seconds = 5\n",
    ] {
        let error = resolve_text_err(file);
        match error {
            OverlayError::InvalidField { .. } => {}
            other => panic!("unsupported owned {file:?} gave {other:?}"),
        }
    }
    for name in [
        "DF_GAMEPLAY_IMAGERY_QUALITY",
        "DF_GAMEPLAY_IMAGERY_PROFILE",
        "DF_GAMEPLAY_IMAGERY_MODEL",
        "DF_GAMEPLAY_IMAGERY_INTERVAL_SECONDS",
        "DF_GAMEPLAY_IMAGERY_CACHE",
        "DF_GAMEPLAY_PRICING",
        "DF_GAMEPLAY_CODEX_EXECUTABLE",
    ] {
        let error = resolve("", &[(name, "x")], &empty_patch(), 1).unwrap_err();
        match error {
            OverlayError::InvalidField { .. } => {}
            other => panic!("unsupported owned {name} gave {other:?}"),
        }
    }
}

#[test]
fn costs_and_transform_profile_sections_are_refused() {
    for file in [
        "[imagery.costs]\nprice_file = \"x\"\n",
        "[imagery.costs]\ntypo = true\n",
        "[imagery.transform_profiles.hero_thumbnail]\nwidth = 96\n",
        "[imagery.transform_profiles]\n",
    ] {
        let error = resolve_text_err(file);
        match error {
            OverlayError::UnknownField { .. } => {}
            other => panic!("unsupported owned {file:?} gave {other:?}"),
        }
    }
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
    let file = "[imagery.scheduler]\nmax_pending_bytes = 9223372036854775807\n\
        [imagery.transforms]\nmax_decoded_bytes = 9223372036854775807\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved.config.scheduler.max_pending_bytes,
        9_223_372_036_854_775_807
    );
    assert_eq!(
        resolved.config.transforms.max_decoded_bytes,
        9_223_372_036_854_775_807
    );
    let env = [(
        "DF_IMAGERY_SCHEDULER_MAX_PENDING_BYTES",
        "18446744073709551615",
    )];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.scheduler.max_pending_bytes, u64::MAX);
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
        assert!(
            !resolved.config.purpose(purpose).enabled,
            "{}",
            purpose.label()
        );
    }
    assert!(resolved.config.purpose(Purpose::CreatorPortrait).enabled);
    assert_eq!(
        resolved
            .provenance
            .source_of("purposes.item_close_up.enabled"),
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
        resolved
            .provenance
            .source_of("purposes.item_close_up.enabled"),
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
        resolved
            .provenance
            .source_of("scheduler.stage_deadline_secs"),
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

#[test]
fn array_table_imagery_is_rejected_not_silent_live() {
    let error = resolve_text_err("[[imagery]]\nmode = \"off\"\n");
    match error {
        OverlayError::InvalidField { field, .. } => assert_eq!(field, "imagery"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn contradicting_deadline_aliases_conflict_in_the_same_layer() {
    let file = "[imagery]\njob_deadline_seconds = 200\n\
        [imagery.scheduler]\njob_deadline_secs = 400\n";
    let error = resolve_text_err(file);
    assert_eq!(
        error,
        OverlayError::AliasConflict {
            field: "scheduler.job_deadline_secs".to_owned(),
            first: "imagery.job_deadline_seconds".to_owned(),
            second: "imagery.scheduler.job_deadline_secs".to_owned(),
        }
    );
}

#[test]
fn agreeing_deadline_aliases_merge_silently() {
    let file = "[imagery]\njob_deadline_seconds = 300\n\
        [imagery.scheduler]\njob_deadline_secs = 300\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.scheduler.job_deadline_secs, 300);
}

#[test]
fn unsupported_prefetch_is_rejected_not_ignored() {
    let error = resolve_text_err("[imagery.prefetch]\nenabled = false\n");
    assert_eq!(
        error,
        OverlayError::UnknownField {
            field: "imagery.prefetch".to_owned(),
        }
    );
}

#[test]
fn recognized_two_megabyte_env_value_is_rejected_before_clone() {
    let big = "x".repeat(2 * 1024 * 1024);
    let pairs = vec![("DF_IMAGERY_MODE", big.as_str())];
    let error = resolve("", &pairs, &empty_patch(), 1).unwrap_err();
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "DF_IMAGERY_MODE");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn file_master_off_is_not_reenabled_by_legacy_plates_env() {
    let file = "[pictures]\nscene_pictures = false\n";
    let env = [("DF_GAMEPLAY_PLATES", "on")];
    let resolved = resolve(file, &env, &empty_patch(), 1).unwrap();
    assert!(!resolved.config.purpose(Purpose::ScenePlate).enabled);
    assert!(!resolved.config.purpose(Purpose::SceneEdit).enabled);
}

#[test]
fn single_quoted_strings_and_comments_parse_as_toml() {
    let file = "# operator file\n[imagery] # section comment\n\
        mode = 'local-only' # trailing comment\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.mode, GlobalMode::LocalOnly);
}

#[test]
fn underscore_numbers_parse_as_toml() {
    let file = "[imagery.scheduler]\nqueue_age_secs = 1_000\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.scheduler.queue_age_secs, 1_000);
}

#[test]
fn float_for_integer_field_is_rejected() {
    let error = resolve_text_err("[imagery.scheduler]\nqueue_age_secs = 1.5\n");
    match error {
        OverlayError::InvalidField { field, .. } => {
            assert_eq!(field, "imagery.scheduler.queue_age_secs");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn unrelated_array_tables_are_ignored() {
    let file = "[[network]]\ngame_port = 63309\n[imagery]\nmode = \"live\"\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.mode, GlobalMode::Live);
}

#[test]
fn backend_alias_spellings_conflict_when_enabled_differs() {
    let file = "[imagery.backends.codex-cli]\nenabled = true\n\
        [imagery.backends.codex_cli]\nenabled = false\n";
    let error = resolve_text_err(file);
    match error {
        OverlayError::AliasConflict { .. } => {}
        other => panic!("alias enabled conflict gave {other:?}"),
    }
    let file = "[imagery.backends.codex-cli]\nenabled = true\n\
        [imagery.backends.codex_cli]\nenabled = true\n";
    let resolved = resolve_text(file);
    assert!(resolved.config.backend(BackendId::CodexCli).enabled);
}

#[test]
fn backend_alias_spellings_conflict_when_account_differs() {
    let file = "[imagery.backends.codex-cli]\naccount = \"first\"\n\
        [imagery.backends.codex_cli]\naccount = \"second\"\n";
    let error = resolve_text_err(file);
    match error {
        OverlayError::AliasConflict { .. } => {}
        other => panic!("alias account conflict gave {other:?}"),
    }
    let file = "[imagery.backends.codex-cli]\naccount = \"shared\"\n\
        [imagery.backends.codex_cli]\naccount = \"shared\"\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved
            .config
            .backend(BackendId::CodexCli)
            .account
            .as_str(),
        "shared"
    );
}

#[test]
fn backend_alias_spellings_conflict_when_concurrency_differs() {
    let file = "[imagery.backends.codex-cli]\nmax_concurrent_jobs = 3\n\
        [imagery.backends.codex_cli]\nmax_concurrent_jobs = 4\n";
    let error = resolve_text_err(file);
    match error {
        OverlayError::AliasConflict { .. } => {}
        other => panic!("alias concurrency conflict gave {other:?}"),
    }
    let file = "[imagery.backends.codex-cli]\nmax_concurrent_jobs = 3\n\
        [imagery.backends.codex_cli]\nmax_concurrent_jobs = 3\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved
            .config
            .backend(BackendId::CodexCli)
            .max_concurrent_jobs,
        3
    );
}

#[test]
fn unsupported_owned_wrong_types_are_refused_without_a_usable_config() {
    for file in [
        "[pictures]\npicture_quality = false\n",
        "[imagery]\nprofile = 123\n",
        "[imagery.backends.codex_cli]\nexecutable = true\n",
        "[imagery.costs]\ntypo = true\n",
    ] {
        assert!(
            resolve(file, &[], &empty_patch(), 1).is_err(),
            "unsupported owned {file:?} must not yield a config"
        );
    }
}

#[test]
fn unknown_empty_backend_and_purpose_tables_are_refused() {
    for file in ["[imagery.backends.typo]\n", "[imagery.purposes.typo]\n"] {
        assert!(
            resolve(file, &[], &empty_patch(), 1).is_err(),
            "unknown empty {file:?} must not yield defaults"
        );
    }
    for file in [
        "[imagery.backends.openai]\n",
        "[imagery.purposes.scene_plate]\n",
    ] {
        let resolved = resolve_text(file);
        assert_eq!(resolved.config, ImageConfig::defaults().unwrap());
    }
}

#[test]
fn overlong_env_names_are_refused_with_a_bounded_error() {
    let key = format!("DF_IMAGERY_{}", "x".repeat(100_000));
    let error = resolve("", &[(key.as_str(), "v")], &empty_patch(), 1).unwrap_err();
    assert!(error.to_string().len() < 1024, "{error}");
}

#[test]
fn overlong_runtime_paths_are_refused_with_a_bounded_error() {
    let mut patch = empty_patch();
    let error = parse_runtime_value(&mut patch, &"x.".repeat(100_000), "x").unwrap_err();
    assert!(error.to_string().len() < 1024, "{error}");
    assert_eq!(patch, empty_patch());
}

#[test]
fn nested_duplicate_key_preserves_its_dotted_path() {
    let error = resolve_text_err("[imagery.scheduler]\nqueue_age_secs = 1\nqueue_age_secs = 2\n");
    assert_eq!(
        error,
        OverlayError::DuplicateField {
            field: "imagery.scheduler.queue_age_secs".to_owned(),
        }
    );
}

#[test]
fn duplicate_oversized_key_falls_back_to_a_bounded_label() {
    let long = "a".repeat(10_000);
    let file = format!("[imagery]\n{long} = true\n{long} = true\n");
    let error = resolve_text_err(&file);
    assert_eq!(
        error,
        OverlayError::DuplicateField {
            field: "imagery".to_owned(),
        }
    );
    let rendered = error.to_string();
    assert!(rendered.len() < 1024, "{rendered}");
    assert!(!rendered.contains(&long));
}

#[test]
fn duplicate_table_header_stays_typed_and_bounded() {
    let error = resolve_text_err("[imagery]\nmode = \"live\"\n[imagery]\nmode = \"off\"\n");
    assert_eq!(
        error,
        OverlayError::DuplicateField {
            field: "imagery".to_owned(),
        }
    );
    assert!(error.to_string().len() < 1024);
}

#[test]
fn complete_development_example_parses_with_expected_winners() {
    let file = "[imagery]\nmode = \"live\"\ndefault_backend = \"codex-cli\"\n\
        [imagery.backends.openai]\nenabled = true\naccount = \"ops-main\"\n\
        max_concurrent_jobs = 5\nmodel = \"gpt-image-1\"\n\
        [imagery.backends.fal]\nenabled = true\n\
        endpoint = \"https://fal.example.com/v1\"\n\
        [imagery.codex]\nexecutable = \"codex\"\n\
        workdir = \"artifacts/tmp/imagery-jobs\"\ncli_profile = \"codex-images-v1\"\n\
        agent_model = \"gpt-5\"\n\
        [imagery.outputs]\ndefault_profile = \"hero-thumbnail\"\n\
        [imagery.outputs.hero-thumbnail]\nwidth_px = 96\nheight_px = 144\n\
        format = \"jpeg\"\nquality = 65\n\
        [imagery.retry]\nmax_attempts = 3\nretry_deadline_secs = 200\n\
        backoff_secs = 5\n\
        [imagery.accounting]\nmode = \"record-only\"\n\
        price_file = \"content/pricing.json\"\n\
        [imagery.quality]\njpeg_quality = 70\nprovider_quality = \"high\"\n\
        [imagery.purposes.creator_portrait]\nenabled = true\n\
        output_profile = \"hero-thumbnail\"\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.mode, GlobalMode::Live);
    assert_eq!(resolved.config.default_backend, BackendId::CodexCli);
    assert_eq!(
        resolved
            .config
            .backend(BackendId::OpenAi)
            .model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-image-1"
    );
    assert_eq!(
        resolved
            .config
            .backend(BackendId::Fal)
            .endpoint
            .as_ref()
            .unwrap()
            .as_str(),
        "https://fal.example.com/v1"
    );
    assert_eq!(resolved.config.codex.executable.as_str(), "codex");
    assert_eq!(
        resolved.config.codex.agent_model.as_ref().unwrap().as_str(),
        "gpt-5"
    );
    assert_eq!(
        resolved.config.default_output_profile.as_str(),
        "hero-thumbnail"
    );
    let selected = resolved
        .config
        .output_profile_for(Purpose::CreatorPortrait)
        .unwrap();
    let delivered = selected.delivered();
    assert_eq!(delivered.dimensions().width_px(), 96);
    assert_eq!(delivered.dimensions().height_px(), 144);
    assert_eq!(resolved.config.retry.max_attempts, 3);
    assert_eq!(resolved.config.retry.retry_deadline_secs, 200);
    assert!(resolved.config.accounting.price_file.is_some());
    assert_eq!(resolved.config.quality.jpeg_quality.value(), 70);
    assert_eq!(
        resolved.config.quality.provider_quality.unwrap(),
        crate::config::GenerationQuality::High
    );
    for field in [
        "backends.openai.model",
        "backends.fal.endpoint",
        "codex.executable",
        "codex.agent_model",
        "outputs.default_profile",
        "outputs.hero-thumbnail.width_px",
        "outputs.hero-thumbnail.quality",
        "purposes.creator_portrait.output_profile",
        "retry.max_attempts",
        "accounting.price_file",
        "quality.jpeg_quality",
        "quality.provider_quality",
    ] {
        assert_eq!(
            resolved.provenance.source_of(field),
            Some(OverlaySource::FileImagery),
            "{field}"
        );
    }
}

#[test]
fn new_env_values_cover_codex_retry_accounting_and_quality() {
    let env = [
        ("DF_IMAGERY_CODEX_EXECUTABLE", "/usr/local/bin/codex"),
        ("DF_IMAGERY_CODEX_WORKDIR", "artifacts/tmp/jobs"),
        ("DF_IMAGERY_CODEX_CLI_PROFILE", "codex-images-v2"),
        ("DF_IMAGERY_CODEX_AGENT_MODEL", "gpt-5"),
        ("DF_IMAGERY_BACKEND_OPENAI_MODEL", "gpt-image-1"),
        (
            "DF_IMAGERY_BACKEND_FAL_ENDPOINT",
            "http://127.0.0.1:8080/v1",
        ),
        ("DF_IMAGERY_RETRY_MAX_ATTEMPTS", "4"),
        ("DF_IMAGERY_RETRY_DEADLINE_SECS", "100"),
        ("DF_IMAGERY_RETRY_BACKOFF_SECS", "3"),
        ("DF_IMAGERY_ACCOUNTING_MODE", "hosted"),
        ("DF_IMAGERY_ACCOUNTING_PRICE_FILE", "content/pricing.json"),
        ("DF_IMAGERY_QUALITY_JPEG_QUALITY", "60"),
        ("DF_IMAGERY_OUTPUTS_DEFAULT_PROFILE", "default"),
        ("DF_IMAGERY_PURPOSE_SCENE_PLATE_OUTPUT_PROFILE", "default"),
    ];
    let resolved = resolve("", &env, &empty_patch(), 1).unwrap();
    assert_eq!(
        resolved.config.codex.executable.as_str(),
        "/usr/local/bin/codex"
    );
    assert_eq!(resolved.config.codex.workdir.as_str(), "artifacts/tmp/jobs");
    assert_eq!(
        resolved.config.codex.agent_model.as_ref().unwrap().as_str(),
        "gpt-5"
    );
    assert_eq!(
        resolved
            .config
            .backend(BackendId::OpenAi)
            .model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-image-1"
    );
    assert_eq!(
        resolved
            .config
            .backend(BackendId::Fal)
            .endpoint
            .as_ref()
            .unwrap()
            .as_str(),
        "http://127.0.0.1:8080/v1"
    );
    assert_eq!(resolved.config.retry.max_attempts, 4);
    assert_eq!(resolved.config.retry.retry_deadline_secs, 100);
    assert_eq!(resolved.config.quality.jpeg_quality.value(), 60);
    assert_eq!(
        resolved.provenance.source_of("codex.executable"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_CODEX_EXECUTABLE".to_owned()
        ))
    );
    assert_eq!(
        resolved.provenance.source_of("retry.max_attempts"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_RETRY_MAX_ATTEMPTS".to_owned()
        ))
    );
}

#[test]
fn new_env_profile_body_merges_over_file_defaults() {
    let file = "[imagery.outputs.hero-thumbnail]\nwidth_px = 96\nheight_px = 144\n\
        format = \"jpeg\"\nquality = 65\n";
    let env = [
        ("DF_IMAGERY_OUTPUT_HERO_THUMBNAIL_QUALITY", "75"),
        ("DF_IMAGERY_OUTPUTS_DEFAULT_PROFILE", "hero-thumbnail"),
    ];
    let resolved = resolve(file, &env, &empty_patch(), 1).unwrap();
    assert_eq!(
        resolved.config.default_output_profile.as_str(),
        "hero-thumbnail"
    );
    let selected = resolved
        .config
        .output_profile_for(Purpose::ScenePlate)
        .unwrap();
    let delivered = selected.delivered();
    assert_eq!(delivered.dimensions().width_px(), 96);
    assert_eq!(
        resolved
            .provenance
            .source_of("outputs.hero-thumbnail.quality"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_OUTPUT_HERO_THUMBNAIL_QUALITY".to_owned()
        ))
    );
    assert_eq!(
        resolved
            .provenance
            .source_of("outputs.hero-thumbnail.width_px"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn none_clears_optional_selections_across_layers() {
    let file = "[imagery.codex]\nagent_model = \"gpt-5\"\n\
        [imagery.purposes.scene_plate]\noutput_profile = \"default\"\n";
    let env = [
        ("DF_IMAGERY_CODEX_AGENT_MODEL", "none"),
        ("DF_IMAGERY_PURPOSE_SCENE_PLATE_OUTPUT_PROFILE", "none"),
        ("DF_IMAGERY_BACKEND_OPENAI_MODEL", "none"),
    ];
    let resolved = resolve(file, &env, &empty_patch(), 1).unwrap();
    assert_eq!(resolved.config.codex.agent_model, None);
    assert_eq!(
        resolved.config.purpose(Purpose::ScenePlate).output_profile,
        None
    );
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "codex.agent_model", "gpt-5").unwrap();
    parse_runtime_value(&mut patch, "purposes.scene_plate.output_profile", "none").unwrap();
    let config = apply_runtime(&resolved.config, &patch).unwrap();
    assert!(config.codex.agent_model.is_some());
    assert_eq!(config.purpose(Purpose::ScenePlate).output_profile, None);
}

#[test]
fn runtime_dotted_paths_set_the_new_scalar_groups() {
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "backends.openai.model", "gpt-image-1").unwrap();
    parse_runtime_value(
        &mut patch,
        "backends.fal.endpoint",
        "https://fal.example.com",
    )
    .unwrap();
    parse_runtime_value(&mut patch, "codex.executable", "/usr/local/bin/codex").unwrap();
    parse_runtime_value(&mut patch, "codex.workdir", "artifacts/tmp/jobs").unwrap();
    parse_runtime_value(&mut patch, "codex.cli_profile", "codex-images-v2").unwrap();
    parse_runtime_value(&mut patch, "codex.agent_model", "gpt-5").unwrap();
    parse_runtime_value(&mut patch, "outputs.default_profile", "default").unwrap();
    parse_runtime_value(&mut patch, "outputs.hero-thumbnail.width_px", "96").unwrap();
    parse_runtime_value(&mut patch, "retry.max_attempts", "3").unwrap();
    parse_runtime_value(&mut patch, "retry.retry_deadline_secs", "120").unwrap();
    parse_runtime_value(&mut patch, "retry.backoff_secs", "2").unwrap();
    parse_runtime_value(&mut patch, "accounting.mode", "record-only").unwrap();
    parse_runtime_value(&mut patch, "accounting.price_file", "content/pricing.json").unwrap();
    parse_runtime_value(&mut patch, "quality.jpeg_quality", "70").unwrap();
    parse_runtime_value(
        &mut patch,
        "purposes.scene_plate.output_profile",
        "hero-thumbnail",
    )
    .unwrap();
    let config = apply_runtime(&ImageConfig::defaults().unwrap(), &patch).unwrap();
    assert_eq!(
        config
            .backend(BackendId::OpenAi)
            .model
            .as_ref()
            .unwrap()
            .as_str(),
        "gpt-image-1"
    );
    assert_eq!(config.codex.executable.as_str(), "/usr/local/bin/codex");
    assert_eq!(config.codex.agent_model.as_ref().unwrap().as_str(), "gpt-5");
    assert_eq!(config.retry.max_attempts, 3);
    assert_eq!(config.quality.jpeg_quality.value(), 70);
    assert_eq!(
        config
            .purpose(Purpose::ScenePlate)
            .output_profile
            .as_ref()
            .unwrap()
            .as_str(),
        "hero-thumbnail"
    );
    let selected = config.output_profile_for(Purpose::ScenePlate).unwrap();
    assert_eq!(selected.delivered().dimensions().width_px(), 96);
}

#[test]
fn runtime_dotted_paths_reject_new_unknowns() {
    let mut patch = RuntimePatch::empty();
    for dotted in [
        "codex.bogus",
        "outputs.default_profile_extra",
        "outputs.hero-thumbnail.bogus",
        "retry.bogus",
        "accounting.bogus",
        "quality.bogus",
        "backends.openai.bogus",
        "purposes.scene_plate.bogus",
    ] {
        let error = parse_runtime_value(&mut patch, dotted, "1").unwrap_err();
        assert_eq!(
            error,
            OverlayError::UnknownField {
                field: dotted.to_owned(),
            },
            "{dotted}"
        );
    }
    assert_eq!(patch, RuntimePatch::empty());
}

#[test]
fn invalid_new_settings_leave_no_partial_result() {
    for file in [
        "[imagery.backends.codex_cli]\nendpoint = \"http://127.0.0.1:8080\"\n",
        "[imagery.codex]\nexecutable = \"codex; evil\"\n",
        "[imagery.codex]\nworkdir = \"jobs/../escape\"\n",
        "[imagery.outputs]\ndefault_profile = \"missing\"\n",
        "[imagery.retry]\nmax_attempts = 0\n",
        "[imagery.accounting]\nmode = \"capped\"\n",
        "[imagery.quality]\njpeg_quality = 0\n",
        "[imagery.outputs.hero-thumbnail]\nformat = \"bmp\"\n",
        "[imagery.outputs.hero-thumbnail]\nquality = 101\n",
        "[imagery.outputs.hero-thumbnail]\nmatte = \"red\"\n",
        "[imagery.outputs.hero-thumbnail]\nwidth_px = 0\n",
    ] {
        assert!(
            resolve(file, &[], &empty_patch(), 1).is_err(),
            "unsupported file {file:?} must not yield a config"
        );
    }
    for (name, value) in [
        ("DF_IMAGERY_CODEX_EXECUTABLE", "codex; evil"),
        ("DF_IMAGERY_BACKEND_FAL_ENDPOINT", "not-a-url"),
        ("DF_IMAGERY_RETRY_MAX_ATTEMPTS", "0"),
        ("DF_IMAGERY_ACCOUNTING_MODE", "capped"),
        ("DF_IMAGERY_QUALITY_JPEG_QUALITY", "101"),
        ("DF_IMAGERY_OUTPUT_HERO_WIDTH_PX", "0"),
    ] {
        assert!(
            resolve("", &[(name, value)], &empty_patch(), 1).is_err(),
            "unsupported env {name} must not yield a config"
        );
    }
    let base = ImageConfig::defaults().unwrap();
    let mut patch = RuntimePatch::empty();
    patch.default_output_profile = Some(crate::config::OutputProfileName::new("missing").unwrap());
    assert!(apply_runtime(&base, &patch).is_err());
    assert_eq!(base, ImageConfig::defaults().unwrap());
}

#[test]
fn unknown_new_sections_and_keys_are_refused() {
    for file in [
        "[imagery.codex]\nbogus = true\n",
        "[imagery.outputs]\nbogus = \"x\"\n",
        "[imagery.outputs.hero-thumbnail]\nbogus = 1\n",
        "[imagery.retry]\nbogus = 1\n",
        "[imagery.accounting]\nbogus = 1\n",
        "[imagery.quality]\nbogus = 1\n",
        "[imagery.backends.openai]\nbogus = 1\n",
        "[imagery.purposes.scene_plate]\nbogus = 1\n",
    ] {
        let error = resolve_text_err(file);
        assert!(
            matches!(error, OverlayError::UnknownField { .. }),
            "unsupported file {file:?} gave {error:?}"
        );
    }
    for name in [
        "DF_IMAGERY_CODEX_BOGUS",
        "DF_IMAGERY_RETRY_BOGUS",
        "DF_IMAGERY_ACCOUNTING_BOGUS",
        "DF_IMAGERY_QUALITY_BOGUS",
        "DF_IMAGERY_OUTPUT_HERO_BOGUS",
        "DF_IMAGERY_BACKEND_OPENAI_BOGUS",
        "DF_IMAGERY_PURPOSE_SCENE_PLATE_BOGUS",
    ] {
        let error = resolve("", &[(name, "1")], &empty_patch(), 1).unwrap_err();
        assert_eq!(
            error,
            OverlayError::UnknownField {
                field: name.to_owned(),
            },
            "{name}"
        );
    }
}

#[test]
fn matte_hex_parses_without_panics_or_echo() {
    use crate::output::AlphaHandling;

    let error = resolve_text_err("[imagery.outputs.hero-thumbnail]\nmatte = \"#ff00aa\"\n");
    match &error {
        OverlayError::InvalidField { field, .. } => {
            assert!(field.contains("matte"), "{field}");
        }
        other => panic!("matte without matte alpha gave {other:?}"),
    }
    let resolved =
        resolve_text("[imagery.outputs.hero-thumbnail]\nalpha = \"matte\"\nmatte = \"#ff00AA\"\n");
    let selected = resolved
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .map(|entry| entry.profile)
        .unwrap();
    let AlphaHandling::Matte(rgb) = selected.delivered().alpha() else {
        panic!("matte alpha was not kept");
    };
    assert_eq!((rgb.red, rgb.green, rgb.blue), (0xff, 0x00, 0xaa));
    for bad in [
        "[imagery.outputs.hero-thumbnail]\nmatte = \"red\"\n",
        "[imagery.outputs.hero-thumbnail]\nmatte = \"#fff\"\n",
        "[imagery.outputs.hero-thumbnail]\nmatte = \"#gggggg\"\n",
        "[imagery.outputs.hero-thumbnail]\nmatte = \"ff00aa\"\n",
        "[imagery.outputs.hero-thumbnail]\nmatte = \"#ff00aa \"\n",
    ] {
        let error = resolve_text_err(bad);
        match &error {
            OverlayError::InvalidField { field, .. } => {
                assert!(field.contains("matte"), "{field}");
            }
            other => panic!("matte {bad:?} gave {other:?}"),
        }
        assert!(!error.to_string().contains("ff00aa"));
    }
}

#[test]
fn profile_quality_counts_convert_without_panics() {
    for file in [
        "[imagery.outputs.hero-thumbnail]\nquality = 101\n",
        "[imagery.outputs.hero-thumbnail]\nquality = 0\n",
        "[imagery.outputs.hero-thumbnail]\nquality = 4294967296\n",
        "[imagery.outputs.hero-thumbnail]\nquality = \"high\"\n",
        "[imagery.quality]\nprovider_quality = 101\n",
        "[imagery.quality]\nprovider_quality = \"unrecognized\"\n",
    ] {
        assert!(
            resolve(file, &[], &empty_patch(), 1).is_err(),
            "quality {file:?} must not yield a config"
        );
    }
    let error = resolve(
        "",
        &[(
            "DF_IMAGERY_OUTPUT_HERO_THUMBNAIL_QUALITY",
            "18446744073709551615",
        )],
        &empty_patch(),
        1,
    )
    .unwrap_err();
    assert!(matches!(error, OverlayError::InvalidField { .. }));
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "outputs.hero-thumbnail.quality", "101").unwrap();
    let config = ImageConfig::defaults().unwrap();
    assert!(apply_runtime(&config, &patch).is_err());
}

#[test]
fn lossless_webp_is_explicit_while_lossy_keeps_its_quality() {
    use crate::output::Encoding;

    let resolved =
        resolve_text("[imagery.outputs.hero-thumbnail]\nformat = \"webp\"\nquality = \"none\"\n");
    let selected = resolved
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .map(|entry| entry.profile)
        .unwrap();
    assert!(matches!(
        selected.delivered().encoding(),
        Encoding::WebP { quality: None }
    ));
    assert!(selected.delivered().encoding().is_lossless());
    let resolved =
        resolve_text("[imagery.outputs.hero-thumbnail]\nformat = \"webp\"\nquality = 70\n");
    let selected = resolved
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .map(|entry| entry.profile)
        .unwrap();
    assert!(matches!(
        selected.delivered().encoding(),
        Encoding::WebP { quality: Some(_) }
    ));
    assert!(!selected.delivered().encoding().is_lossless());
    let error =
        resolve_text_err("[imagery.outputs.hero-thumbnail]\nformat = \"png\"\nquality = 70\n");
    assert!(matches!(error, OverlayError::InvalidField { .. }));
}

#[test]
fn partial_profile_overrides_keep_unmentioned_fields() {
    let resolved = resolve_text("[imagery.outputs.hero-thumbnail]\nwidth_px = 96\n");
    let selected = resolved
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .map(|entry| entry.profile)
        .unwrap();
    let delivered = selected.delivered();
    assert_eq!(delivered.dimensions().width_px(), 96);
    assert_eq!(delivered.dimensions().height_px(), 1024);
    let generated = selected.generated().unwrap();
    assert_eq!(generated.width_px(), 1024);
    assert_eq!(
        resolved
            .provenance
            .source_of("outputs.hero-thumbnail.width_px"),
        Some(OverlaySource::FileImagery)
    );
    assert_eq!(
        resolved.provenance.source_of("outputs.hero-thumbnail"),
        None
    );
}

#[test]
fn generated_none_converts_a_provider_profile_to_local() {
    use crate::output::OutputProfile;

    let resolved = resolve_text(
        "[imagery.outputs.hero-thumbnail]\ngenerated_width_px = \"none\"\n\
        generated_height_px = \"none\"\nwidth_px = 48\nheight_px = 64\nformat = \"png\"\n",
    );
    let selected = resolved
        .config
        .output_profiles
        .iter()
        .find(|entry| entry.name.as_str() == "hero-thumbnail")
        .map(|entry| entry.profile)
        .unwrap();
    assert!(matches!(selected, OutputProfile::Local { .. }));
    let error = resolve_text_err(
        "[imagery.outputs.hero-thumbnail]\ngenerated_width_px = \"none\"\nwidth_px = 48\n",
    );
    assert!(matches!(error, OverlayError::InvalidField { .. }));
}

#[test]
fn absolute_workdir_and_spaced_executable_resolve() {
    let file = "[imagery.codex]\nworkdir = \"/var/lib/dungeonflux/jobs\"\n\
        executable = \"/Applications/My Tools/codex's-bin/codex\"\n";
    let resolved = resolve_text(file);
    assert_eq!(
        resolved.config.codex.workdir.as_str(),
        "/var/lib/dungeonflux/jobs"
    );
    assert_eq!(
        resolved.config.codex.executable.as_str(),
        "/Applications/My Tools/codex's-bin/codex"
    );
    assert_eq!(
        resolved.provenance.source_of("codex.workdir"),
        Some(OverlaySource::FileImagery)
    );
}

#[test]
fn malformed_endpoints_are_refused_across_layers() {
    for file in [
        "[imagery.backends.fal]\nendpoint = \"https://user@example.com\"\n",
        "[imagery.backends.fal]\nendpoint = \"http://127.0.0.1:8080/v1?token=x\"\n",
        "[imagery.backends.fal]\nendpoint = \"http://127.0.0.1:8080/v1#frag\"\n",
        "[imagery.backends.fal]\nendpoint = \"http://host:99999/v1\"\n",
    ] {
        assert!(
            resolve(file, &[], &empty_patch(), 1).is_err(),
            "endpoint {file:?} must not yield a config"
        );
    }
    let error = resolve(
        "",
        &[(
            "DF_IMAGERY_BACKEND_FAL_ENDPOINT",
            "http://127.0.0.1:1/v1?x=1",
        )],
        &empty_patch(),
        1,
    )
    .unwrap_err();
    assert!(matches!(error, OverlayError::InvalidField { .. }));
    let mut patch = RuntimePatch::empty();
    let error = parse_runtime_value(&mut patch, "backends.fal.endpoint", "https://x@example.com")
        .unwrap_err();
    assert!(matches!(error, OverlayError::InvalidField { .. }));
    assert_eq!(patch, RuntimePatch::empty());
}

#[test]
fn more_than_sixteen_named_profiles_resolve() {
    let mut file = String::from("[imagery.outputs]\ndefault_profile = \"extra-00\"\n");
    for index in 0..20 {
        file.push_str(&format!(
            "[imagery.outputs.extra-{index:02}]\nwidth_px = 32\nheight_px = 32\nformat = \"png\"\n"
        ));
    }
    let resolved = resolve_text(&file);
    assert_eq!(resolved.config.output_profiles.len(), 21);
    assert_eq!(resolved.config.default_output_profile.as_str(), "extra-00");
}

#[test]
fn provider_quality_round_trips_across_layers_with_clear() {
    let resolved = resolve_text("[imagery.quality]\nprovider_quality = \"high\"\n");
    assert_eq!(
        resolved.config.quality.provider_quality.unwrap(),
        crate::config::GenerationQuality::High
    );
    assert_eq!(
        resolved.provenance.source_of("quality.provider_quality"),
        Some(OverlaySource::FileImagery)
    );
    let env = [("DF_IMAGERY_QUALITY_PROVIDER_QUALITY", "none")];
    let resolved = resolve(
        "[imagery.quality]\nprovider_quality = \"high\"\n",
        &env,
        &empty_patch(),
        1,
    )
    .unwrap();
    assert_eq!(resolved.config.quality.provider_quality, None);
    assert_eq!(
        resolved.provenance.source_of("quality.provider_quality"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_QUALITY_PROVIDER_QUALITY".to_owned()
        ))
    );
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "quality.provider_quality", "medium").unwrap();
    let config = apply_runtime(&resolved.config, &patch).unwrap();
    assert_eq!(
        config.quality.provider_quality.unwrap(),
        crate::config::GenerationQuality::Medium
    );
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(&mut clear, "quality.provider_quality", "none").unwrap();
    let config = apply_runtime(&config, &clear).unwrap();
    assert_eq!(config.quality.provider_quality, None);
}

#[test]
fn short_job_deadlines_resolve_with_an_effective_retry_budget() {
    let resolved =
        resolve_text("[imagery.scheduler]\njob_deadline_secs = 60\nstage_deadline_secs = 45\n");
    assert_eq!(resolved.config.scheduler.job_deadline_secs, 60);
    assert_eq!(resolved.config.effective_retry_deadline_secs(), 60);
    let file = "[imagery.scheduler]\njob_deadline_secs = 300\nstage_deadline_secs = 200\n\
        [imagery.retry]\nretry_deadline_secs = 200\n";
    let resolved = resolve_text(file);
    assert_eq!(resolved.config.effective_retry_deadline_secs(), 200);
}

#[test]
fn generation_quality_named_variants_resolve_across_every_overlay() {
    use crate::config::GenerationQuality;
    for (name, expected) in [
        ("low", GenerationQuality::Low),
        ("medium", GenerationQuality::Medium),
        ("high", GenerationQuality::High),
        ("auto", GenerationQuality::Auto),
    ] {
        let text = format!("[imagery.quality]\nprovider_quality = \"{name}\"\n");
        assert_eq!(
            resolve_text(&text).config.quality.provider_quality,
            Some(expected)
        );
        let env = [("DF_IMAGERY_QUALITY_PROVIDER_QUALITY", name)];
        assert_eq!(
            resolve("", &env, &empty_patch(), 1)
                .unwrap()
                .config
                .quality
                .provider_quality,
            Some(expected)
        );
        let mut patch = empty_patch();
        parse_runtime_value(&mut patch, "quality.provider_quality", name).unwrap();
        assert_eq!(
            apply_runtime(&ImageConfig::defaults().unwrap(), &patch)
                .unwrap()
                .quality
                .provider_quality,
            Some(expected)
        );
    }
    let mut patch = empty_patch();
    assert!(parse_runtime_value(&mut patch, "quality.provider_quality", "90").is_err());
    assert_eq!(patch, empty_patch());
}

#[test]
fn operation_models_resolve_independently_with_per_field_precedence() {
    let text = "[imagery.backends.fal]\nmodel = \"fallback\"\ngeneration_model = \"generation\"\nedit_model = \"edit\"\nalpha_model = \"alpha\"\n";
    let env = [("DF_IMAGERY_BACKEND_FAL_EDIT_MODEL", "environment-edit")];
    let mut patch = empty_patch();
    parse_runtime_value(&mut patch, "backends.fal.alpha_model", "runtime-alpha").unwrap();
    let resolved = resolve(text, &env, &patch, 1).unwrap();
    let backend = resolved.config.backend(BackendId::Fal);
    assert_eq!(backend.model.as_ref().unwrap().as_str(), "fallback");
    assert_eq!(
        backend.generation_model.as_ref().unwrap().as_str(),
        "generation"
    );
    assert_eq!(
        backend.edit_model.as_ref().unwrap().as_str(),
        "environment-edit"
    );
    assert_eq!(
        backend.alpha_model.as_ref().unwrap().as_str(),
        "runtime-alpha"
    );
    assert_eq!(
        resolved
            .provenance
            .source_of("backends.fal.generation_model"),
        Some(OverlaySource::FileImagery)
    );
    assert_eq!(
        resolved.provenance.source_of("backends.fal.edit_model"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_BACKEND_FAL_EDIT_MODEL".to_owned()
        ))
    );
    assert_eq!(
        resolved.provenance.source_of("backends.fal.alpha_model"),
        Some(OverlaySource::Runtime)
    );
    let mut clear = empty_patch();
    parse_runtime_value(&mut clear, "backends.fal.alpha_model", "none").unwrap();
    assert_eq!(
        apply_runtime(&resolved.config, &clear)
            .unwrap()
            .backend(BackendId::Fal)
            .alpha_model,
        None
    );
}

#[test]
fn codex_stream_bounds_follow_file_environment_and_runtime_precedence() {
    // Stdout must hold one message line (default 24 MiB), so every layer keeps stdout above it.
    let file = "[imagery.codex]
max_stdout_bytes = 33554432
max_stderr_bytes = 524288
";
    let from_file = resolve_text(file);
    assert_eq!(from_file.config.codex.max_stdout_bytes, 33554432);
    assert_eq!(from_file.config.codex.max_stderr_bytes, 524288);
    let env = [
        ("DF_IMAGERY_CODEX_MAX_STDOUT_BYTES", "50331648"),
        ("DF_IMAGERY_CODEX_MAX_STDERR_BYTES", "1048576"),
    ];
    let from_env = resolve(file, &env, &empty_patch(), 1).unwrap();
    assert_eq!(from_env.config.codex.max_stdout_bytes, 50331648);
    assert_eq!(from_env.config.codex.max_stderr_bytes, 1048576);
    let mut patch = empty_patch();
    patch.codex_max_stdout_bytes = Some(67108864);
    patch.codex_max_stderr_bytes = Some(2097152);
    let runtime = resolve(file, &env, &patch, 2).unwrap();
    assert_eq!(runtime.config.codex.max_stdout_bytes, 67108864);
    assert_eq!(runtime.config.codex.max_stderr_bytes, 2097152);
    for field in ["codex.max_stdout_bytes", "codex.max_stderr_bytes"] {
        assert_eq!(
            runtime.provenance.source_of(field),
            Some(OverlaySource::Runtime)
        );
    }
}

#[test]
fn zero_codex_stream_bounds_are_rejected_at_each_configuration_boundary() {
    for (field, environment) in [
        ("max_stdout_bytes", "DF_IMAGERY_CODEX_MAX_STDOUT_BYTES"),
        ("max_stderr_bytes", "DF_IMAGERY_CODEX_MAX_STDERR_BYTES"),
        ("max_message_bytes", "DF_IMAGERY_CODEX_MAX_MESSAGE_BYTES"),
    ] {
        let file = format!("[imagery.codex]\n{field} = 0\n");
        assert!(resolve(&file, &[], &empty_patch(), 1).is_err());
        assert!(resolve("", &[(environment, "0")], &empty_patch(), 1).is_err());
        let mut patch = empty_patch();
        parse_runtime_value(&mut patch, &format!("codex.{field}"), "0").unwrap();
        assert!(resolve("", &[], &patch, 1).is_err());
    }
}

#[test]
fn codex_message_limit_tracks_every_overlay_without_changing_stream_bounds() {
    let defaults = resolve_text("");
    assert_eq!(defaults.config.codex.max_message_bytes, 25_165_824);
    assert_eq!(
        defaults.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::Default)
    );
    let text = "[imagery.codex]
max_message_bytes = 25165824
max_stdout_bytes = 33554432
max_stderr_bytes = 524288
";
    let from_file = resolve_text(text);
    assert_eq!(from_file.config.codex.max_message_bytes, 25_165_824);
    assert_eq!(
        from_file.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::FileImagery)
    );
    let env = [("DF_IMAGERY_CODEX_MAX_MESSAGE_BYTES", "31457280")];
    let from_env = resolve(text, &env, &empty_patch(), 1).unwrap();
    assert_eq!(from_env.config.codex.max_message_bytes, 31_457_280);
    assert_eq!(
        from_env.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_CODEX_MAX_MESSAGE_BYTES".to_owned()
        ))
    );
    let mut patch = empty_patch();
    parse_runtime_value(&mut patch, "codex.max_message_bytes", "33554432").unwrap();
    let runtime = resolve(text, &env, &patch, 2).unwrap();
    assert_eq!(runtime.config.codex.max_message_bytes, 33_554_432);
    assert_eq!(runtime.config.codex.max_stdout_bytes, 33_554_432);
    assert_eq!(runtime.config.codex.max_stderr_bytes, 524_288);
    assert_eq!(
        runtime.provenance.source_of("codex.max_message_bytes"),
        Some(OverlaySource::Runtime)
    );
    for invalid in ["-1", "none", "18446744073709551616"] {
        let prior = patch.clone();
        assert!(parse_runtime_value(&mut patch, "codex.max_message_bytes", invalid).is_err());
        assert_eq!(patch, prior);
    }
}

#[test]
fn trusted_analysis_routes_resolve_file_environment_and_runtime_fields() {
    for stage in ["classification", "inspection"] {
        let text = format!(
            "[imagery.analysis.{stage}]\naccount = \"file-analysis\"\nmodel = \"custom/file-model\"\nendpoint = \"http://analysis.operator.test:8080/v1/responses\"\nmaximum_output_tokens = 1024\n"
        );
        let file = resolve_text(&text);
        let field = format!("analysis.{stage}");
        let configured = if stage == "classification" {
            file.config.analysis.classification
        } else {
            file.config.analysis.inspection
        }
        .unwrap();
        assert_eq!(configured.account.as_str(), "file-analysis");
        assert_eq!(configured.model.as_str(), "custom/file-model");
        assert_eq!(configured.maximum_output_tokens, 1024);
        assert_eq!(
            file.provenance.source_of(&format!("{field}.endpoint")),
            Some(OverlaySource::FileImagery)
        );
        let name = format!("DF_IMAGERY_ANALYSIS_{}_ACCOUNT", stage.to_ascii_uppercase());
        let env = [(name.as_str(), "environment-analysis")];
        let mut patch = RuntimePatch::empty();
        parse_runtime_value(
            &mut patch,
            &format!("{field}.model"),
            "custom/runtime-model",
        )
        .unwrap();
        let resolved = resolve(&text, &env, &patch, 1).unwrap();
        let configured = if stage == "classification" {
            resolved.config.analysis.classification
        } else {
            resolved.config.analysis.inspection
        }
        .unwrap();
        assert_eq!(configured.account.as_str(), "environment-analysis");
        assert_eq!(configured.model.as_str(), "custom/runtime-model");
        assert_eq!(
            configured.endpoint.as_str(),
            "http://analysis.operator.test:8080/v1/responses"
        );
        assert_eq!(
            resolved.provenance.source_of(&format!("{field}.account")),
            Some(OverlaySource::NewEnv(name))
        );
        assert_eq!(
            resolved.provenance.source_of(&format!("{field}.model")),
            Some(OverlaySource::Runtime)
        );
    }
}

#[test]
fn complete_analysis_routes_can_be_configured_by_environment_alone_and_cleared() {
    let env = [
        (
            "DF_IMAGERY_ANALYSIS_CLASSIFICATION_ACCOUNT",
            "operator-analysis",
        ),
        ("DF_IMAGERY_ANALYSIS_CLASSIFICATION_MODEL", "custom/model"),
        (
            "DF_IMAGERY_ANALYSIS_CLASSIFICATION_ENDPOINT",
            "http://localhost:9080/responses",
        ),
        (
            "DF_IMAGERY_ANALYSIS_CLASSIFICATION_MAXIMUM_OUTPUT_TOKENS",
            "4096",
        ),
    ];
    let resolved = resolve("", &env, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        resolved
            .config
            .analysis
            .classification
            .as_ref()
            .unwrap()
            .maximum_output_tokens,
        4096
    );
    let text = "[imagery.analysis.inspection]\naccount = \"operator-analysis\"\nmodel = \"custom/model\"\nendpoint = \"http://localhost:9080/responses\"\nmaximum_output_tokens = 4096\n";
    let clear = [("DF_IMAGERY_ANALYSIS_INSPECTION", "none")];
    let resolved = resolve(text, &clear, &RuntimePatch::empty(), 1).unwrap();
    assert!(resolved.config.analysis.inspection.is_none());
    assert_eq!(
        resolved
            .provenance
            .source_of("analysis.inspection.endpoint"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_ANALYSIS_INSPECTION".to_owned()
        ))
    );
    let disabled =
        resolve_text("[imagery.analysis]\nclassification = \"none\"\ninspection = \"none\"\n");
    assert!(disabled.config.analysis.classification.is_none());
    assert!(disabled.config.analysis.inspection.is_none());
}

#[test]
fn analysis_route_errors_are_atomic_and_redacted() {
    for text in [
        "[imagery.analysis.inspection]\naccount = \"operator\"\n",
        "[imagery.analysis.inspection]\naccount = 12\n",
        "[imagery.analysis.unknown]\n",
        "[imagery.analysis.inspection]\ncredential = \"private-test-value\"\n",
        "[imagery.analysis.inspection]\naccount = \"operator\"\nmodel = \"custom/model\"\nendpoint = \"http://localhost:9080/responses\"\nmaximum_output_tokens = 0\n",
    ] {
        let error = resolve(text, &[], &RuntimePatch::empty(), 1).unwrap_err();
        assert!(!format!("{error:?}").contains("private-test-value"));
    }
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "analysis.inspection.model", "custom/model").unwrap();
    let prior = patch.clone();
    for (path, value) in [
        (
            "analysis.inspection.endpoint",
            "http://user:private-test-value@localhost:9080",
        ),
        ("analysis.inspection.maximum_output_tokens", "-1"),
        ("analysis.unknown.model", "custom/model"),
        ("analysis.inspection.api_key", "private-test-value"),
    ] {
        let error = parse_runtime_value(&mut patch, path, value).unwrap_err();
        assert!(!format!("{error:?}").contains("private-test-value"));
        assert_eq!(patch, prior);
    }
    assert!(apply_runtime(&ImageConfig::defaults().unwrap(), &patch).is_err());
}

#[test]
fn purpose_generation_overrides_follow_all_input_layers_and_clear_independently() {
    use crate::config::GenerationQuality;
    let text = "[imagery]\nbackend = \"fal\"\n[imagery.purposes.animation_frame]\nbackend = \"bfl\"\nquality_override = \"medium\"\nmodel_override = \"custom/file-frame\"\n";
    let file = resolve_text(text);
    let frame = file.config.purpose(Purpose::AnimationFrame);
    assert_eq!(frame.quality_override, Some(GenerationQuality::Medium));
    assert_eq!(
        frame.model_override.as_ref().unwrap().as_str(),
        "custom/file-frame"
    );
    assert_eq!(
        file.provenance
            .source_of("purposes.animation_frame.model_override"),
        Some(OverlaySource::FileImagery)
    );
    let env = [
        (
            "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_QUALITY_OVERRIDE",
            "high",
        ),
        (
            "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_MODEL_OVERRIDE",
            "custom/environment-frame",
        ),
    ];
    let environment = resolve(text, &env, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        environment
            .config
            .purpose(Purpose::AnimationFrame)
            .quality_override,
        Some(GenerationQuality::High)
    );
    assert_eq!(
        environment
            .provenance
            .source_of("purposes.animation_frame.model_override"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_MODEL_OVERRIDE".to_owned()
        ))
    );
    let mut runtime = RuntimePatch::empty();
    parse_runtime_value(
        &mut runtime,
        "purposes.animation_frame.quality_override",
        "low",
    )
    .unwrap();
    parse_runtime_value(
        &mut runtime,
        "purposes.animation_frame.model_override",
        "custom/runtime-frame",
    )
    .unwrap();
    let resolved = resolve(text, &env, &runtime, 1).unwrap();
    let frame = resolved.config.purpose(Purpose::AnimationFrame);
    assert_eq!(frame.quality_override, Some(GenerationQuality::Low));
    assert_eq!(
        frame.model_override.as_ref().unwrap().as_str(),
        "custom/runtime-frame"
    );
    assert_eq!(frame.backend_override, Some(BackendId::Bfl));
    assert_eq!(resolved.config.default_backend, BackendId::Fal);
    assert_eq!(
        resolved
            .provenance
            .source_of("purposes.animation_frame.quality_override"),
        Some(OverlaySource::Runtime)
    );
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.quality_override",
        "none",
    )
    .unwrap();
    let cleared = apply_runtime(&resolved.config, &clear).unwrap();
    assert_eq!(
        cleared.purpose(Purpose::AnimationFrame).quality_override,
        None
    );
    assert_eq!(
        cleared.purpose(Purpose::AnimationFrame).model_override,
        frame.model_override
    );
    let none = [
        (
            "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_QUALITY_OVERRIDE",
            "none",
        ),
        ("DF_IMAGERY_PURPOSE_ANIMATION_FRAME_MODEL_OVERRIDE", "none"),
    ];
    let cleared = resolve(text, &none, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .quality_override,
        None
    );
    assert_eq!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .model_override,
        None
    );
    let file_clear = resolve_text(
        "[imagery.purposes.animation_frame]\nquality_override = \"none\"\nmodel_override = \"none\"\n",
    );
    assert_eq!(
        file_clear
            .config
            .purpose(Purpose::AnimationFrame)
            .quality_override,
        None
    );
    assert_eq!(
        file_clear
            .config
            .purpose(Purpose::AnimationFrame)
            .model_override,
        None
    );
}

#[test]
fn invalid_purpose_generation_overrides_do_not_mutate_the_patch() {
    let mut patch = RuntimePatch::empty();
    for (path, value) in [
        ("purposes.animation_frame.quality_override", "90"),
        (
            "purposes.animation_frame.model_override",
            "model with whitespace",
        ),
        ("purposes.unknown.model_override", "custom/model"),
    ] {
        assert!(parse_runtime_value(&mut patch, path, value).is_err());
        assert_eq!(patch, RuntimePatch::empty());
    }
    assert!(
        resolve(
            "[imagery.purposes.animation_frame]\nquality_override = 90\n",
            &[],
            &patch,
            1
        )
        .is_err()
    );
}

#[test]
fn moderation_override_resolves_every_layer_and_clears_without_changing_quality() {
    use crate::request::ProviderModeration;
    let text = "[imagery.purposes.animation_frame]\nquality_override = \"medium\"\nmoderation_override = \"low\"\n";
    let file = resolve_text(text);
    assert_eq!(
        file.config
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        Some(ProviderModeration::Low)
    );
    assert_eq!(
        file.provenance
            .source_of("purposes.animation_frame.moderation_override"),
        Some(OverlaySource::FileImagery)
    );
    let env = [(
        "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_MODERATION_OVERRIDE",
        "auto",
    )];
    let environment = resolve(text, &env, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        environment
            .config
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        Some(ProviderModeration::Auto)
    );
    assert_eq!(
        environment
            .provenance
            .source_of("purposes.animation_frame.moderation_override"),
        Some(OverlaySource::NewEnv(env[0].0.to_owned()))
    );
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(
        &mut patch,
        "purposes.animation_frame.moderation_override",
        "low",
    )
    .unwrap();
    let runtime = resolve(text, &env, &patch, 1).unwrap();
    assert_eq!(
        runtime
            .config
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        Some(ProviderModeration::Low)
    );
    assert_eq!(
        runtime
            .provenance
            .source_of("purposes.animation_frame.moderation_override"),
        Some(OverlaySource::Runtime)
    );
    parse_runtime_value(
        &mut patch,
        "purposes.animation_frame.moderation_override",
        "none",
    )
    .unwrap();
    let cleared = resolve(text, &env, &patch, 1).unwrap();
    assert_eq!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .moderation_override,
        None
    );
    assert_eq!(
        cleared
            .config
            .purpose(Purpose::AnimationFrame)
            .quality_override,
        Some(crate::config::GenerationQuality::Medium)
    );
    let empty = RuntimePatch::empty();
    let mut invalid = empty.clone();
    assert!(
        parse_runtime_value(
            &mut invalid,
            "purposes.animation_frame.moderation_override",
            "mature"
        )
        .is_err()
    );
    assert_eq!(invalid, empty);
}

#[test]
fn analysis_preparation_bounds_follow_each_input_layer_with_field_provenance() {
    let file = "[imagery.analysis.preparation]\nmax_pixels = 40000000\nmax_picture_bytes = 2097152\nmax_request_bytes = 33554432\n";
    let configured = resolve_text(file);
    assert_eq!(
        configured.config.analysis.preparation.max_pixels,
        40_000_000
    );
    assert_eq!(
        configured.config.analysis.preparation.max_picture_bytes,
        2_097_152
    );
    assert_eq!(
        configured.config.analysis.preparation.max_request_bytes,
        33_554_432
    );
    assert_eq!(
        configured
            .provenance
            .source_of("analysis.preparation.max_pixels"),
        Some(OverlaySource::FileImagery)
    );
    let env = [
        ("DF_IMAGERY_ANALYSIS_PREPARATION_MAX_PIXELS", "80000000"),
        (
            "DF_IMAGERY_ANALYSIS_PREPARATION_MAX_PICTURE_BYTES",
            "4194304",
        ),
        (
            "DF_IMAGERY_ANALYSIS_PREPARATION_MAX_REQUEST_BYTES",
            "67108864",
        ),
    ];
    let configured = resolve(file, &env, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        configured.config.analysis.preparation.max_pixels,
        80_000_000
    );
    assert_eq!(
        configured.config.analysis.preparation.max_picture_bytes,
        4_194_304
    );
    assert_eq!(
        configured.config.analysis.preparation.max_request_bytes,
        67_108_864
    );
    assert_eq!(
        configured
            .provenance
            .source_of("analysis.preparation.max_picture_bytes"),
        Some(OverlaySource::NewEnv(env[1].0.to_owned()))
    );
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "analysis.preparation.max_pixels", "160000000").unwrap();
    parse_runtime_value(
        &mut patch,
        "analysis.preparation.max_picture_bytes",
        "8388608",
    )
    .unwrap();
    parse_runtime_value(
        &mut patch,
        "analysis.preparation.max_request_bytes",
        "134217728",
    )
    .unwrap();
    let configured = resolve(file, &env, &patch, 1).unwrap();
    assert_eq!(
        configured.config.analysis.preparation.max_pixels,
        160_000_000
    );
    assert_eq!(
        configured.config.analysis.preparation.max_picture_bytes,
        8_388_608
    );
    assert_eq!(
        configured.config.analysis.preparation.max_request_bytes,
        134_217_728
    );
    assert_eq!(
        configured
            .provenance
            .source_of("analysis.preparation.max_request_bytes"),
        Some(OverlaySource::Runtime)
    );
    for (field, name) in [
        ("max_pixels", env[0].0),
        ("max_picture_bytes", env[1].0),
        ("max_request_bytes", env[2].0),
    ] {
        assert!(
            resolve(
                &format!("[imagery.analysis.preparation]\n{field} = 0\n"),
                &[],
                &RuntimePatch::empty(),
                1
            )
            .is_err()
        );
        assert!(resolve("", &[(name, "0")], &RuntimePatch::empty(), 1).is_err());
        let mut invalid = RuntimePatch::empty();
        parse_runtime_value(&mut invalid, &format!("analysis.preparation.{field}"), "0").unwrap();
        assert!(resolve("", &[], &invalid, 1).is_err());
    }
}

#[test]
fn standalone_analysis_routes_follow_file_environment_and_runtime_controls() {
    for stage in ["head_locations", "staging_identity", "staging_facing"] {
        let file = format!(
            "[imagery.analysis.{stage}]\naccount = \"private-analysis\"\nmodel = \"file-model\"\nendpoint = \"https://analysis.example/v1\"\nmaximum_output_tokens = 2048\n"
        );
        let variable = format!("DF_IMAGERY_ANALYSIS_{}_MODEL", stage.to_ascii_uppercase());
        let mut patch = RuntimePatch::empty();
        parse_runtime_value(
            &mut patch,
            &format!("analysis.{stage}.maximum_output_tokens"),
            "4096",
        )
        .unwrap();
        let effective = resolve(&file, &[(&variable, "env-model")], &patch, 1).unwrap();
        let route = match stage {
            "head_locations" => effective.config.analysis.head_locations.as_ref(),
            "staging_identity" => effective.config.analysis.staging_identity.as_ref(),
            _ => effective.config.analysis.staging_facing.as_ref(),
        }
        .unwrap();
        assert_eq!(route.account.as_str(), "private-analysis");
        assert_eq!(route.model.as_str(), "env-model");
        assert_eq!(route.maximum_output_tokens, 4096);
        assert_eq!(
            effective
                .provenance
                .source_of(&format!("analysis.{stage}.model")),
            Some(OverlaySource::NewEnv(variable))
        );
        assert_eq!(
            effective
                .provenance
                .source_of(&format!("analysis.{stage}.maximum_output_tokens")),
            Some(OverlaySource::Runtime)
        );
    }
}

#[test]
fn segmentation_preserves_fal_route_and_separate_operator_preparation_bounds() {
    use crate::config::SegmentationProvider;
    let file = concat!(
        "[imagery.analysis.segmentation]\nprovider = \"fal\"\naccount = \"sam-private\"\n",
        "model = \"fal-ai/sam-3-1/image-rle\"\nendpoint = \"https://queue.fal.run\"\n",
        "[imagery.analysis.segmentation_preparation]\nmax_pixels = 40000000\nmax_picture_bytes = 6291456\nmax_request_bytes = 12582912\n",
    );
    let env = [
        (
            "DF_IMAGERY_ANALYSIS_SEGMENTATION_MODEL",
            "private/sam-image-rle",
        ),
        (
            "DF_IMAGERY_ANALYSIS_SEGMENTATION_PREPARATION_MAX_REQUEST_BYTES",
            "33554432",
        ),
    ];
    let effective = resolve(file, &env, &RuntimePatch::empty(), 1).unwrap();
    let route = effective.config.analysis.segmentation.as_ref().unwrap();
    assert_eq!(route.provider, SegmentationProvider::Fal);
    assert_eq!(route.model.as_str(), env[0].1);
    assert_eq!(route.account.as_str(), "sam-private");
    assert_eq!(
        effective
            .config
            .analysis
            .segmentation_preparation
            .max_pixels,
        40000000
    );
    assert_eq!(
        effective
            .config
            .analysis
            .segmentation_preparation
            .max_picture_bytes,
        6291456
    );
    assert_eq!(
        effective
            .config
            .analysis
            .segmentation_preparation
            .max_request_bytes,
        33554432
    );
    assert_eq!(
        effective.config.analysis.preparation.max_picture_bytes,
        1048576
    );
    assert_eq!(
        effective
            .provenance
            .source_of("analysis.segmentation.provider"),
        Some(OverlaySource::FileImagery)
    );
    let clear = [("DF_IMAGERY_ANALYSIS_SEGMENTATION", "none")];
    assert!(
        resolve(file, &clear, &RuntimePatch::empty(), 1)
            .unwrap()
            .config
            .analysis
            .segmentation
            .is_none()
    );
    for field in ["max_pixels", "max_picture_bytes", "max_request_bytes"] {
        let mut invalid = RuntimePatch::empty();
        parse_runtime_value(
            &mut invalid,
            &format!("analysis.segmentation_preparation.{field}"),
            "0",
        )
        .unwrap();
        assert!(resolve(file, &[], &invalid, 1).is_err());
    }
    assert!(resolve_text(file).provenance.ignored.is_empty());
    assert!(
        resolve(
            &file.replace("provider = \"fal\"", "provider = \"openai\""),
            &[],
            &RuntimePatch::empty(),
            1
        )
        .is_err()
    );
}

#[test]
fn purpose_accounts_remain_distinct_from_backend_and_unrelated_purposes() {
    let file = "[imagery.backends.openai]\naccount = \"general-openai\"\n[imagery.purposes.animation_frame]\naccount_override = \"creator-openai\"\n";
    let env = [(
        "DF_IMAGERY_PURPOSE_ANIMATION_FRAME_ACCOUNT_OVERRIDE",
        "operator-creator",
    )];
    let effective = resolve(file, &env, &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        effective.config.backend(BackendId::OpenAi).account.as_str(),
        "general-openai"
    );
    assert_eq!(
        effective
            .config
            .purpose(Purpose::AnimationFrame)
            .account_override
            .as_ref()
            .unwrap()
            .as_str(),
        "operator-creator"
    );
    assert!(
        effective
            .config
            .purpose(Purpose::ScenePlate)
            .account_override
            .is_none()
    );
    let mut clear = RuntimePatch::empty();
    parse_runtime_value(
        &mut clear,
        "purposes.animation_frame.account_override",
        "none",
    )
    .unwrap();
    let effective = resolve(file, &env, &clear, 1).unwrap();
    assert!(
        effective
            .config
            .purpose(Purpose::AnimationFrame)
            .account_override
            .is_none()
    );
    assert_eq!(
        effective.config.backend(BackendId::OpenAi).account.as_str(),
        "general-openai"
    );
}

#[test]
fn quota_file_environment_and_runtime_layers_preserve_each_winning_source() {
    let file = "[imagery.quota]\nenabled = false\ncooldown_ms = 12345\nmaximum_accounts = 17\n";
    let env = [
        ("DF_IMAGERY_QUOTA_ENABLED", "true"),
        ("DF_IMAGERY_QUOTA_COOLDOWN_MS", "54321"),
    ];
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "quota.maximum_accounts", "31").unwrap();
    let resolved = resolve(file, &env, &patch, 7).unwrap();
    assert!(resolved.config.quota.enabled);
    assert_eq!(resolved.config.quota.cooldown_ms, 54_321);
    assert_eq!(resolved.config.quota.maximum_accounts, 31);
    assert_eq!(
        resolved.provenance.source_of("quota.enabled"),
        Some(OverlaySource::NewEnv("DF_IMAGERY_QUOTA_ENABLED".to_owned()))
    );
    assert_eq!(
        resolved.provenance.source_of("quota.cooldown_ms"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_QUOTA_COOLDOWN_MS".to_owned()
        ))
    );
    assert_eq!(
        resolved.provenance.source_of("quota.maximum_accounts"),
        Some(OverlaySource::Runtime)
    );
    let file_only = resolve(file, &[], &RuntimePatch::empty(), 1).unwrap();
    assert!(!file_only.config.quota.enabled);
    assert_eq!(file_only.config.quota.cooldown_ms, 12_345);
    assert_eq!(
        file_only.provenance.source_of("quota.maximum_accounts"),
        Some(OverlaySource::FileImagery)
    );
    let defaults = resolve("", &[], &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        defaults.config.quota,
        crate::config::QuotaSettings {
            enabled: true,
            cooldown_ms: 900_000,
            maximum_accounts: 1_024
        }
    );
    for field in [
        "quota.enabled",
        "quota.cooldown_ms",
        "quota.maximum_accounts",
    ] {
        assert_eq!(
            defaults.provenance.source_of(field),
            Some(OverlaySource::Default)
        );
    }
}

#[test]
fn invalid_quota_limits_are_refused_even_when_the_breaker_is_disabled() {
    for file in [
        "[imagery.quota]\nenabled=false\ncooldown_ms=0\n",
        "[imagery.quota]\nenabled=false\nmaximum_accounts=0\n",
        "[imagery.quota]\nmaximum_accounts=4294967296\n",
        "[imagery.quota]\ncooldown_ms=-1\n",
        "[imagery.quota]\nenabled=\"sometimes\"\n",
    ] {
        assert!(
            resolve(file, &[], &RuntimePatch::empty(), 1).is_err(),
            "{file}"
        );
    }
    for pair in [
        ("DF_IMAGERY_QUOTA_COOLDOWN_MS", "0"),
        ("DF_IMAGERY_QUOTA_MAXIMUM_ACCOUNTS", "4294967296"),
        ("DF_IMAGERY_QUOTA_ENABLED", "sometimes"),
    ] {
        assert!(resolve("", &[pair], &RuntimePatch::empty(), 1).is_err());
    }
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "quota.cooldown_ms", "18446744073709551615").unwrap();
    parse_runtime_value(&mut patch, "quota.maximum_accounts", "4294967295").unwrap();
    let resolved = resolve("", &[], &patch, 1).unwrap();
    assert_eq!(resolved.config.quota.cooldown_ms, u64::MAX);
    assert_eq!(resolved.config.quota.maximum_accounts, u32::MAX);
    assert!(parse_runtime_value(&mut patch, "quota.unknown", "1").is_err());
}

#[test]
fn creator_alternate_account_layers_can_explicitly_clear_dedicated_defaults() {
    use crate::config::AccountHandle;
    use crate::config_overlay::resolve_with_defaults;
    let mut defaults = ImageConfig::defaults().unwrap();
    defaults.creator_bfl_account = Some(AccountHandle::new("dedicated-bfl").unwrap());
    defaults.purposes[Purpose::CreatorPortrait.index()].account_override =
        Some(AccountHandle::new("primary-openai").unwrap());
    defaults.backends[BackendId::Bfl.index()].account = AccountHandle::new("general-bfl").unwrap();
    let seeded =
        resolve_with_defaults(defaults.clone(), "", &[], &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        seeded.provenance.source_of("creator.bfl_account"),
        Some(OverlaySource::Default)
    );
    let file = "[imagery.creator]\nbfl_account=\"file-bfl\"\n";
    let file_only =
        resolve_with_defaults(defaults.clone(), file, &[], &RuntimePatch::empty(), 1).unwrap();
    assert_eq!(
        file_only
            .config
            .creator_bfl_account
            .as_ref()
            .unwrap()
            .as_str(),
        "file-bfl"
    );
    assert_eq!(
        file_only.provenance.source_of("creator.bfl_account"),
        Some(OverlaySource::FileImagery)
    );
    let env = [("DF_IMAGERY_CREATOR_BFL_ACCOUNT", "none")];
    let cleared =
        resolve_with_defaults(defaults.clone(), file, &env, &RuntimePatch::empty(), 1).unwrap();
    assert!(cleared.config.creator_bfl_account.is_none());
    assert_eq!(
        cleared.provenance.source_of("creator.bfl_account"),
        Some(OverlaySource::NewEnv(
            "DF_IMAGERY_CREATOR_BFL_ACCOUNT".to_owned()
        ))
    );
    let mut patch = RuntimePatch::empty();
    parse_runtime_value(&mut patch, "creator.bfl_account", "runtime-bfl").unwrap();
    let runtime = resolve_with_defaults(defaults.clone(), file, &env, &patch, 1).unwrap();
    assert_eq!(
        runtime
            .config
            .creator_bfl_account
            .as_ref()
            .unwrap()
            .as_str(),
        "runtime-bfl"
    );
    assert_eq!(
        runtime.provenance.source_of("creator.bfl_account"),
        Some(OverlaySource::Runtime)
    );
    parse_runtime_value(&mut patch, "creator.bfl_account", "none").unwrap();
    let explicit_none = resolve_with_defaults(defaults, file, &[], &patch, 1).unwrap();
    assert!(explicit_none.config.creator_bfl_account.is_none());
    assert_eq!(
        explicit_none
            .config
            .purpose(Purpose::CreatorPortrait)
            .account_override
            .as_ref()
            .unwrap()
            .as_str(),
        "primary-openai"
    );
    assert_eq!(
        explicit_none
            .config
            .backend(BackendId::Bfl)
            .account
            .as_str(),
        "general-bfl"
    );
    assert!(
        resolve(
            "[imagery.creator]\nbfl_account=\"bad/account\"\n",
            &[],
            &RuntimePatch::empty(),
            1
        )
        .is_err()
    );
    assert!(
        resolve(
            "",
            &[("DF_IMAGERY_CREATOR_BFL_ACCOUNT", "bad/account")],
            &RuntimePatch::empty(),
            1
        )
        .is_err()
    );
    assert!(
        resolve(
            "[imagery.creator]\nbfl_account=\"none\"\n",
            &[],
            &RuntimePatch::empty(),
            1
        )
        .unwrap()
        .config
        .creator_bfl_account
        .is_none()
    );
}
