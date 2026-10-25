//! The room's table settings, stored beside the grants of the one local demonstration session.
//! A row exists only once a player has changed a setting; absence means the startup defaults.
use super::LocalDemoScopeIssuer;
use df_session::submission::RepositoryError;
use std::time::Duration;

/// The settings a player last chose, as the database holds them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredTableSettings {
    pub choice_hints_hidden: bool,
}

const READ_DEADLINE: Duration = Duration::from_secs(2);
const WRITE_DEADLINE: Duration = Duration::from_secs(5);

impl LocalDemoScopeIssuer {
    /// The stored settings, or `None` when no player has changed one yet (including a database
    /// created before this table existed). A failed read is an error, never a default.
    pub async fn stored_table_settings(
        &self,
    ) -> Result<Option<StoredTableSettings>, RepositoryError> {
        tokio::time::timeout(READ_DEADLINE, async {
            let client = self.authority.client()?;
            let table_exists: bool = client
                .query_one(
                    "SELECT to_regclass('df_local_demo.table_settings') IS NOT NULL",
                    &[],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?
                .try_get(0)
                .map_err(|_| RepositoryError::InvalidCandidate)?;
            if !table_exists {
                return Ok(None);
            }
            let row = client
                .query_opt(
                    "SELECT choice_hints_hidden FROM df_local_demo.table_settings WHERE singleton",
                    &[],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            row.map(|row| {
                Ok(StoredTableSettings {
                    choice_hints_hidden: row
                        .try_get(0)
                        .map_err(|_| RepositoryError::InvalidCandidate)?,
                })
            })
            .transpose()
        })
        .await
        .map_err(|_| RepositoryError::Unavailable)?
    }

    /// Replaces the stored settings. Called only by the serialization owner, after the caller
    /// has been authenticated as a seated player of this session.
    pub fn store_table_settings(
        &mut self,
        settings: StoredTableSettings,
    ) -> Result<(), RepositoryError> {
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            tokio::time::timeout(WRITE_DEADLINE, async {
                let client = self.authority.client()?;
                client
                    .batch_execute(
                        "CREATE TABLE IF NOT EXISTS df_local_demo.table_settings (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), choice_hints_hidden boolean NOT NULL)",
                    )
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                client
                    .execute(
                        "INSERT INTO df_local_demo.table_settings (singleton, choice_hints_hidden) VALUES (true, $1) ON CONFLICT (singleton) DO UPDATE SET choice_hints_hidden = EXCLUDED.choice_hints_hidden",
                        &[&settings.choice_hints_hidden],
                    )
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok(())
            })
            .await
            .map_err(|_| RepositoryError::Unavailable)?
        })
    }
}
