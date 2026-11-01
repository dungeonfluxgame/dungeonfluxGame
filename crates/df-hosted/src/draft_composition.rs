//! Configured, read-only private delivery shares the writer's catalog and object root.

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::Router;
use df_assets::{DeliveryBudget, DraftDeliveryLimits, NativeFileStore};
use df_auth::web_session_policy::SessionLifetimes;
use df_customer::{AccountStore, CustomerService, EmailSender};
use df_persistence::{media_catalog::PostgresMediaCatalog, postgres_tls::PostgresTransport};

use crate::{config::ConfigError, database::Connection};

pub(crate) enum Settings {
    Disabled,
    Enabled(Box<Enabled>),
}

pub(crate) struct Enabled {
    pub(crate) object_root: PathBuf,
    pub(crate) catalog: tokio_postgres::Config,
    pub(crate) disclosure: tokio_postgres::Config,
    pub(crate) maximum_image_bytes: u64,
    pub(crate) maximum_delivery_bytes: u64,
    pub(crate) operation_timeout: Duration,
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    key: &'static str,
) -> Result<String, ConfigError> {
    get(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn positive(get: &impl Fn(&str) -> Option<String>, key: &'static str) -> Result<u64, ConfigError> {
    required(get, key)?
        .parse()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(ConfigError::Invalid(key))
}

fn database(
    get: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    accounts: &tokio_postgres::Config,
    transport: &PostgresTransport,
) -> Result<tokio_postgres::Config, ConfigError> {
    let mut database: tokio_postgres::Config = required(get, key)?
        .parse()
        .map_err(|_| ConfigError::Invalid(key))?;
    // One database owns both account authentication and the final disclosure snapshot.
    if database.get_hosts() != accounts.get_hosts()
        || database.get_hostaddrs() != accounts.get_hostaddrs()
        || database.get_ports() != accounts.get_ports()
        || database.get_dbname() != accounts.get_dbname()
        || df_persistence::postgres_tls::check_transport(&database, transport).is_err()
    {
        return Err(ConfigError::Invalid(key));
    }
    database.connect_timeout(Duration::from_secs(5));
    Ok(database)
}

impl Settings {
    pub(crate) fn parse(
        get: &impl Fn(&str) -> Option<String>,
        accounts: &tokio_postgres::Config,
        transport: &PostgresTransport,
    ) -> Result<Self, ConfigError> {
        match get("DF_DRAFT_DELIVERY_ENABLED").as_deref() {
            None | Some("false") => return Ok(Self::Disabled),
            Some("true") => {}
            Some(_) => return Err(ConfigError::Invalid("DF_DRAFT_DELIVERY_ENABLED")),
        }
        let object_root = PathBuf::from(required(get, "DF_MEDIA_OBJECT_ROOT")?);
        if !object_root.is_absolute() {
            return Err(ConfigError::Invalid("DF_MEDIA_OBJECT_ROOT"));
        }
        let catalog = database(get, "DF_MEDIA_CATALOG_DATABASE_URL", accounts, transport)?;
        let disclosure = database(get, "DF_DRAFT_DISCLOSURE_DATABASE_URL", accounts, transport)?;
        if catalog.get_user() == disclosure.get_user() {
            return Err(ConfigError::Invalid("DF_DRAFT_DISCLOSURE_DATABASE_URL"));
        }
        let maximum_image_bytes = positive(get, "DF_DRAFT_MAX_IMAGE_BYTES")?;
        let maximum_delivery_bytes = positive(get, "DF_DRAFT_MAX_DELIVERY_BYTES")?;
        if maximum_delivery_bytes < maximum_image_bytes
            || isize::try_from(maximum_delivery_bytes).is_err()
        {
            return Err(ConfigError::Invalid("DF_DRAFT_MAX_DELIVERY_BYTES"));
        }
        let operation_timeout =
            Duration::from_millis(positive(get, "DF_DRAFT_OPERATION_TIMEOUT_MS")?);
        if std::time::Instant::now()
            .checked_add(operation_timeout)
            .is_none()
        {
            return Err(ConfigError::Invalid("DF_DRAFT_OPERATION_TIMEOUT_MS"));
        }
        Ok(Self::Enabled(Box::new(Enabled {
            object_root,
            catalog,
            disclosure,
            maximum_image_bytes,
            maximum_delivery_bytes,
            operation_timeout,
        })))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenError {
    Objects,
    Catalog,
    Disclosure,
}

pub(crate) async fn open<S: AccountStore, M: EmailSender>(
    settings: Settings,
    transport: &PostgresTransport,
    sessions: SessionLifetimes,
    service: CustomerService<S, M>,
) -> Result<Router, OpenError> {
    let Settings::Enabled(settings) = settings else {
        df_observe::record_audit_line(
            "hosted.draft_delivery",
            "HOSTED draft_delivery state=disabled".to_owned(),
        );
        return Ok(df_portal::draft_images::router(
            service,
            Arc::new(Disabled),
            df_portal::draft_images::DraftImagePolicy::default(),
        ));
    };
    let catalog = PostgresMediaCatalog::open_existing(settings.catalog, transport.clone(), 2)
        .await
        .map_err(|_| OpenError::Catalog)?;
    let disclosure = Connection::open(&settings.disclosure, transport)
        .await
        .map_err(|_| OpenError::Disclosure)?;
    crate::draft_images::check_disclosure_reader(&disclosure)
        .await
        .map_err(|_| OpenError::Disclosure)?;
    let maximum_image_bytes = settings.maximum_image_bytes;
    let object_root = settings.object_root;
    // Never create an empty alternate store when the configured writer volume is absent.
    let objects = tokio::task::spawn_blocking(move || {
        if !object_root.is_dir() {
            return Err(OpenError::Objects);
        }
        NativeFileStore::new(object_root, maximum_image_bytes).map_err(|_| OpenError::Objects)
    })
    .await
    .map_err(|_| OpenError::Objects)??;
    let concurrent = usize::try_from(settings.maximum_delivery_bytes / maximum_image_bytes)
        .map_err(|_| OpenError::Objects)?;
    let adapter = crate::draft_images::production_adapter(
        catalog,
        objects,
        disclosure,
        sessions,
        DraftDeliveryLimits {
            max_image_bytes: maximum_image_bytes,
        },
        DeliveryBudget::new(concurrent),
        settings.operation_timeout,
    );
    df_observe::record_audit_line(
        "hosted.draft_delivery",
        format!("HOSTED draft_delivery state=enabled slots={concurrent}"),
    );
    Ok(crate::draft_images::router(service, Arc::new(adapter)))
}

struct Disabled;

impl df_portal::draft_images::DraftImageSource for Disabled {
    async fn buffer_candidate(
        &self,
        _owner: df_types::AccountId,
        _asset: df_assets::AssetId,
        _variant: df_assets::DraftImageVariant,
        _deadline: df_assets::Deadline,
    ) -> Result<df_assets::DraftImageCandidate, df_assets::DraftDeliveryError> {
        Err(df_assets::DraftDeliveryError::Unavailable)
    }

    async fn authorize_release(
        &self,
        _owner: df_types::AccountId,
        _session: df_types::WebSessionId,
        _basis: df_assets::DeliveryBasis,
        _deadline: df_assets::Deadline,
    ) -> Result<(), df_assets::DraftDeliveryError> {
        Err(df_assets::DraftDeliveryError::Unavailable)
    }
}

#[cfg(test)]
#[path = "draft_composition_tests.rs"]
mod tests;
