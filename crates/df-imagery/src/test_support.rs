//! Local-only fixtures run real consumer requests and the real native raster worker.
//! Only infrastructure ports use bounded in-memory state; no raw executor is exposed.

mod execution;
mod journal;
mod ports;
mod state;

use crate::runtime::{NativeImageRuntime, RuntimeError, RuntimeHandle, RuntimeSettings};
use crate::{
    AssetId, Audience, GlobalMode, ImageAuthorizer, ImageConfig, ImageOwner, ImageService,
    ImageServiceApi, SourceRef,
};
use state::{FixtureClock, FixtureIds, Store};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Fixture setup, exact source admission, or owned worker failure.
#[derive(Debug)]
pub enum FixtureError {
    Configuration(crate::ConfigError),
    Service(crate::ImageServiceError),
    Runtime(RuntimeError),
    InvalidImage,
    Capacity,
    MissingAsset,
    Worker,
}
impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transform fixture failed: {self:?}")
    }
}
impl std::error::Error for FixtureError {}

/// Owned local transform service for consumer tests; the caller supplies its real principal type.
pub struct TransformFixture<P: Sync> {
    owner: ImageOwner,
    config: ImageConfig,
    service: Arc<dyn ImageServiceApi<Principal = P>>,
    store: Store,
    stop: Option<RuntimeHandle>,
    worker: Option<JoinHandle<Result<(), RuntimeError>>>,
}
impl<P: Sync + 'static> TransformFixture<P> {
    /// Starts one actual native worker, with no credentials or paid routes.
    pub async fn new<A>(owner: ImageOwner, authorizer: A) -> Result<Self, FixtureError>
    where
        A: ImageAuthorizer<Principal = P> + 'static,
    {
        Self::new_inner(owner, authorizer, None).await
    }

    /// Uses one shared observer for actual consumer-service and native-worker lifecycle events.
    pub async fn new_with_observer<A>(
        owner: ImageOwner,
        authorizer: A,
        observer: Arc<dyn crate::telemetry::ImageObserver>,
    ) -> Result<Self, FixtureError>
    where
        A: ImageAuthorizer<Principal = P> + 'static,
    {
        Self::new_inner(owner, authorizer, Some(observer)).await
    }

    async fn new_inner<A>(
        owner: ImageOwner,
        authorizer: A,
        observer: Option<Arc<dyn crate::telemetry::ImageObserver>>,
    ) -> Result<Self, FixtureError>
    where
        A: ImageAuthorizer<Principal = P> + 'static,
    {
        let mut config = ImageConfig::defaults().map_err(FixtureError::Configuration)?;
        config.mode = GlobalMode::LocalOnly;
        for gate in &mut config.backends {
            gate.enabled = false;
        }
        let ids = FixtureIds::new();
        let store = Store::new(owner.clone(), ids.clone()).map_err(FixtureError::Runtime)?;
        let service = ImageService::new(
            store.clone(),
            config.clone(),
            FixtureClock,
            ids.clone(),
            authorizer,
        )
        .map_err(FixtureError::Configuration)?
        .with_results(store.clone())
        .with_observations(store.clone())
        .with_config_revision(1)
        .map_err(FixtureError::Service)?;
        let service = if let Some(observer) = &observer {
            service.with_observer(observer.clone())
        } else {
            service
        };
        let runtime = NativeImageRuntime {
            journal: store.clone(),
            execution: store.clone(),
            sources: store.clone(),
            publisher: store.clone(),
            admission: store.clone(),
            clock: FixtureClock,
            ids,
            credentials: ports::LocalOnly,
            costs: ports::LocalOnly,
            cost_policy: ports::LocalOnly,
            usage: store.clone(),
            telemetry: ports::FixtureObserver(observer),
            settings: RuntimeSettings {
                config: config.clone(),
                active_config_revision: 1,
                capacity: store.scope.clone(),
                poll_interval: Duration::from_millis(1),
            },
        };
        let (stop, future) = runtime.start();
        Ok(Self {
            owner,
            config,
            service: Arc::new(service),
            store,
            stop: Some(stop),
            worker: Some(tokio::spawn(future)),
        })
    }

    pub fn service(&self) -> Arc<dyn ImageServiceApi<Principal = P>> {
        self.service.clone()
    }
    pub fn owner(&self) -> &ImageOwner {
        &self.owner
    }

    pub fn now_ms(&self) -> i64 {
        state::NOW_MS
    }
    pub fn config(&self) -> ImageConfig {
        self.config.clone()
    }

    /// Admits approved encoded fixture pixels unchanged and returns their exact catalog pin.
    pub async fn import_source(
        &self,
        bytes: Vec<u8>,
        audience: Audience,
    ) -> Result<SourceRef, FixtureError> {
        self.store.import_source(bytes, audience)
    }

    /// Reads an imported or actually published asset; unknown ids never create bytes.
    pub async fn read_asset(&self, asset: AssetId) -> Result<Vec<u8>, FixtureError> {
        self.store.read_asset(asset)
    }

    /// Stops and joins this fixture's worker, including its current owned execution.
    pub async fn shutdown(mut self) -> Result<(), FixtureError> {
        let stopped = match self.stop.take() {
            Some(stop) => stop.shutdown().await,
            None => return Err(FixtureError::Worker),
        };
        let joined = match self.worker.take() {
            Some(worker) => worker.await.map_err(|_| FixtureError::Worker)?,
            None => return Err(FixtureError::Worker),
        };
        stopped.map_err(FixtureError::Runtime)?;
        joined.map_err(FixtureError::Runtime)
    }
}
impl<P: Sync> Drop for TransformFixture<P> {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

#[cfg(test)]
mod tests;
