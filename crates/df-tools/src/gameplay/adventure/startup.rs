//! Local-demo startup adapter; all random selection remains in the pure generator.
use super::super::{
    model,
    pack::LoadedPack,
    startup_configuration::{AdventureSettings, content_rating},
    storyteller::Storyteller,
};
use super::{
    ForcedEntries, GenerateError, GenerateOptions, GeneratedAdventure, Tables, TablesError,
    expression, generate,
    record::{self, AdventureRecord, ModelExpression, RecordError},
};
use df_persistence::local_demo_scope::{self, AdventureStartup, DemoAdventure};
use df_render::art_contract;
use std::{
    fmt,
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const CONNECTION_DEADLINE: Duration = Duration::from_secs(20);
const CONNECTION_ATTEMPT_DEADLINE: Duration = Duration::from_secs(2);
const CONNECTION_RETRY_BACKOFF: Duration = Duration::from_millis(250);

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
    FreeRoam(crate::gameplay::place_creation::CreationError),
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
            Self::FreeRoam(error) => {
                write!(f, "free-roam generation unavailable: {}", error.label())
            }
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
    let (client, connection) =
        connect_with_retry(|| configuration.connect(tokio_postgres::NoTls)).await?;
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

async fn connect_with_retry<F, Fut, T, E>(mut connect: F) -> Result<T, StartupError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let deadline = Instant::now() + CONNECTION_DEADLINE;
    let mut attempt = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StartupError::Connection);
        }
        attempt += 1;
        let attempt_deadline = CONNECTION_ATTEMPT_DEADLINE.min(remaining);
        let result = tokio::time::timeout(attempt_deadline, connect()).await;
        if let Ok(Ok(value)) = result {
            return Ok(value);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StartupError::Connection);
        }
        println!(
            "ADVENTURE database_connect outcome=retry attempt={attempt} elapsed_ms={}",
            CONNECTION_DEADLINE.saturating_sub(remaining).as_millis()
        );
        tokio::time::sleep(CONNECTION_RETRY_BACKOFF.min(remaining)).await;
    }
}

#[cfg(test)]
#[tokio::test]
async fn database_connection_retries_after_two_failures() {
    let mut attempts = 0;
    let result = connect_with_retry(|| {
        attempts += 1;
        std::future::ready(match attempts {
            1 | 2 => Err(()),
            _ => Ok("connected"),
        })
    })
    .await;

    assert!(matches!(result, Ok("connected")));
    assert_eq!(attempts, 3);
}

#[cfg(test)]
#[tokio::test(flavor = "current_thread")]
async fn fresh_adventure_requests_emotional_profiles_for_generated_dossiers() {
    use crate::gameplay::{
        adventure::RulesMode,
        startup_configuration::{AdventureMode, AdventureSettings},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    use std::{path::PathBuf, sync::Arc, time::Duration};

    let compute: Arc<dyn Fn(&str) -> serde_json::Value + Send + Sync> = Arc::new(|prompt| {
        let facts = prompt
            .lines()
            .find_map(|line| line.strip_prefix("FACTS: "))
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .expect("storyteller request has bounded facts");
        if facts.get("dossier").is_some() {
            let profile = serde_json::json!({
                "orientation": "They keep a steady eye on the road.",
                "adult_only": true,
                "feelings": [],
                "forgiveness": "forgiving",
                "patience": "middling",
                "tells": [],
                "legacy_reactions": facts["dossier"]["legacy_reactions"],
            });
            return serde_json::json!({"profile": profile.to_string()});
        }
        let slots = prompt
            .lines()
            .find_map(|line| line.strip_prefix("SLOTS: "))
            .and_then(|line| serde_json::from_str::<Vec<serde_json::Value>>(line).ok())
            .expect("storyteller request has bounded slots");
        let answers = slots
            .into_iter()
            .map(|slot| {
                let name = slot["name"].as_str().expect("a named output slot");
                let draft = facts["drafts"][name]
                    .as_str()
                    .expect("the request includes its pattern draft");
                (name.to_owned(), serde_json::Value::String(draft.to_owned()))
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::Value::Object(answers)
    });
    let endpoint = FakeEndpoint::start((0..128).map(|_| Reply::Compute(compute.clone())).collect());
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/campaign/luna6/B31/startup-profile-cache");
    let _ = std::fs::remove_dir_all(&cache);
    let storyteller = teller(endpoint.address, 200_000, Duration::from_secs(15), &cache);
    let settings = AdventureSettings {
        mode: AdventureMode::Random,
        seed: Some(51),
        tables: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables"),
        rules_mode: RulesMode::Standard,
        chapter_type: None,
    };

    let fresh = fresh_adventure(&settings, &storyteller).await.unwrap();
    let profile_requests = endpoint
        .bodies()
        .iter()
        .filter(|body| {
            body["input"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("npc_emotional_profile"))
        })
        .count();

    assert_eq!(profile_requests, fresh.generated.dossiers.len());
    assert!(
        fresh
            .generated
            .dossiers
            .iter()
            .all(|dossier| dossier.emotional_profile.is_some())
    );
    let documents = fresh
        .generated
        .documents_expressed(true, fresh.model)
        .unwrap();
    let (_, stored) = super::record::reload(&documents).unwrap();
    assert!(
        stored
            .dossiers
            .iter()
            .all(|dossier| dossier.emotional_profile.is_some())
    );
    let _ = std::fs::remove_dir_all(cache);
}

/// The pattern adventure is always built first; the storyteller may then rewrite its text.
/// Whatever stops it (off, over budget, slow, refused) leaves the pattern adventure unchanged.
async fn fresh_adventure(
    settings: &AdventureSettings,
    storyteller: &Storyteller,
) -> Result<FreshAdventure, StartupError> {
    if settings.mode == super::super::startup_configuration::AdventureMode::FreeRoam {
        let seed = match settings.seed {
            Some(seed) => seed,
            None => u64::from_le_bytes(model::random::<8>().map_err(|_| StartupError::Seed)?),
        };
        let generated =
            super::free_roam::generate(seed, settings.rules_mode, content_rating(), storyteller)
                .await
                .map_err(StartupError::FreeRoam)?;
        return Ok(FreshAdventure {
            generated,
            model: None,
            prose: "prose=narrative-engine free roam".into(),
        });
    }
    if settings.mode == super::super::startup_configuration::AdventureMode::MerchantShop {
        return Ok(FreshAdventure {
            generated: fresh_shop(settings)?,
            model: None,
            prose: "prose=authored freeplay shop".into(),
        });
    }
    let (tables, seed, baseline) = pattern_adventure(settings)?;
    let options = GenerateOptions {
        rules_mode: settings.rules_mode,
        rating: content_rating(),
    };
    let outcome = expression::express(
        &tables,
        seed,
        options,
        &forced_entries(settings),
        &baseline,
        storyteller,
    )
    .await;
    let mut fresh = match outcome {
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
    };
    super::super::npc_dossier::write_emotional_profiles(storyteller, &mut fresh.generated.dossiers)
        .await;
    Ok(fresh)
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
            rating: content_rating(),
        },
        &forced_entries(settings),
    )
    .map_err(StartupError::Generate)?;
    Ok((tables, seed, generated))
}
fn fresh_shop(settings: &AdventureSettings) -> Result<GeneratedAdventure, StartupError> {
    let seed = match settings.seed {
        Some(seed) => seed,
        None => u64::from_le_bytes(model::random::<8>().map_err(|_| StartupError::Seed)?),
    };
    super::shop::generate(seed, settings.rules_mode, content_rating())
        .map_err(StartupError::Generate)
}
/// Development override of the chapter type; the generator refuses ids missing from the tables.
fn forced_entries(settings: &AdventureSettings) -> ForcedEntries {
    settings
        .chapter_type
        .iter()
        .map(|kind| (super::beat_tables::CHAPTER_TYPE.into(), kind.clone()))
        .collect::<ForcedEntries>()
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
                None if settings.mode
                    == super::super::startup_configuration::AdventureMode::FreeRoam =>
                {
                    return Err(StartupError::FreeRoam(
                        crate::gameplay::place_creation::CreationError::Off,
                    ));
                }
                None => FreshAdventure {
                    generated: if settings.mode
                        == super::super::startup_configuration::AdventureMode::MerchantShop
                    {
                        fresh_shop(settings)?
                    } else {
                        pattern_adventure(settings)?.2
                    },
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
/// A reviewed photograph the scene names that is not in the directory is replaced, for
/// presentation only, by the art contract's room-photo role: the place's live room photograph,
/// made on demand, with the neutral stage until it is ready. No painting stands in for it.
pub(crate) fn substitute_missing_art(
    loaded: &mut LoadedPack,
    _record: &AdventureRecord,
    asset_root: &Path,
) -> Vec<ArtSubstitution> {
    // A new freeplay location remains pending when its own reference is absent.
    if loaded
        .pack
        .beat_script
        .as_ref()
        .is_some_and(|script| script.mode == super::super::beat_script::ScriptMode::Freeplay)
    {
        return Vec::new();
    }
    let fallback = art_contract::ROOM_PHOTO;
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
            scene.asset = fallback.to_owned();
            println!("ADVENTURE art missing {art_key} -> {fallback}");
            substitutions.push(ArtSubstitution {
                art_key,
                fallback: fallback.to_owned(),
            });
        }
    }
    // A picture is presentation, not a new authored outcome. Keep script source
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
                    .insert(id.clone(), fallback.to_owned());
                println!("ADVENTURE art missing {art_key} -> {fallback}");
                substitutions.push(ArtSubstitution {
                    art_key,
                    fallback: fallback.to_owned(),
                });
            }
        }
    }
    substitutions
}
