//! Startup composition for explicitly requested creator portraits.
use super::{
    cost::LedgerMeter,
    creator_classification::Classifier,
    creator_portraits::Portraits,
    imagery::{ImageRoute, Imagery},
    media_policy::{self, Capability, ImageProvider},
    resources::{Hub, Limits, ResourceKind},
    startup_configuration as config,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Assemble isolated portrait providers; no paid work is requested during startup.
pub(super) fn start(hub: Hub, current: &Path) -> Result<Portraits, io::Error> {
    let enabled = match media_policy::gates()
        .input(Capability::CreatorPortraits)
        .ok()
        .as_deref()
    {
        None | Some("" | "off") => false,
        Some("on") => true,
        Some(_) => {
            return Err(io::Error::other(
                "DF_GAMEPLAY_CREATOR_PORTRAITS must be on or off",
            ));
        }
    };
    let cache = std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| current.join("artifacts/tmp/play/imagery-cache"));
    let cache = if cache.is_absolute() {
        cache
    } else {
        current.join(cache)
    };
    if !enabled {
        return Ok(Portraits::new(None, None, None, cache));
    }
    let fal = std::env::var("DF_GAMEPLAY_CREATOR_FAL_KEY")
        .ok()
        .filter(|v| !v.is_empty());
    let selection =
        media_policy::image_provider(std::env::var("DF_GAMEPLAY_IMAGERY_PROVIDER"), fal.is_some())?;
    let guard = |name| match std::env::var(name) {
        Ok(value) if value == "on" => Ok(true),
        Ok(value) if value == "off" => Ok(false),
        Err(std::env::VarError::NotPresent) => Ok(false),
        _ => Err(io::Error::other(
            "Creator portrait provider guard configuration refused",
        )),
    };
    let guarded = guard("DF_GAMEPLAY_DISABLE_OPENAI_API")? | guard("DF_GAMEPLAY_CODEX_MODE")?;
    if guarded && selection == ImageProvider::OpenAi {
        return Err(io::Error::other(
            "Creator portraits require a permitted image provider",
        ));
    }
    let settings_for =
        |external_provider: bool| -> Result<config::ImageryConfiguration, io::Error> {
            let mut inputs = config::ImageryInputs::from_environment();
            inputs.credential = if external_provider {
                Err(std::env::VarError::NotPresent)
            } else {
                std::env::var("DF_GAMEPLAY_CREATOR_OPENAI_KEY")
            };
            // This is a validated independent provider, so image settings do not need an OpenAI key.
            inputs.independent_provider = external_provider;
            inputs.mode = Ok("on".into());
            inputs.quality = Ok("medium".into());
            inputs.deadline_seconds = Ok("170".into());
            let config::ImagerySettings::On(mut settings) =
                config::imagery_settings(inputs, current, config::content_rating())?
            else {
                return Err(io::Error::other(
                    "Creator portrait image provider credential unavailable",
                ));
            };
            settings.deadline = Duration::from_secs(180);
            Ok(settings)
        };
    let settings = settings_for(selection == ImageProvider::Flux)?;
    let primary = match selection {
        ImageProvider::OpenAi => ImageRoute::OpenAi(
            df_providers::NativeImageEditProvider::new(
                df_providers::ImageEditConfig::live(),
                settings
                    .credential
                    .as_ref()
                    .ok_or_else(|| io::Error::other("Creator portrait credential unavailable"))?
                    .expose(),
                settings.deadline,
            )
            .map_err(|_| io::Error::other("Creator portrait provider configuration refused"))?,
        ),
        ImageProvider::Flux => ImageRoute::Flux(
            df_providers::NativeFalImageProvider::new(
                df_providers::FalImageConfig::live(),
                fal.as_deref()
                    .ok_or_else(|| io::Error::other("Creator FLUX credential unavailable"))?,
                settings.deadline,
            )
            .map_err(|_| io::Error::other("Creator portrait provider configuration refused"))?,
        ),
    };
    let jev = config::approach_settings(
        std::env::var("DF_GAMEPLAY_CREATOR_JEV_API_KEY"),
        std::env::var("DF_GAMEPLAY_CREATOR_JEV_BASE_URL"),
    )?
    .ok_or_else(|| io::Error::other("Creator portraits require the dedicated Jev credential"))?;
    let classifier = df_providers::SystemOneChoiceProvider::new(
        jev.credential,
        &jev.base_url,
        LedgerMeter::shared(hub.ledger(), "creator_classification"),
    )
    .map_err(|_| io::Error::other("Creator Jev configuration refused"))?;
    let manifest = current
        .join("artifacts/tmp/creator-portraits")
        .join(format!("run-{}", std::process::id()));
    std::fs::create_dir_all(&manifest)?;
    let alternate = match std::env::var("DF_GAMEPLAY_CREATOR_BFL_KEY")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(key) => {
            let alt_settings = settings_for(true)?;
            let route = ImageRoute::Bfl(
                df_providers::NativeBflImageProvider::new(
                    df_providers::BflImageConfig::live(),
                    &key,
                    alt_settings.deadline,
                )
                .map_err(|_| {
                    io::Error::other("Alternate portrait provider configuration refused")
                })?,
            );
            Some(Imagery::new_in_hub(
                alt_settings,
                route,
                Some(&manifest),
                hub.clone(),
                true,
            )?)
        }
        None => None,
    };
    let imagery = Imagery::new_in_hub(settings, primary, Some(&manifest), hub.clone(), true)?;
    // new_in_hub installs general limits; a portrait operation owns exactly one paid attempt.
    hub.set_limits(
        ResourceKind::CreatorPortrait,
        Limits {
            timeout: Duration::from_secs(210),
            attempts: 1,
        },
    );
    Ok(Portraits::new(
        Some(imagery),
        alternate,
        Some(Arc::new(Classifier::from_provider(classifier))),
        cache,
    ))
}
