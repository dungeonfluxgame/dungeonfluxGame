//! Immutable generated adventure bytes, committed with the local run's first checkpoint.
use super::DemoInitializationError;
use df_session::submission::RepositoryError;
use tokio_postgres::{Client, Transaction};
/// The largest stored pack, tuning or record document. Scene models, dossiers and item detail
/// grew the generated pack past 256 KiB (267,859 bytes on 2026-10-14).
pub const ADVENTURE_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAXIMUM_DOCUMENT_BYTES: usize = ADVENTURE_DOCUMENT_BYTES;
/// Native-only private content: deliberately no Debug or public view projection.
#[derive(Clone, Eq, PartialEq)]
pub struct DemoAdventure {
    pub pack: Vec<u8>,
    pub tuning: Vec<u8>,
    pub record: Vec<u8>,
}
pub enum AdventureStartup {
    Fresh,
    Stored(DemoAdventure),
}
fn refused(stage: &'static str, class: RepositoryError) -> DemoInitializationError {
    DemoInitializationError { stage, class }
}
impl DemoAdventure {
    pub(super) fn validate_bounds(&self) -> Result<(), DemoInitializationError> {
        if [&self.pack, &self.tuning, &self.record]
            .iter()
            .any(|bytes| bytes.is_empty() || bytes.len() > MAXIMUM_DOCUMENT_BYTES)
        {
            return Err(refused("adventure_bytes", RepositoryError::Capacity));
        }
        Ok(())
    }
}
/// Reads only an admitted local-demo database. Existing runs without the immutable row refuse
/// random-adventure resume; absence is never permission to regenerate an existing run.
pub async fn stored_adventure(
    client: &Client,
) -> Result<AdventureStartup, DemoInitializationError> {
    let catalog=client.query_one("SELECT current_database(), to_regnamespace('df_game') IS NOT NULL, to_regnamespace('df_local_demo') IS NOT NULL, to_regclass('df_local_demo.adventure') IS NOT NULL",&[]).await.map_err(|_|refused("adventure_catalog",RepositoryError::Unavailable))?;
    let database: String = catalog
        .try_get(0)
        .map_err(|_| refused("adventure_catalog", RepositoryError::InvalidCandidate))?;
    if !database.starts_with("df_gameplay_demo_") {
        return Err(refused(
            "configured_identity",
            RepositoryError::Unauthorized,
        ));
    }
    let has_game_schema: bool = catalog
        .try_get(1)
        .map_err(|_| refused("adventure_catalog", RepositoryError::InvalidCandidate))?;
    let has_local_schema: bool = catalog
        .try_get(2)
        .map_err(|_| refused("adventure_catalog", RepositoryError::InvalidCandidate))?;
    let has_adventure_table: bool = catalog
        .try_get(3)
        .map_err(|_| refused("adventure_catalog", RepositoryError::InvalidCandidate))?;
    if !has_game_schema && !has_local_schema && !has_adventure_table {
        return Ok(AdventureStartup::Fresh);
    }
    if !has_game_schema || !has_local_schema || !has_adventure_table {
        return Err(refused(
            "adventure_missing_or_incomplete",
            RepositoryError::InvalidCandidate,
        ));
    }
    let maximum_document_bytes = MAXIMUM_DOCUMENT_BYTES as i32;
    let adventure_row=client.query_opt("SELECT pack,tuning,record FROM df_local_demo.adventure WHERE singleton AND octet_length(pack) BETWEEN 1 AND $1 AND octet_length(tuning) BETWEEN 1 AND $1 AND octet_length(record) BETWEEN 1 AND $1",&[&maximum_document_bytes]).await.map_err(|_|refused("adventure_load",RepositoryError::Unavailable))?.ok_or_else(||refused("adventure_missing_or_oversize",RepositoryError::InvalidCandidate))?;
    let adventure = DemoAdventure {
        pack: adventure_row
            .try_get("pack")
            .map_err(|_| refused("adventure_load", RepositoryError::InvalidCandidate))?,
        tuning: adventure_row
            .try_get("tuning")
            .map_err(|_| refused("adventure_load", RepositoryError::InvalidCandidate))?,
        record: adventure_row
            .try_get("record")
            .map_err(|_| refused("adventure_load", RepositoryError::InvalidCandidate))?,
    };
    adventure.validate_bounds()?;
    Ok(AdventureStartup::Stored(adventure))
}
pub(super) async fn insert(
    transaction: &Transaction<'_>,
    adventure: &DemoAdventure,
) -> Result<(), DemoInitializationError> {
    adventure.validate_bounds()?;
    transaction.batch_execute("CREATE TABLE df_local_demo.adventure (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), pack bytea NOT NULL CHECK(octet_length(pack) BETWEEN 1 AND 1048576), tuning bytea NOT NULL CHECK(octet_length(tuning) BETWEEN 1 AND 1048576), record bytea NOT NULL CHECK(octet_length(record) BETWEEN 1 AND 1048576)); REVOKE ALL ON df_local_demo.adventure FROM PUBLIC;
        CREATE FUNCTION df_local_demo.refuse_adventure_mutation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'generated adventure bytes are immutable'; END; $$;
        CREATE TRIGGER immutable_adventure BEFORE UPDATE OR DELETE ON df_local_demo.adventure FOR EACH ROW EXECUTE FUNCTION df_local_demo.refuse_adventure_mutation();").await.map_err(|_|refused("adventure_schema",RepositoryError::Unavailable))?;
    transaction.execute("INSERT INTO df_local_demo.adventure (singleton,pack,tuning,record) VALUES(true,$1,$2,$3)",&[&adventure.pack,&adventure.tuning,&adventure.record]).await.map_err(|_|refused("adventure_seed",RepositoryError::Unavailable))?;
    Ok(())
}
