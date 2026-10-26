//! Host-only operator grant service over the current account authority and durable operator store.
//! It never authenticates HTTP callers, confirms MFA codes or handles secret material.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::operator_contract::{OperatorCommand, OperatorError, OperatorReceipt};
use crate::operator_ports::OperatorStore;
use crate::{AccountStatus, AccountStore, Clock, StoreError};

/// Admission service for trusted host composition. AF02 supplies separate MFA mechanics.
pub struct OperatorService<A, S> {
    accounts: A,
    store: S,
    clock: Arc<dyn Clock>,
}

impl<A: AccountStore, S: OperatorStore> OperatorService<A, S> {
    pub fn new(accounts: A, store: S, clock: Arc<dyn Clock>) -> Self {
        Self {
            accounts,
            store,
            clock,
        }
    }

    /// An approved replay returns its immutable receipt even after account or grant expiry.
    pub async fn execute_host(
        &self,
        command: OperatorCommand,
    ) -> Result<OperatorReceipt, OperatorError> {
        let operation = command.operation();
        let correlation = match &command {
            OperatorCommand::Grant(value) => value.correlation,
            OperatorCommand::Revoke(value) => value.correlation,
        };
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            if let Some(receipt) = self.store.receipt(&command).await? {
                return Ok(receipt);
            }
            // Revocation must remain possible when an account has closed or lost its MFA.
            if matches!(command, OperatorCommand::Grant(_)) {
                let account =
                    self.accounts.account(command.account()).await.map_err(
                        |error| match error {
                            StoreError::Unknown => OperatorError::Unknown,
                            StoreError::InvalidRecord => OperatorError::InvalidRecord,
                            StoreError::Unavailable => OperatorError::Unavailable,
                        },
                    )?;
                if !account.is_some_and(|account| {
                    account.is_verified && account.status == AccountStatus::Active
                }) {
                    return Err(OperatorError::AccountNotVerified);
                }
            }
            self.store.execute(command, self.clock.now_ms()).await
        })
        .await
        .unwrap_or(Err(OperatorError::Unknown));
        df_observe::record_audit_line(
            "operator.command",
            format!(
                "OPERATOR command operation_hash={} correlation_hash={} outcome={:?} elapsed_ms={}",
                df_observe::operation_hash(operation.as_bytes()),
                df_observe::operation_hash(correlation.as_bytes()),
                result.as_ref().map(|_| ()),
                started.elapsed().as_millis()
            ),
        );
        result
    }
}
