//! Local-demo startup adapter; all random selection remains in the pure generator.
use super::super::{model, pack::LoadedPack, startup_configuration::AdventureSettings};
use super::{
    ForcedEntries, GenerateError, GenerateOptions, Tables, TablesError, generate,
    record::{self, AdventureRecord, RecordError},
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
    select(settings, current, database, state)
}
/// Store state determines fresh/resume; requested seed/tables apply only to a fresh database.
pub(crate) fn select(
    settings: &AdventureSettings,
    current: &Path,
    database: &str,
    state: AdventureStartup,
) -> Result<PreparedAdventure, StartupError> {
    if settings.mode == super::super::startup_configuration::AdventureMode::Campaign {
        return super::campaign_startup::select(settings, current, database, state);
    }
    if !database.starts_with("df_gameplay_demo_")
        || database.len() > 63
        || !database
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(StartupError::DatabaseName);
    }
    let directory = current.join("development/runs").join(database);
    let (documents, fresh) = match state {
        AdventureStartup::Fresh => {
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
            let documents = generated
                .documents(settings.seed.is_some())
                .map_err(StartupError::Generate)?;
            record::create_cache(&directory, &documents).map_err(StartupError::Record)?;
            (documents, true)
        }
        AdventureStartup::Stored(documents) => {
            record::verify_cache(&directory, &documents).map_err(StartupError::Record)?;
            (documents, false)
        }
    };
    let (loaded, record) = record::reload(&documents).map_err(StartupError::Record)?;
    Ok(PreparedAdventure {
        directory,
        loaded,
        documents,
        record,
        fresh,
        campaign: None,
    })
}
