//! Finance service construction. Unimplemented paid-admission collaborators always refuse.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};

use df_commerce::finance_ids::{CampaignKey, PayerId};
use df_finance::{
    ports::*,
    service::{FinanceConfig, FinanceService},
};
use sha2::{Digest, Sha256};
use tokio_postgres::{Client, Transaction};

pub(super) struct Unavailable;

impl FencePort for Unavailable {
    async fn fence_current(
        &self,
        _transaction: &Transaction<'_>,
        _request: &FenceRequest,
    ) -> Result<FenceVerdict, PortUnavailable> {
        Err(PortUnavailable)
    }
}

impl MediaPort for Unavailable {
    async fn admit_generation(
        &self,
        _transaction: &Transaction<'_>,
        _request: &MediaRequest,
    ) -> Result<MediaVerdict, PortUnavailable> {
        Err(PortUnavailable)
    }
}

impl MemberPort for Unavailable {
    async fn authorized_payers(
        &self,
        _campaign: CampaignKey,
        _payers: &[PayerId],
    ) -> Result<BTreeSet<PayerId>, PortUnavailable> {
        Err(PortUnavailable)
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        i64::try_from(df_customer::Clock::now_ms(&df_customer::SystemClock)).unwrap_or(i64::MAX)
    }
}

struct Ids {
    seed: [u8; 32],
    sequence: AtomicU64,
}

impl IdSource for Ids {
    fn next_id(&self) -> [u8; 16] {
        let mut hash = Sha256::new();
        hash.update(self.seed);
        hash.update(self.sequence.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        let digest = hash.finalize();
        let mut id = [0; 16];
        id.copy_from_slice(&digest[..16]);
        id
    }
}

pub(super) type Service = FinanceService<Unavailable, Unavailable, Unavailable>;

pub(super) fn create(client: Client) -> Result<Service, df_portal::app_api::ports::PortError> {
    let token = df_customer::SecretToken::generate()
        .map_err(|_| df_portal::app_api::ports::PortError::Unavailable)?;
    Ok(FinanceService::new(
        client,
        Unavailable,
        Unavailable,
        Unavailable,
        Box::new(SystemClock),
        Box::new(Ids {
            seed: Sha256::digest(token.text()).into(),
            sequence: AtomicU64::new(0),
        }),
        FinanceConfig::default(),
    ))
}
