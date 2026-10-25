//! Account profiles and the account owner's personal-tenant binding (S1-02).

use crate::database::Connection;
use df_campaign::{
    errors::{StoreError, StoreResult},
    store::{AccountDirectory, AccountProfile},
};
use df_customer::AccountStore;
use df_types::{AccountId, TenantId};

pub(super) struct Accounts {
    accounts: df_persistence::account_store::PostgresAccountStore,
    connection: Connection,
}

impl Accounts {
    pub(super) fn new(
        accounts: df_persistence::account_store::PostgresAccountStore,
        connection: Connection,
    ) -> Self {
        Self {
            accounts,
            connection,
        }
    }
}

impl AccountDirectory for Accounts {
    async fn profile(&self, account: AccountId) -> StoreResult<Option<AccountProfile>> {
        let record = self
            .accounts
            .account(account)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let tenant = if record.is_verified {
            let installed: bool = self
                .connection
                .client
                .query_one("SELECT to_regclass('accounts.tenant') IS NOT NULL", &[])
                .await
                .map_err(query_error)?
                .try_get(0)
                .map_err(|_| StoreError::InvalidRecord)?;
            if !installed {
                df_observe::record_audit_line(
                    "hosted.tenant_binding_unavailable",
                    "HOSTED tenant_binding_unavailable owner=S1-02".to_owned(),
                );
                return Err(StoreError::Unavailable);
            }
            let row = self
                .connection
                .client
                .query_opt(
                    "SELECT tenant_id FROM accounts.tenant WHERE owner_account_id = $1",
                    &[&account.as_bytes().as_slice()],
                )
                .await
                .map_err(query_error)?;
            row.map(|row| {
                let bytes: Vec<u8> = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
                TenantId::from_bytes(&bytes).map_err(|_| StoreError::InvalidRecord)
            })
            .transpose()?
        } else {
            None
        };
        Ok(Some(AccountProfile {
            display_name: record.display_name.map(|name| name.as_str().to_owned()),
            verified: record.is_verified,
            tenant,
            status: match record.status {
                df_customer::AccountStatus::Active => df_campaign::records::AccountStatus::Active,
                df_customer::AccountStatus::Closing => df_campaign::records::AccountStatus::Closing,
                df_customer::AccountStatus::Closed => df_campaign::records::AccountStatus::Closed,
            },
        }))
    }
}

fn query_error(error: tokio_postgres::Error) -> StoreError {
    df_observe::record_audit_line(
        "hosted.account_directory_failed",
        format!(
            "HOSTED account_directory_failed sqlstate={}",
            error.code().map_or("connection", |code| code.code())
        ),
    );
    StoreError::Unavailable
}
