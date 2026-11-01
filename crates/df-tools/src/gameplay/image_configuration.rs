//! One live configuration owner, with legacy domain defaults applied only below explicit choices.

use super::{
    imagery::{ImageKind, OutputSpec},
    startup_configuration::{ImageryProfile, ImagerySettings},
};
use df_imagery::{
    AlphaHandling, BackendId, ConfigControl, Dimensions, Encoding, GenerationQuality, GlobalMode,
    ImageConfig, ModelName, OutputProfile, OutputProfileName, OverlaySource, Purpose,
    RasterOutputSpec, RuntimePatch,
};
use std::{
    io,
    sync::{Arc, Mutex},
};

pub(super) fn control(
    legacy: &ImagerySettings,
    qualification: bool,
) -> Result<Arc<Mutex<ConfigControl>>, io::Error> {
    let config_text = match std::env::var("DF_IMAGERY_CONFIG") {
        Ok(path) if !path.is_empty() => {
            let metadata = std::fs::metadata(&path)
                .map_err(|_| invalid("imagery configuration file unavailable"))?;
            if metadata.len() > 1_048_576 {
                return Err(invalid("imagery configuration file too large"));
            }
            std::fs::read_to_string(path)
                .map_err(|_| invalid("imagery configuration file unreadable"))?
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => String::new(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                df_imagery::OverlayError::InvalidField {
                    field: "DF_IMAGERY_CONFIG".to_owned(),
                    reason: "environment value is not valid Unicode",
                },
            ));
        }
    };
    let pairs = collect_overlay_environment(std::env::vars_os())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let env = pairs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let defaults = legacy_defaults(legacy, qualification)?;
    let mut control =
        ConfigControl::build_with_defaults(defaults, &config_text, &env, &RuntimePatch::empty(), 1)
            .map_err(|_| invalid("imagery configuration refused"))?;
    normalize(&mut control)?;
    Ok(Arc::new(Mutex::new(control)))
}

/// Inspects names before values; unrelated process state and host-owned settings stay outside the overlay.
fn collect_overlay_environment(
    pairs: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Result<Vec<(String, String)>, df_imagery::OverlayError> {
    let mut selected = Vec::new();
    for (name, value) in pairs {
        let Some(name_text) = name.to_str() else {
            let bytes = name.as_encoded_bytes();
            if bytes.starts_with(b"DF_IMAGERY_") || bytes.starts_with(b"DF_GAMEPLAY_IMAGERY") {
                return Err(df_imagery::OverlayError::InvalidField {
                    field: "imagery.environment".to_owned(),
                    reason: "environment name is not valid Unicode",
                });
            }
            continue;
        };
        let owned = name_text.starts_with("DF_IMAGERY_")
            || name_text.starts_with("DF_GAMEPLAY_IMAGERY")
            || matches!(
                name_text,
                "DF_GAMEPLAY_PLATES"
                    | "DF_GAMEPLAY_CREATOR_PORTRAITS"
                    | "DF_GAMEPLAY_CHARACTER_VIDEO"
                    | "DF_GAMEPLAY_DISABLE_OPENAI_API"
            );
        if !owned || super::image_startup::is_host_setting(name_text) {
            continue;
        }
        let field = name_text.to_owned();
        let value = value
            .into_string()
            .map_err(|_| df_imagery::OverlayError::InvalidField {
                field: field.clone(),
                reason: "environment value is not valid Unicode",
            })?;
        selected.push((field, value));
    }
    Ok(selected)
}

#[cfg(test)]
mod environment_tests;

/// Domain defaults precede file/environment/runtime layers, so a partial route/profile edit
/// merges over its complete historical contract and an explicit None remains a real choice.
fn profile_defaults(
    legacy: &ImagerySettings,
    qualification: bool,
    off_profile: ImageryProfile,
) -> Result<ImageConfig, io::Error> {
    let mut config = ImageConfig::defaults().map_err(|_| invalid("image defaults invalid"))?;
    if qualification {
        config.mode = GlobalMode::Off;
    } else if matches!(legacy, ImagerySettings::Off(_)) {
        config.mode = GlobalMode::LocalOnly;
    }
    config.purposes[Purpose::CreatorPortrait.index()].enabled = false;
    config.transforms.max_output_bytes = 6 * 1024 * 1024;
    // Legacy independent foreground/presence/background pools admitted 3 + 3 + 6 image jobs.
    config.backends[BackendId::OpenAi.index()].max_concurrent_jobs = 12;
    config.backends[BackendId::Fal.index()].max_concurrent_jobs = 12;
    config.backends[BackendId::Bfl.index()].max_concurrent_jobs = 12;
    if let ImagerySettings::On(settings) = legacy {
        config.scheduler.stage_deadline_secs = settings.deadline.as_secs();
    }
    let (profile, quality, model) = match legacy {
        ImagerySettings::On(settings) => (
            settings.profile,
            settings.quality,
            settings.model.model_id(),
        ),
        ImagerySettings::Off(_) => (
            off_profile,
            df_providers::ImageQuality::Medium,
            "gpt-image-2.5-flare",
        ),
    };
    for (purpose, kind, delivered) in [
        (
            Purpose::CanonicalPerson,
            ImageKind::CanonicalCharacter,
            OutputSpec::CUTOUT,
        ),
        (
            Purpose::CanonicalRoom,
            ImageKind::CanonicalRoom,
            OutputSpec::BACKDROP,
        ),
        (
            Purpose::ScenePlate,
            ImageKind::ScenePlate,
            OutputSpec::PLATE,
        ),
        (
            Purpose::SceneEdit,
            ImageKind::ScenePlateEdit,
            OutputSpec::PLATE,
        ),
        (
            Purpose::Expression,
            ImageKind::CastExpression,
            OutputSpec::CUTOUT,
        ),
        (
            Purpose::ItemCloseUp,
            ImageKind::ArtifactCloseUp,
            OutputSpec::CLOSE_UP,
        ),
        (
            Purpose::PartyMural,
            ImageKind::ScenePlate,
            OutputSpec::PLATE,
        ),
        (
            Purpose::CombatStill,
            ImageKind::ScenePlate,
            OutputSpec::PLATE,
        ),
        (
            Purpose::CreatorPortrait,
            ImageKind::CanonicalCharacter,
            OutputSpec {
                width: 1024,
                height: 1536,
                transparent: false,
            },
        ),
    ] {
        let (quality, generated) = super::imagery::drawn_for(profile, quality, kind, delivered);
        // Profile names keep to lowercase letters, digits and hyphens; purpose labels use underscores.
        let name =
            OutputProfileName::new(&format!("gameplay-{}", purpose.label().replace('_', "-")))
                .map_err(|_| invalid("gameplay profile name invalid"))?;
        let body = provider_profile(generated, delivered, &config)?;
        config
            .output_profiles
            .push(df_imagery::NamedOutputProfile::new(name.clone(), body));
        let gate = &mut config.purposes[purpose.index()];
        gate.output_profile = Some(name);
        gate.quality_override = Some(quality_of(quality));
        if purpose != Purpose::CreatorPortrait {
            gate.model_override =
                Some(ModelName::new(model).map_err(|_| invalid("gameplay image model invalid"))?);
        }
    }
    let frame = Purpose::AnimationFrame.index();
    let name = OutputProfileName::new("gameplay-animation-frame")
        .map_err(|_| invalid("frame profile invalid"))?;
    let dimensions =
        Dimensions::new(1024, 1536).map_err(|_| invalid("frame dimensions invalid"))?;
    let delivered = RasterOutputSpec::new(dimensions, AlphaHandling::PreserveAlpha, Encoding::Png)
        .map_err(|_| invalid("frame output invalid"))?;
    config
        .output_profiles
        .push(df_imagery::NamedOutputProfile::new(
            name.clone(),
            OutputProfile::provider(dimensions, delivered),
        ));
    let gate = &mut config.purposes[frame];
    gate.backend_override = Some(BackendId::OpenAi);
    gate.model_override =
        Some(ModelName::new("gpt-image-2.5-sunburst").map_err(|_| invalid("frame model invalid"))?);
    gate.quality_override = Some(match profile {
        ImageryProfile::Dev => GenerationQuality::Medium,
        ImageryProfile::Production => GenerationQuality::High,
    });
    gate.moderation_override = Some(df_imagery::ProviderModeration::Low);
    gate.output_profile = Some(name);
    config
        .validate()
        .map_err(|_| invalid("gameplay profile defaults invalid"))?;
    Ok(config)
}

pub(super) fn legacy_profile_defaults(
    legacy: &ImagerySettings,
    qualification: bool,
) -> Result<ImageConfig, io::Error> {
    profile_defaults(legacy, qualification, ImageryProfile::default())
}

pub(super) fn legacy_defaults(
    legacy: &ImagerySettings,
    qualification: bool,
) -> Result<ImageConfig, io::Error> {
    let mut config = if matches!(legacy, ImagerySettings::Off(_)) {
        profile_defaults(legacy, qualification, ImageryProfile::from_environment()?)?
    } else {
        legacy_profile_defaults(legacy, qualification)?
    };
    if std::env::var("DF_GAMEPLAY_CREATOR_OPENAI_KEY").is_ok_and(|key| !key.is_empty()) {
        config.purposes[Purpose::CreatorPortrait.index()].account_override = Some(
            df_imagery::AccountHandle::new("creator-openai")
                .map_err(|_| invalid("creator account invalid"))?,
        );
    }
    if std::env::var("DF_GAMEPLAY_CREATOR_BFL_KEY").is_ok_and(|key| !key.is_empty()) {
        config.creator_bfl_account = Some(
            df_imagery::AccountHandle::new("creator-bfl")
                .map_err(|_| invalid("creator BFL account invalid"))?,
        );
    }
    if let Some(jev) = super::startup_configuration::approach_settings(
        std::env::var("DF_GAMEPLAY_CREATOR_JEV_API_KEY"),
        std::env::var("DF_GAMEPLAY_CREATOR_JEV_BASE_URL"),
    )? {
        config.analysis.classification = Some(df_imagery::AnalysisRoute {
            account: df_imagery::AccountHandle::new("creator-jev")
                .map_err(|_| invalid("creator analysis account invalid"))?,
            model: ModelName::new("jev-latest").map_err(|_| invalid("creator model invalid"))?,
            endpoint: df_imagery::ApiEndpoint::new(&format!(
                "{}/v1/systemone",
                jev.base_url.trim_end_matches('/')
            ))
            .map_err(|_| invalid("creator endpoint invalid"))?,
            maximum_output_tokens: 1,
        });
    }
    if std::env::var("OPENAI_API_KEY").is_ok_and(|key| !key.is_empty())
        && std::env::var("DF_GAMEPLAY_DISABLE_OPENAI_API").as_deref() != Ok("on")
    {
        let route = df_imagery::AnalysisRoute {
            account: df_imagery::AccountHandle::new("gameplay-openai-vision")
                .map_err(|_| invalid("vision account invalid"))?,
            model: ModelName::new("gpt-5.5").map_err(|_| invalid("vision model invalid"))?,
            endpoint: df_imagery::ApiEndpoint::new("https://api.openai.com/v1/chat/completions")
                .map_err(|_| invalid("vision endpoint invalid"))?,
            maximum_output_tokens: 3000,
        };
        config.analysis.inspection = Some(route.clone());
        let mut heads = route.clone();
        heads.maximum_output_tokens = 2000;
        config.analysis.head_locations = Some(heads);
        config.analysis.staging_identity = Some(route.clone());
        config.analysis.staging_facing = Some(route);
    }
    if std::env::var("FAL_KEY")
        .or_else(|_| std::env::var("DF_FAL_KEY"))
        .is_ok_and(|key| !key.is_empty())
    {
        config.analysis.segmentation = Some(df_imagery::SegmentationRoute {
            provider: df_imagery::SegmentationProvider::Fal,
            account: df_imagery::AccountHandle::new("gameplay-fal-segmentation")
                .map_err(|_| invalid("segmentation account invalid"))?,
            model: ModelName::new("fal-ai/sam-3-1/image-rle")
                .map_err(|_| invalid("segmentation model invalid"))?,
            endpoint: df_imagery::ApiEndpoint::new("https://queue.fal.run")
                .map_err(|_| invalid("segmentation endpoint invalid"))?,
        });
    }
    config
        .validate()
        .map_err(|_| invalid("gameplay image defaults invalid"))?;
    Ok(config)
}

/// An explicit global or backend choice supersedes an inherited domain default. This runs on
/// the same ConfigControl before each capture; the acknowledged revision contains the change.
pub(super) fn normalize(control: &mut ConfigControl) -> Result<(), io::Error> {
    let resolved = control.read_effective();
    let is_default =
        |field: &str| resolved.provenance.source_of(field) == Some(OverlaySource::Default);
    let global_profile = !is_default("outputs.default_profile")
        || resolved.provenance.fields.iter().any(|field| {
            field.field.starts_with(&format!(
                "outputs.{}.",
                resolved.config.default_output_profile.as_str()
            )) && field.source != OverlaySource::Default
        });
    let mut patch = RuntimePatch::empty();
    let mut changed = false;
    for purpose in (0..Purpose::COUNT).filter_map(Purpose::from_index) {
        let prefix = format!("purposes.{}", purpose.label());
        let gate = resolved.config.purpose(purpose);
        let clear_backend = gate.backend_override.is_some()
            && is_default(&format!("{prefix}.backend_override"))
            && !is_default("default_backend");
        if clear_backend {
            patch.purpose_backend[purpose.index()] = Some(None);
            changed = true;
        }
        let backend = if clear_backend {
            resolved.config.default_backend
        } else {
            gate.backend_override
                .unwrap_or(resolved.config.default_backend)
        };
        let backend_model_explicit = ["model", "generation_model", "edit_model", "alpha_model"]
            .iter()
            .any(|field| !is_default(&format!("backends.{}.{}", backend.label(), field)));
        if gate.model_override.is_some()
            && is_default(&format!("{prefix}.model_override"))
            && (backend != BackendId::OpenAi || backend_model_explicit)
        {
            patch.purpose_model_override[purpose.index()] = Some(None);
            changed = true;
        }
        if gate.account_override.is_some()
            && is_default(&format!("{prefix}.account_override"))
            && backend != BackendId::OpenAi
        {
            patch.purpose_account_override[purpose.index()] = Some(None);
            changed = true;
        }
        if gate.output_profile.is_some()
            && global_profile
            && is_default(&format!("{prefix}.output_profile"))
        {
            patch.purpose_output_profile[purpose.index()] = Some(None);
            changed = true;
        }
        if gate.quality_override.is_some()
            && !is_default("quality.provider_quality")
            && is_default(&format!("{prefix}.quality_override"))
        {
            patch.purpose_quality_override[purpose.index()] = Some(None);
            changed = true;
        }
    }
    if changed {
        control
            .apply(patch, resolved.provenance.revision)
            .map_err(|_| invalid("image default normalization refused"))?;
    }
    Ok(())
}

fn provider_profile(
    generated: OutputSpec,
    delivered: OutputSpec,
    config: &ImageConfig,
) -> Result<OutputProfile, io::Error> {
    let generated = Dimensions::new(generated.width, generated.height)
        .map_err(|_| invalid("generated dimensions invalid"))?;
    let dimensions = Dimensions::new(delivered.width, delivered.height)
        .map_err(|_| invalid("delivered dimensions invalid"))?;
    let (alpha, encoding) = if delivered.transparent {
        (
            AlphaHandling::PreserveAlpha,
            Encoding::WebP { quality: None },
        )
    } else {
        (
            AlphaHandling::Opaque,
            Encoding::Jpeg {
                quality: config.quality.jpeg_quality,
            },
        )
    };
    let delivered = RasterOutputSpec::new(dimensions, alpha, encoding)
        .map_err(|_| invalid("gameplay output invalid"))?;
    Ok(OutputProfile::provider(generated, delivered))
}
fn quality_of(quality: df_providers::ImageQuality) -> GenerationQuality {
    match quality {
        df_providers::ImageQuality::Low => GenerationQuality::Low,
        df_providers::ImageQuality::Medium => GenerationQuality::Medium,
        df_providers::ImageQuality::High => GenerationQuality::High,
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::other(message)
}

pub(super) fn snapshot(
    control: &Arc<Mutex<ConfigControl>>,
) -> Result<df_imagery::ResolvedConfig, io::Error> {
    let mut control = control
        .lock()
        .map_err(|_| invalid("image config unavailable"))?;
    normalize(&mut control)?;
    Ok(control.read_effective())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_defaults() -> ImageConfig {
        let mut config = ImageConfig::defaults().unwrap();
        let frame = &mut config.purposes[Purpose::AnimationFrame.index()];
        frame.backend_override = Some(BackendId::OpenAi);
        frame.model_override = Some(ModelName::new("gpt-image-2.5-sunburst").unwrap());
        frame.account_override =
            Some(df_imagery::AccountHandle::new("legacy-frame-account").unwrap());
        config
    }

    #[test]
    fn parsed_legacy_deadline_seeds_the_actual_config_before_operator_layers() {
        use super::super::startup_configuration::{ImageryInputs, imagery_settings};
        use std::env::VarError::NotPresent;
        let legacy = imagery_settings(
            ImageryInputs {
                credential: Ok("fixture-credential".to_owned()),
                independent_provider: false,
                mode: Ok("on".to_owned()),
                model: Err(NotPresent),
                quality: Err(NotPresent),
                profile: Err(NotPresent),
                interval_seconds: Err(NotPresent),
                deadline_seconds: Ok("45".to_owned()),
                cache: Err(NotPresent),
            },
            std::path::Path::new("."),
        )
        .unwrap();
        let defaults = legacy_profile_defaults(&legacy, false).unwrap();
        assert_eq!(defaults.scheduler.stage_deadline_secs, 45);
        let control = ConfigControl::build_with_defaults(
            defaults.clone(),
            "",
            &[("DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS", "45")],
            &RuntimePatch::empty(),
            1,
        )
        .unwrap();
        let resolved = control.read_effective();
        assert_eq!(resolved.config.scheduler.stage_deadline_secs, 45);
        assert_eq!(
            resolved
                .provenance
                .source_of("scheduler.stage_deadline_secs"),
            Some(OverlaySource::LegacyEnv(
                "DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS".to_owned()
            ))
        );
        let control = ConfigControl::build_with_defaults(
            defaults,
            "[imagery.scheduler]\nstage_deadline_secs=70\n",
            &[
                ("DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS", "45"),
                ("DF_IMAGERY_SCHEDULER_STAGE_DEADLINE_SECS", "80"),
            ],
            &RuntimePatch::empty(),
            1,
        )
        .unwrap();
        let resolved = control.read_effective();
        assert_eq!(resolved.config.scheduler.stage_deadline_secs, 80);
        assert_eq!(
            resolved
                .provenance
                .source_of("scheduler.stage_deadline_secs"),
            Some(OverlaySource::NewEnv(
                "DF_IMAGERY_SCHEDULER_STAGE_DEADLINE_SECS".to_owned()
            ))
        );
    }

    #[test]
    fn legacy_limits_are_default_provenance_and_operator_limits_replace_them() {
        let defaults = legacy_profile_defaults(
            &ImagerySettings::Off(
                crate::gameplay::startup_configuration::ImageryOff::Qualification,
            ),
            false,
        )
        .unwrap();
        assert_eq!(defaults.transforms.max_output_bytes, 6 * 1024 * 1024);
        assert_eq!(defaults.backend(BackendId::OpenAi).max_concurrent_jobs, 12);
        let mut patch = RuntimePatch::empty();
        patch.transforms_max_output_bytes = Some(12 * 1024 * 1024);
        let control = ConfigControl::build_with_defaults(defaults, "", &[], &patch, 1).unwrap();
        assert_eq!(
            control.read_effective().config.transforms.max_output_bytes,
            12 * 1024 * 1024
        );
    }

    #[test]
    fn live_backend_change_clears_only_inherited_openai_model_and_account() {
        let mut control = ConfigControl::build_with_defaults(
            frame_defaults(),
            "",
            &[],
            &RuntimePatch::empty(),
            1,
        )
        .unwrap();
        let mut patch = RuntimePatch::empty();
        patch.default_backend = Some(BackendId::CodexCli);
        control.apply(patch, 1).unwrap();
        normalize(&mut control).unwrap();
        let effective = control.read_effective();
        let frame = effective.config.purpose(Purpose::AnimationFrame);
        assert_eq!(effective.config.default_backend, BackendId::CodexCli);
        assert_eq!(frame.backend_override, None);
        assert_eq!(frame.model_override, None);
        assert_eq!(frame.account_override, None);
        assert_eq!(effective.provenance.revision, 3);
        normalize(&mut control).unwrap();
        assert_eq!(control.revision(), 3);
    }

    #[test]
    fn explicit_purpose_model_and_account_survive_backend_default_normalization() {
        let mut patch = RuntimePatch::empty();
        patch.default_backend = Some(BackendId::CodexCli);
        patch.purpose_backend[Purpose::AnimationFrame.index()] = Some(Some(BackendId::CodexCli));
        patch.purpose_model_override[Purpose::AnimationFrame.index()] =
            Some(Some(ModelName::new("operator-model").unwrap()));
        patch.purpose_account_override[Purpose::AnimationFrame.index()] = Some(Some(
            df_imagery::AccountHandle::new("operator-account").unwrap(),
        ));
        let mut control =
            ConfigControl::build_with_defaults(frame_defaults(), "", &[], &patch, 1).unwrap();
        normalize(&mut control).unwrap();
        let effective = control.read_effective();
        let frame = effective.config.purpose(Purpose::AnimationFrame);
        assert_eq!(frame.backend_override, Some(BackendId::CodexCli));
        assert_eq!(
            frame.model_override.as_ref().unwrap().as_str(),
            "operator-model"
        );
        assert_eq!(
            frame.account_override.as_ref().unwrap().as_str(),
            "operator-account"
        );
        assert_eq!(control.revision(), 1);
    }

    #[test]
    fn explicit_global_profile_choice_replaces_inherited_domain_profile_without_touching_explicit_purpose()
     {
        let mut defaults = frame_defaults();
        let name = OutputProfileName::new("domain-frame").unwrap();
        defaults
            .output_profiles
            .push(df_imagery::NamedOutputProfile::new(
                name.clone(),
                defaults.output_profiles.first().unwrap().profile.clone(),
            ));
        defaults.purposes[Purpose::AnimationFrame.index()].output_profile = Some(name.clone());
        defaults.purposes[Purpose::CanonicalPerson.index()].output_profile = Some(name.clone());
        let mut patch = RuntimePatch::empty();
        patch.default_output_profile = Some(OutputProfileName::new("default").unwrap());
        patch.purpose_output_profile[Purpose::CanonicalPerson.index()] = Some(Some(name.clone()));
        let mut control = ConfigControl::build_with_defaults(defaults, "", &[], &patch, 1).unwrap();
        normalize(&mut control).unwrap();
        let effective = control.read_effective();
        assert_eq!(
            effective
                .config
                .purpose(Purpose::AnimationFrame)
                .output_profile,
            None
        );
        assert_eq!(
            effective
                .config
                .purpose(Purpose::CanonicalPerson)
                .output_profile,
            Some(name)
        );
    }
}
