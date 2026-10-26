//! Account profiles and the account owner's personal-tenant binding (S1-02).

use crate::database::Connection;
use df_campaign::{
    errors::{StoreError, StoreResult},
    store::{AccountDirectory, AccountProfile},
};
use df_customer::AccountStore;
use df_types::AccountId;

pub(super) struct Accounts {
    accounts: df_persistence::account_store::PostgresAccountStore,
}

impl Accounts {
    pub(super) fn new(
        accounts: df_persistence::account_store::PostgresAccountStore,
        _connection: Connection,
    ) -> Self {
        Self { accounts }
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
        let tenant = self
            .accounts
            .tenant_for(account)
            .await
            .map_err(account_error)?;
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

fn account_error(error: df_customer::StoreError) -> StoreError {
    let reason = match error {
        df_customer::StoreError::InvalidRecord => "invalid_record",
        df_customer::StoreError::Unavailable => "unavailable",
        df_customer::StoreError::Unknown => "unknown",
    };
    df_observe::record_audit_line(
        "hosted.account_directory_failed",
        format!("HOSTED account_directory_failed source=tenant_lookup reason={reason}"),
    );
    match error {
        df_customer::StoreError::InvalidRecord => StoreError::InvalidRecord,
        df_customer::StoreError::Unavailable | df_customer::StoreError::Unknown => {
            StoreError::Unavailable
        }
    }
}
