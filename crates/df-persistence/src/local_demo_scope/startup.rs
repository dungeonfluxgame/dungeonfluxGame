//! Trusted local startup. Stored bytes never admit a new inventory or owner.
use df_model::checkpoint::{Checkpoint, ReferenceInventory};
use df_session::submission::RepositoryError;
use df_types::SessionId;
use tokio_postgres::{Client, Transaction};

use super::{
    CAMPAIGN, DISPLAY, DemoInitializationError, PLAYER, TENANT, initialize_with_adventure,
};
use crate::{NativeCodecLimits, NativeRecoverySource};

/// Credentials remain private to the native owner; this type intentionally has no Debug.
pub struct DemoStartup {
    pub checkpoint: Checkpoint,
    pub player_credential: [u8; 32],
    pub display_credential: [u8; 32],
    pub restored: bool,
}
/// Fresh source-owned identities are used only for an empty database; restore keeps grants.
pub struct DemoStartupIdentity {
    pub fence: [u8; 16],
    pub player_credential: [u8; 32],
    pub display_credential: [u8; 32],
}
fn error(stage: &'static str, class: RepositoryError) -> DemoInitializationError {
    DemoInitializationError { stage, class }
}

/// No failure is treated as permission to seed or overwrite an existing database.
pub async fn admit_startup(
    client: &mut Client,
    cold: &Checkpoint,
    identity: DemoStartupIdentity,
    codec: NativeCodecLimits,
    recovery: &NativeRecoverySource,
    validate: impl FnMut(&Checkpoint) -> Result<(), RepositoryError>,
) -> Result<DemoStartup, DemoInitializationError> {
    admit_startup_with_adventure(client, cold, identity, codec, recovery, None, validate).await
}
/// The optional adventure joins the same bootstrap transaction as the first checkpoint.
/// Restore compares all three authoritative documents before claiming the existing run.
pub async fn admit_startup_with_adventure(
    client: &mut Client,
    cold: &Checkpoint,
    identity: DemoStartupIdentity,
    codec: NativeCodecLimits,
    recovery: &NativeRecoverySource,
    adventure: Option<&super::DemoAdventure>,
    mut validate: impl FnMut(&Checkpoint) -> Result<(), RepositoryError>,
) -> Result<DemoStartup, DemoInitializationError> {
    let DemoStartupIdentity {
        fence,
        player_credential: player,
        display_credential: display,
    } = identity;
    if fence == [0; 16] {
        return Err(error("configured_identity", RepositoryError::Unauthorized));
    }
    let row = client.query_one("SELECT current_database(), to_regnamespace('df_game') IS NOT NULL AS game, to_regnamespace('df_local_demo') IS NOT NULL AS local", &[])
        .await.map_err(|_| error("startup_catalog", RepositoryError::Unavailable))?;
    let database: String = row
        .try_get(0)
        .map_err(|_| error("startup_catalog", RepositoryError::Unavailable))?;
    if !database.starts_with("df_gameplay_demo_") {
        return Err(error("configured_identity", RepositoryError::Unauthorized));
    }
    let game: bool = row
        .try_get("game")
        .map_err(|_| error("startup_catalog", RepositoryError::Unavailable))?;
    let local: bool = row
        .try_get("local")
        .map_err(|_| error("startup_catalog", RepositoryError::Unavailable))?;
    let fresh = classify_catalog(game, local)?;
    if fresh {
        validate(cold).map_err(|e| error("cold_validation", e))?;
        initialize_with_adventure(client, cold, fence, player, display, codec, adventure).await?;
        return Ok(DemoStartup {
            checkpoint: cold.clone(),
            player_credential: player,
            display_credential: display,
            restored: false,
        });
    }
    if let Some(adventure) = adventure {
        match super::stored_adventure(client).await? {
            super::AdventureStartup::Stored(stored) if &stored == adventure => {}
            _ => {
                return Err(error(
                    "adventure_revision",
                    RepositoryError::InvalidCandidate,
                ));
            }
        }
    }
    let tx = client
        .transaction()
        .await
        .map_err(|_| error("startup_transaction", RepositoryError::Unavailable))?;
    let admitted = restore(&tx, cold, fence, codec, recovery, &mut validate).await;
    match admitted {
        Ok(state) => {
            tx.commit()
                .await
                .map_err(|_| error("claim_commit", RepositoryError::UnresolvedCommit))?;
            Ok(state)
        }
        Err(original) => {
            tx.rollback()
                .await
                .map_err(|_| error("startup_rollback", RepositoryError::Unavailable))?;
            Err(original)
        }
    }
}

/// Read only bounded startup provenance after complete schema/catalog admission. The result
/// diagnoses incompatible files; it never grants an owner lease or supplies recovery authority.
pub async fn stored_startup_configuration(
    client: &Client,
    cold: &Checkpoint,
    codec: NativeCodecLimits,
) -> Result<Option<df_types::RevisionLabel>, RepositoryError> {
    let catalog = client.query_one("SELECT to_regnamespace('df_game') IS NOT NULL, to_regnamespace('df_local_demo') IS NOT NULL", &[]).await.map_err(|_| RepositoryError::Unavailable)?;
    let game: bool = catalog
        .try_get(0)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let local: bool = catalog
        .try_get(1)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    if classify_catalog(game, local).map_err(|error| error.class)? {
        return Ok(None);
    }
    let maximum =
        i64::try_from(codec.maximum_document_bytes).map_err(|_| RepositoryError::Capacity)?;
    let row = client
        .query_opt(
            crate::sql::LOAD_CURRENT,
            &[
                &TENANT.as_slice(),
                &cold.basis().session.as_bytes().as_slice(),
                &maximum,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .ok_or(RepositoryError::InvalidCandidate)?;
    let version: i32 = row
        .try_get("codec_version")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let schema: i32 = row
        .try_get("schema_version")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    if schema != i32::from(df_model::checkpoint::CHECKPOINT_SCHEMA) {
        return Err(RepositoryError::InvalidCandidate);
    }
    let bytes: Vec<u8> = row
        .try_get("complete_envelope")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    crate::checkpoint_codec::validate_storage_format(version, b"DFCP", &bytes)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    crate::checkpoint_codec::decode_checkpoint_configuration(&bytes, codec)
        .map(Some)
        .map_err(|_| RepositoryError::InvalidCandidate)
}

async fn restore(
    tx: &Transaction<'_>,
    cold: &Checkpoint,
    fence: [u8; 16],
    codec: NativeCodecLimits,
    recovery: &NativeRecoverySource,
    validate: &mut impl FnMut(&Checkpoint) -> Result<(), RepositoryError>,
) -> Result<DemoStartup, DemoInitializationError> {
    tx.batch_execute("SET LOCAL statement_timeout='2000ms'; SET LOCAL lock_timeout='1000ms'")
        .await
        .map_err(|_| error("startup_bounds", RepositoryError::Unavailable))?;
    let session = cold.basis().session;
    let row = tx
        .query_opt(
            crate::sql::LOCK_SESSION,
            &[&TENANT.as_slice(), &session.as_bytes().as_slice()],
        )
        .await
        .map_err(|_| error("owner_lock", RepositoryError::Unavailable))?
        .ok_or_else(|| error("missing_session", RepositoryError::InvalidCandidate))?;
    let live: bool = row
        .try_get("lease_current")
        .map_err(|_| error("owner_lock", RepositoryError::InvalidCandidate))?;
    let old_fence: Vec<u8> = row
        .try_get("owner_fence")
        .map_err(|_| error("owner_lock", RepositoryError::InvalidCandidate))?;
    validate_claim(live, &old_fence, fence)?;
    let basis = crate::decision_rows::current_row_basis(&row, session)
        .map_err(|e| error("stored_basis", e))?;
    let maximum = i64::try_from(codec.maximum_document_bytes)
        .map_err(|_| error("checkpoint_bounds", RepositoryError::Capacity))?;
    let stored = tx
        .query_opt(
            crate::sql::LOAD_CURRENT,
            &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &maximum],
        )
        .await
        .map_err(|_| error("checkpoint_read", RepositoryError::Unavailable))?
        .ok_or_else(|| error("missing_checkpoint", RepositoryError::InvalidCandidate))?;
    let inventory = ReferenceInventory {
        rules: &recovery.rules,
        content: &recovery.content,
        resources: &recovery.resources,
        assets: &recovery.assets,
    };
    let checkpoint = crate::decision_rows::decode_current_row(
        &stored,
        session,
        basis,
        cold.pins(),
        inventory,
        recovery.limits,
        codec,
    )
    .map_err(|e| error("checkpoint_decode", e))?;
    validate(&checkpoint).map_err(|e| error("engine_validation", e))?;
    let player = credential(tx, session, PLAYER, b"player").await?;
    let display = credential(tx, session, DISPLAY, b"display").await?;
    if player == display {
        return Err(error(
            "retained_access_unavailable",
            RepositoryError::Unauthorized,
        ));
    }
    let count = tx.execute(&format!("UPDATE df_game.sessions SET owner_fence=$3::bytea, lease_until=clock_timestamp()+interval '{} seconds' WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND owner_fence=$4::bytea AND lease_until<=clock_timestamp()", super::lease_seconds()), &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &fence.as_slice(), &old_fence]).await.map_err(|_| error("owner_claim", RepositoryError::Unavailable))?;
    if count != 1 {
        return Err(error("owner_claim", RepositoryError::RevisionConflict));
    }
    Ok(DemoStartup {
        checkpoint,
        player_credential: player,
        display_credential: display,
        restored: true,
    })
}

fn classify_catalog(game: bool, local: bool) -> Result<bool, DemoInitializationError> {
    match (game, local) {
        (false, false) => Ok(true),
        (true, true) => Ok(false),
        _ => Err(error(
            "incomplete_schema",
            RepositoryError::InvalidCandidate,
        )),
    }
}
fn validate_claim(
    live: bool,
    old_fence: &[u8],
    fence: [u8; 16],
) -> Result<(), DemoInitializationError> {
    if live {
        return Err(error("live_owner", RepositoryError::RevisionConflict));
    }
    if old_fence.len() != 16 || old_fence == [0; 16] || old_fence == fence || fence == [0; 16] {
        return Err(error("owner_fence", RepositoryError::InvalidCandidate));
    }
    Ok(())
}
fn admitted_credential(bytes: Vec<u8>) -> Result<[u8; 32], DemoInitializationError> {
    let credential: [u8; 32] = bytes
        .try_into()
        .map_err(|_| error("retained_access_unavailable", RepositoryError::Unauthorized))?;
    if credential == [0; 32] {
        return Err(error(
            "retained_access_unavailable",
            RepositoryError::Unauthorized,
        ));
    }
    Ok(credential)
}

async fn credential(
    tx: &Transaction<'_>,
    session: SessionId,
    principal: [u8; 16],
    role: &[u8],
) -> Result<[u8; 32], DemoInitializationError> {
    let rows = tx.query("SELECT CASE WHEN octet_length(credential)=32 THEN credential END AS credential FROM df_local_demo.grants WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND campaign_id=$3::bytea AND principal_id=$4::bytea AND effective_role=$5::bytea AND active AND expires_at>clock_timestamp() LIMIT 2", &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &CAMPAIGN.as_slice(), &principal.as_slice(), &role]).await.map_err(|_| error("retained_access_read", RepositoryError::Unavailable))?;
    if rows.len() != 1 {
        return Err(error(
            "retained_access_unavailable",
            RepositoryError::Unauthorized,
        ));
    }
    let bytes: Vec<u8> = rows[0]
        .try_get(0)
        .map_err(|_| error("retained_access_unavailable", RepositoryError::Unauthorized))?;
    admitted_credential(bytes)
}

/// An old owner cannot release a replacement. Canonical state and grant expiry are untouched.
pub async fn release_owner(
    client: &Client,
    session: SessionId,
    fence: [u8; 16],
) -> Result<(), RepositoryError> {
    if fence == [0; 16] {
        return Err(RepositoryError::Unauthorized);
    }
    let count = tokio::time::timeout(std::time::Duration::from_secs(2), client.execute("UPDATE df_game.sessions SET lease_until=LEAST(lease_until,clock_timestamp()) WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND owner_fence=$3::bytea", &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &fence.as_slice()])).await.map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?;
    if count != 1 {
        return Err(RepositoryError::RevisionConflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_completely_absent_catalog_is_fresh() {
        assert!(classify_catalog(false, false).unwrap());
        assert!(!classify_catalog(true, true).unwrap());
        for catalog in [(false, true), (true, false)] {
            assert_eq!(
                classify_catalog(catalog.0, catalog.1).unwrap_err().class,
                RepositoryError::InvalidCandidate
            );
        }
    }
    #[test]
    fn live_owner_never_admits_even_a_distinct_fence() {
        assert_eq!(
            validate_claim(true, &[1; 16], [2; 16]).unwrap_err().stage,
            "live_owner"
        );
        assert_eq!(
            validate_claim(true, &[1; 16], [1; 16]).unwrap_err().class,
            RepositoryError::RevisionConflict
        );
        assert!(validate_claim(false, &[1; 16], [2; 16]).is_ok());
    }
    #[test]
    fn expired_claim_requires_complete_distinct_nonzero_identity() {
        for (old, new) in [
            (vec![1; 15], [2; 16]),
            (vec![1; 17], [2; 16]),
            (vec![1; 16], [1; 16]),
            (vec![1; 16], [0; 16]),
            (vec![0; 16], [2; 16]),
        ] {
            assert_eq!(
                validate_claim(false, &old, new).unwrap_err().class,
                RepositoryError::InvalidCandidate
            );
        }
    }
    #[test]
    fn retained_grant_is_exact_and_nonzero() {
        assert_eq!(admitted_credential(vec![1; 32]).unwrap(), [1; 32]);
        for bytes in [vec![0; 32], vec![1; 31], vec![1; 33], Vec::new()] {
            assert_eq!(
                admitted_credential(bytes).unwrap_err().class,
                RepositoryError::Unauthorized
            );
        }
    }
}
