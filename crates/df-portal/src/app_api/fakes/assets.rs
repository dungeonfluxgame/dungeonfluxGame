//! Fake generated-media catalog (TODO(O2): replaced by an adapter over `df-assets`
//! `CatalogService` / `AssetRecord`). In-memory, paged, filterable by kind and campaign.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::wire::{ActionOutcome, AssetHoldState, AssetKind, AssetRow, AssetsView};
use df_customer::Clock;
use df_types::AccountId;

use super::{Receipts, lock};
use crate::app_api::ports::{AssetsPort, OperatorAct, PortError, PortResult};

const PAGE_ASSETS: usize = 20;

#[derive(Default)]
struct AssetsState {
    assets: HashMap<AccountId, Vec<AssetRow>>,
    receipts: Receipts<ActionOutcome>,
}

pub struct FakeAssets {
    _clock: Arc<dyn Clock>,
    state: Mutex<AssetsState>,
}

impl FakeAssets {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            _clock: clock,
            state: Mutex::new(AssetsState::default()),
        }
    }

    pub fn seed_asset(&self, account: AccountId, row: AssetRow) {
        let mut state = lock(&self.state);
        let rows = state.assets.entry(account).or_default();
        rows.push(row);
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
    }
}

impl AssetsPort for FakeAssets {
    async fn list(
        &self,
        account: AccountId,
        kind: Option<AssetKind>,
        campaign_id: Option<&str>,
        cursor: Option<&str>,
    ) -> PortResult<AssetsView> {
        let start = match cursor {
            None => 0,
            Some(text) => text.parse::<usize>().map_err(|_| PortError::NotFound)?,
        };
        let state = lock(&self.state);
        let all = state.assets.get(&account).cloned().unwrap_or_default();
        let filtered: Vec<AssetRow> = all
            .into_iter()
            .filter(|row| kind.is_none_or(|kind| row.kind == kind))
            .filter(|row| campaign_id.is_none_or(|id| row.campaign_id.as_deref() == Some(id)))
            .collect();
        let total = filtered.len() as u64;
        let page: Vec<AssetRow> = filtered
            .iter()
            .skip(start)
            .take(PAGE_ASSETS)
            .cloned()
            .collect();
        let end = start + page.len();
        Ok(AssetsView {
            next_cursor: (end < filtered.len()).then(|| end.to_string()),
            assets: page,
            total,
        })
    }

    async fn set_hold(
        &self,
        act: OperatorAct,
        account: AccountId,
        asset_id: &str,
        held: bool,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let rows = state.assets.get_mut(&account).ok_or(PortError::NotFound)?;
        let row = rows
            .iter_mut()
            .find(|row| row.asset_id == asset_id)
            .ok_or(PortError::NotFound)?;
        row.hold_state = if held {
            AssetHoldState::Held
        } else {
            AssetHoldState::Ready
        };
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Done);
        Ok(ActionOutcome::Done)
    }
}
