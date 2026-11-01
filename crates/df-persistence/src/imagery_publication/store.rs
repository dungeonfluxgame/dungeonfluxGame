//! Bounded PostgreSQL implementation of the owner-scoped result port.

use super::{
    PublicationCommand, PublicationOutcome, commit_publication, finalize_image_in, read_result_in,
    rows,
};
use crate::media_pool::Pool;
use crate::postgres_tls::{PostgresTransport, TransportRefusal, check_transport};
use df_imagery::publication::{ImageResults, PublicationError, PublishedImage};
use df_imagery::{ImageJobId, ImageOwner};
use std::sync::Arc;
use tokio_postgres::{Config, IsolationLevel};

/// Opening the already migrated publication store failed without changing any schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationOpenError {
    Transport(TransportRefusal),
    ConnectionBound,
    Unavailable,
    MissingSchema,
}

/// Publication transactions and scoped reads share the established bounded native pool.
#[derive(Clone)]
pub struct PostgresImagePublication {
    pub(super) pool: Arc<Pool>,
}

impl PostgresImagePublication {
    /// Output-only cleanup and its terminal seal commit together; failed acknowledgement is Unknown.
    pub async fn no_delivery(
        &self,
        request: &df_imagery::publication::NoDeliveryRequest<'_>,
    ) -> Result<super::NoDeliveryOutcome, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        let outcome = super::finalize_no_delivery_in(&tx, request).await?;
        commit_publication(tx).await?;
        Ok(outcome)
    }

    /// Rereads a scoped durable terminal seal after a lost cleanup acknowledgement.
    pub async fn read_no_delivery(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<df_imagery::publication::NoDeliveryReceipt>, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        let result = super::read_no_delivery_in(&tx, owner, job).await?;
        commit_publication(tx).await?;
        Ok(result)
    }

    /// Composition installs 0042 after 0038/0039/0040; runtime never migrates an older schema.
    pub async fn open_existing(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, PublicationOpenError> {
        check_transport(&config, &transport).map_err(PublicationOpenError::Transport)?;
        if connections == 0 {
            return Err(PublicationOpenError::ConnectionBound);
        }
        let pool = Pool::new(config, transport, connections);
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| PublicationOpenError::Unavailable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(PublicationOpenError::Unavailable)?;
            let present: bool = client.query_one("SELECT to_regclass('imagery.image_job_result') IS NOT NULL AND to_regclass('imagery.image_result_variant') IS NOT NULL", &[]).await.map_err(|error| { rows::record_failure(&error); PublicationOpenError::Unavailable })?.get(0);
            if !present {
                return Err(PublicationOpenError::MissingSchema);
            }
        }
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// A precommit refusal drops and rolls back the complete catalog/result transaction.
    pub async fn publish(
        &self,
        command: &PublicationCommand<'_>,
    ) -> Result<PublicationOutcome, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(rows::database_error)?;
        let outcome = finalize_image_in(&tx, command).await?;
        commit_publication(tx).await?;
        Ok(outcome)
    }
}

impl ImageResults for PostgresImagePublication {
    async fn read_result(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(rows::database_error)?;
        let result = read_result_in(&tx, owner, job).await?;
        commit_publication(tx).await?;
        Ok(result)
    }
}
