//! Fake email preferences and data-export status (TODO(B): replaced by B's account service).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::wire::{EmailPreferences, ExportStatus};
use df_customer::Clock;
use df_types::{AccountId, OperationId};

use super::lock;
use crate::app_api::ports::{AccountExtrasPort, PortResult};

#[derive(Default)]
struct ExtrasState {
    preferences: HashMap<AccountId, EmailPreferences>,
    exports: HashMap<AccountId, (ExportStatus, Option<u64>)>,
    seen_operations: HashMap<(AccountId, OperationId), (ExportStatus, Option<u64>)>,
}

pub struct FakeExtras {
    clock: Arc<dyn Clock>,
    state: Mutex<ExtrasState>,
}

impl FakeExtras {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(ExtrasState::default()),
        }
    }
}

impl AccountExtrasPort for FakeExtras {
    async fn preferences(&self, account: AccountId) -> PortResult<EmailPreferences> {
        Ok(lock(&self.state)
            .preferences
            .get(&account)
            .copied()
            .unwrap_or(EmailPreferences {
                product_news: false,
                session_reminders: true,
                transactional_always_on: true,
            }))
    }

    async fn save_preferences(
        &self,
        account: AccountId,
        preferences: EmailPreferences,
        _operation: OperationId,
    ) -> PortResult<()> {
        lock(&self.state).preferences.insert(
            account,
            EmailPreferences {
                transactional_always_on: true,
                ..preferences
            },
        );
        Ok(())
    }

    async fn export_status(&self, account: AccountId) -> PortResult<(ExportStatus, Option<u64>)> {
        Ok(lock(&self.state)
            .exports
            .get(&account)
            .copied()
            .unwrap_or((ExportStatus::NotRequested, None)))
    }

    async fn request_export(
        &self,
        account: AccountId,
        operation: OperationId,
    ) -> PortResult<(ExportStatus, Option<u64>)> {
        let mut state = lock(&self.state);
        if let Some(first) = state.seen_operations.get(&(account, operation)) {
            return Ok(*first);
        }
        let value = (ExportStatus::Requested, Some(self.clock.now_ms()));
        state.exports.insert(account, value);
        state.seen_operations.insert((account, operation), value);
        Ok(value)
    }
}
