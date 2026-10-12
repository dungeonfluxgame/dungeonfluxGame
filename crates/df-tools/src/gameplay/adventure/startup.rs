//! Local-demo startup adapter; all random selection remains in the pure generator.
use super::super::{
    model, pack::LoadedPack, startup_configuration::AdventureSettings, storyteller::Storyteller,
};
use super::{
    ForcedEntries, GenerateError, GenerateOptions, GeneratedAdventure, Tables, TablesError,
    expression, generate,
    record::{self, AdventureRecord, ModelExpression, RecordError},
};
use df_persistence::local_demo_scope::{self, AdventureStartup, DemoAdventure};
use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) struct PreparedAdventure {
    pub(crate) directory: PathBuf,
    pub(crate) loaded: LoadedPack,
    pub(crate) documents: DemoAdventure,
    pub(crate) record: AdventureRecord,
    pub(crate) fresh: bool,
    pub(crate) campaign: Option<super::campaign_startup::CampaignRun>,
    /// The startup summary of how the story text was written.
    pub(crate) prose: String,
}
/// A freshly rolled adventure and how its text was written.
struct FreshAdventure {
    generated: GeneratedAdventure,
    model: Option<ModelExpression>,
    prose: String,
}
#[derive(Debug)]
pub(crate) enum StartupError {
    Database(local_demo_scope::DemoInitializationError),
    Connection,
    Driver,
    Seed,
    Tables(TablesError),
    Generate(GenerateError),
    Record(RecordError),
    DatabaseName,
    Campaign(super::campaign_startup::CampaignStartupError),
}
impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(
                f,
                "adventure startup database refused at {}: {:?}",
                e.stage, e.class
            ),
            Self::Connection => {
                f.write_str("adventure database connection unavailable or deadline expired")
            }
            Self::Driver => f.write_str("adventure database driver did not close"),
            Self::Seed => f.write_str("OS adventure seed unavailable"),
            Self::Tables(e) => e.fmt(f),
            Self::Generate(e) => e.fmt(f),
            Self::Record(e) => e.fmt(f),
            Self::Campaign(error) => error.fmt(f),
            Self::DatabaseName => f.write_str("adventure run database identity unavailable"),
        }
    }
}
impl std::error::Error for StartupError {}
pub(crate) async fn prepare(
    settings: &AdventureSettings,
    configuration: &tokio_postgres::Config,
    current: &Path,
    storyteller: &Storyteller,
) -> Result<PreparedAdventure, StartupError> {
    let database = configuration
        .get_dbname()
        .ok_or(StartupError::DatabaseName)?;
    let (client, connection) = tokio::time::timeout(
        Duration::from_secs(2),
        configuration.connect(tokio_postgres::NoTls),
    )
    .await
    .map_err(|_| StartupError::Connection)?
    .map_err(|_| StartupError::Connection)?;
    let driver = tokio::spawn(connection);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        local_demo_scope::stored_adventure(&client),
    )
    .await;
    drop(client);
    super::super::join_grant_driver(driver)
        .await
        .map_err(|_| StartupError::Driver)?;
    let state = result
        .map_err(|_| StartupError::Connection)?
        .map_err(StartupError::Database)?;
    // Campaign chapters are generated from the campaign record and keep pattern prose for now.
    if settings.mode == super::super::startup_configuration::AdventureMode::Campaign {
        return super::campaign_startup::select(settings, current, database, state);
    }
    let fresh = match &state {
        AdventureStartup::Fresh => Some(fresh_adventure(settings, storyteller).await?),
        AdventureStartup::Stored(_) => None,
    };
    select_with(settings, current, database, state, fresh)
}
/// The pattern adventure is always built first; the storyteller may then rewrite its text.
/// Whatever stops it (off, over budget, slow, refused) leaves the pattern adventure unchanged.
async fn fresh_adventure(
    settings: &AdventureSettings,
    storyteller: &Storyteller,
) -> Result<FreshAdventure, StartupError> {
    let (tables, seed, baseline) = pattern_adventure(settings)?;
    let options = GenerateOptions {
        rules_mode: settings.rules_mode,
    };
    let outcome = expression::express(
        &tables,
        seed,
        options,
        &ForcedEntries::new(),
        &baseline,
        storyteller,
    )
    .await;
    Ok(match outcome {
        Ok((generated, model)) => FreshAdventure {
            prose: model.summary(),
            generated,
            model: Some(model),
        },
        Err(reason) => FreshAdventure {
            generated: baseline,
            model: None,
            prose: format!("prose=patterns ({reason})"),
        },
    })
}
fn pattern_adventure(
    settings: &AdventureSettings,
) -> Result<(Tables, u64, GeneratedAdventure), StartupError> {
    let seed = match settings.seed {
        Some(seed) => seed,
        None => u64::from_le_bytes(model::random::<8>().map_err(|_| StartupError::Seed)?),
    };
    let tables = Tables::load(&settings.tables).map_err(StartupError::Tables)?;
    let generated = generate(
        &tables,
        seed,
        GenerateOptions {
            rules_mode: settings.rules_mode,
        },
        &ForcedEntries::new(),
    )
    .map_err(StartupError::Generate)?;
    Ok((tables, seed, generated))
}
/// Store state determines fresh/resume; requested seed/tables apply only to a fresh database.
#[cfg(test)]
pub(crate) fn select(
    settings: &AdventureSettings,
    current: &Path,
    database: &str,
    state: AdventureStartup,
) -> Result<PreparedAdventure, StartupError> {
    if settings.mode == super::super::startup_configuration::AdventureMode::Campaign {
        return super::campaign_startup::select(settings, current, database, state);
    }
    select_with(settings, current, database, state, None)
}
/// [`select`], using an already rolled (and possibly model-written) adventure when fresh.
fn select_with(
    settings: &AdventureSettings,
    current: &Path,
    database: &str,
    state: AdventureStartup,
    fresh: Option<FreshAdventure>,
) -> Result<PreparedAdventure, StartupError> {
    if !database.starts_with("df_gameplay_demo_")
        || database.len() > 63
        || !database
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(StartupError::DatabaseName);
    }
    let directory = current.join("development/runs").join(database);
    let (documents, is_fresh, prose) = match state {
        AdventureStartup::Fresh => {
            let fresh = match fresh {
                Some(fresh) => fresh,
                None => FreshAdventure {
                    generated: pattern_adventure(settings)?.2,
                    model: None,
                    prose: "prose=patterns (expression not attempted)".to_owned(),
                },
            };
            let documents = fresh
                .generated
                .documents_expressed(settings.seed.is_some(), fresh.model)
                .map_err(StartupError::Generate)?;
            record::create_cache(&directory, &documents).map_err(StartupError::Record)?;
            (documents, true, Some(fresh.prose))
        }
        AdventureStartup::Stored(documents) => {
            record::verify_cache(&directory, &documents).map_err(StartupError::Record)?;
            (documents, false, None)
        }
    };
    let (loaded, record) = record::reload(&documents).map_err(StartupError::Record)?;
    let prose = prose.unwrap_or_else(|| record.prose_summary());
    Ok(PreparedAdventure {
        directory,
        loaded,
        documents,
        record,
        fresh: is_fresh,
        prose,
        campaign: None,
    })
}

/// Presentation substitution only: stored pack/record bytes and intended settings stay pinned.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ArtSubstitution {
    pub(crate) art_key: String,
    pub(crate) fallback: String,
}
fn painting_present(asset_root: &Path, asset: &str) -> bool {
    let Some(stem) = asset.strip_prefix("assets/ui/generated/") else {
        return false;
    };
    ["webp", "png", "jpg"].iter().any(|extension| {
        asset_root
            .join("generated")
            .join(format!("{stem}.{extension}"))
            .is_file()
    })
}
/// Called at native composition with the actual served UI art directory, on fresh and resumed runs.
/// The required harbor painting is the final fallback when even the location painting is absent.
pub(crate) fn substitute_missing_art(
    loaded: &mut LoadedPack,
    record: &AdventureRecord,
    asset_root: &Path,
) -> Vec<ArtSubstitution> {
    let general = match &record.settings {
        Some(settings) => Some((
            settings.confrontation.art_key.clone(),
            format!(
                "assets/ui/generated/scene-{}",
                settings.confrontation.art_key.trim_start_matches("loc-")
            ),
        )),
        None => loaded
            .pack
            .scenes
            .iter()
            .find(|s| s.key == super::super::pack::SceneKey::Room)
            .map(|s| (s.asset.clone(), s.asset.clone())),
    };
    let (fallback_key, fallback_asset) = match general {
        Some((key, asset)) if painting_present(asset_root, &asset) => (key, asset),
        _ => (
            "harbor-general".into(),
            "assets/ui/scenes/mara-harbor-v4.png".into(),
        ),
    };
    let mut substitutions = Vec::new();
    for scene in &mut loaded.pack.scenes {
        if scene.asset.starts_with("assets/ui/generated/")
            && !painting_present(asset_root, &scene.asset)
        {
            let art_key = scene
                .setting
                .as_ref()
                .map(|s| s.art_key.clone())
                .unwrap_or_else(|| scene.asset.clone());
            scene.asset = fallback_asset.clone();
            println!("ADVENTURE art missing {art_key} -> {fallback_key}");
            substitutions.push(ArtSubstitution {
                art_key,
                fallback: fallback_key.clone(),
            });
        }
    }
    // A painting is presentation, not a new authored outcome. Keep script source
    // scenes/receipts immutable when asset availability changes across a resume.
    loaded.script_scene_assets.clear();
    if let Some(script) = &loaded.pack.beat_script {
        for (id, scene) in &script.scenes {
            if scene.asset.starts_with("assets/ui/generated/")
                && !painting_present(asset_root, &scene.asset)
            {
                let art_key = scene
                    .setting
                    .as_ref()
                    .map(|setting| setting.art_key.clone())
                    .unwrap_or_else(|| scene.asset.clone());
                loaded
                    .script_scene_assets
                    .insert(id.clone(), fallback_asset.clone());
                println!("ADVENTURE art missing {art_key} -> {fallback_key}");
                substitutions.push(ArtSubstitution {
                    art_key,
                    fallback: fallback_key.clone(),
                });
            }
        }
    }
    substitutions
}
