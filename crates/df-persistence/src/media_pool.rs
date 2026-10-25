//! A small fixed set of PostgreSQL connections for the media catalog: each opened on first use and
//! reopened after it closes, with the session limits every media statement runs under.
//! TODO(O2-07): `account_store` keeps an identical private pool; share one when B's file frees.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use df_assets::CatalogError;
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_postgres::{Client, Config};

use crate::postgres_tls::{PostgresTransport, connect};

const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
/// The catalog never holds a transaction open across an external call, so these are generous.
const SESSION_SETTINGS: &str = "SET statement_timeout = '10s'; SET lock_timeout = '10s'; \
    SET idle_in_transaction_session_timeout = '15s'";

pub(crate) struct Slot {
    pub(crate) client: Option<Client>,
    driver: Option<JoinHandle<()>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.abort();
        }
    }
}

pub(crate) struct Pool {
    config: Config,
    transport: PostgresTransport,
    slots: Vec<Mutex<Slot>>,
    next: AtomicUsize,
}

impl Pool {
    pub(crate) fn new(config: Config, transport: PostgresTransport, connections: usize) -> Self {
        Self {
            config,
            transport,
            slots: (0..connections.max(1))
                .map(|_| {
                    Mutex::new(Slot {
                        client: None,
                        driver: None,
                    })
                })
                .collect(),
            next: AtomicUsize::new(0),
        }
    }

    async fn open_one(&self) -> Result<(Client, JoinHandle<()>), CatalogError> {
        let (client, connection) =
            tokio::time::timeout(CONNECT_DEADLINE, connect(&self.config, &self.transport))
                .await
                .map_err(|_| CatalogError::Unavailable)?
                .map_err(|_| CatalogError::Unavailable)?;
        // The driver ends with its connection; a closed client is replaced at the next checkout.
        let driver = tokio::spawn(async move {
            let _ended = connection.await;
        });
        client
            .batch_execute(SESSION_SETTINGS)
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        Ok((client, driver))
    }

    pub(crate) async fn checkout(&self) -> Result<tokio::sync::MutexGuard<'_, Slot>, CatalogError> {
        // Relaxed: the counter only spreads callers over slots.
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.slots.len();
        let mut slot = self.slots[index].lock().await;
        if slot.client.as_ref().is_none_or(Client::is_closed) {
            let (client, driver) = self.open_one().await?;
            if let Some(old) = slot.driver.replace(driver) {
                old.abort();
            }
            slot.client = Some(client);
        }
        Ok(slot)
    }
}
