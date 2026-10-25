//! Parses deployment settings without exposing credential values in diagnostics.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use df_customer::{HostConfig, LinkStyle, MailIdentity, PublicOrigin};
use df_persistence::postgres_tls::{PostgresTransport, check_transport};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Development,
    Demo,
    Production,
}

pub(crate) enum Identity {
    Local,
    WorkOs { client_id: String, api_key: String },
}

pub(crate) enum Mail {
    Outbox(PathBuf),
    Resend(String),
}

/// Validated native configuration. Deliberately has no Debug implementation.
pub struct Config {
    pub(crate) profile: Profile,
    pub(crate) listen: SocketAddr,
    pub(crate) origin: String,
    pub(crate) bundle: PathBuf,
    pub(crate) database: tokio_postgres::Config,
    pub(crate) transport: PostgresTransport,
    pub(crate) migrate_database: Option<tokio_postgres::Config>,
    pub(crate) hosts: HostConfig,
    pub(crate) identity: Identity,
    pub(crate) mail: Mail,
    pub(crate) proxies: df_portal::TrustedProxies,
    pub(crate) trial_email_key: [u8; 32],
    pub(crate) campaign_keys: df_campaign::service::ServiceKeys,
    pub(crate) stripe: Option<crate::checkout::Settings>,
}

/// Configuration failures identify names only; they never retain an environment value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid(&'static str),
    DevelopmentOnly(&'static str),
    PaymentsNotConfigured,
}

fn present(get: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    get(key).filter(|value| !value.trim().is_empty())
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    key: &'static str,
) -> Result<String, ConfigError> {
    present(get, key).ok_or(ConfigError::Missing(key))
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::parse(|key| std::env::var(key).ok())
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let profile = match required(&get, "DF_PROFILE")?.as_str() {
            "development" => Profile::Development,
            "demo" => Profile::Demo,
            "production" => Profile::Production,
            _ => return Err(ConfigError::Invalid("DF_PROFILE")),
        };
        reject_development_ports(&get, profile)?;
        let stripe = crate::checkout::Settings::parse(&get)?;
        let listen: SocketAddr = present(&get, "DF_PORTAL_LISTEN")
            .unwrap_or_else(|| "127.0.0.1:8099".to_owned())
            .parse()
            .map_err(|_| ConfigError::Invalid("DF_PORTAL_LISTEN"))?;
        if !listen.ip().is_loopback() || listen.port() == 0 || listen.port() == 63309 {
            return Err(ConfigError::Invalid("DF_PORTAL_LISTEN"));
        }
        let origin = required(&get, "DF_GAMEPLAY_PUBLIC_URL")?;
        let public_origin = PublicOrigin::parse(&origin)
            .map_err(|_| ConfigError::Invalid("DF_GAMEPLAY_PUBLIC_URL"))?;
        if profile != Profile::Development && !origin.starts_with("https://") {
            return Err(ConfigError::Invalid("DF_GAMEPLAY_PUBLIC_URL"));
        }
        let hosts = HostConfig::new(
            public_origin,
            MailIdentity::new(
                "DungeonFlux",
                &present(&get, "DF_MAIL_FROM")
                    .unwrap_or_else(|| "accounts@mail.dungeonfluxdnd.com".to_owned()),
                &present(&get, "DF_MAIL_REPLY_TO")
                    .unwrap_or_else(|| "support@dungeonfluxdnd.com".to_owned()),
            )
            .map_err(|_| ConfigError::Invalid("DF_MAIL_FROM/DF_MAIL_REPLY_TO"))?,
            LinkStyle::App,
        );
        let campaign_keys = df_campaign::service::ServiceKeys::from_hex(
            &required(&get, "DF_CAMPAIGN_IP_KEY")?,
            &required(&get, "DF_CAMPAIGN_CODE_KEY")?,
        )
        .map_err(|_| ConfigError::Invalid("DF_CAMPAIGN_IP_KEY/DF_CAMPAIGN_CODE_KEY"))?;
        let trial_email_key =
            df_campaign::secrets::unhex(&required(&get, "DF_TRIAL_EMAIL_HMAC_KEY")?)
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .filter(|key| {
                    key != &[0; 32]
                        && key != campaign_keys.ip_key()
                        && key != campaign_keys.code_key()
                })
                .ok_or(ConfigError::Invalid("DF_TRIAL_EMAIL_HMAC_KEY"))?;
        let identity = identity(&get, profile)?;
        if let Some(selected) = present(&get, "DF_GAMEPLAY_IDENTITY") {
            let agrees = matches!(
                (&identity, selected.as_str()),
                (Identity::Local, "local")
                    | (Identity::Local, "local-passwords")
                    | (Identity::Local, "local-password")
                    | (Identity::WorkOs { .. }, "workos")
            );
            if !agrees {
                return Err(ConfigError::Invalid("DF_GAMEPLAY_IDENTITY"));
            }
        }
        if let Some(selected) = present(&get, "DF_GAMEPLAY_MAIL_MODE") {
            let configured = present(&get, "DF_RESEND_API_KEY").is_some();
            if !matches!(
                (configured, selected.as_str()),
                (true, "real") | (false, "outbox") | (false, "dev") | (false, "dev-outbox")
            ) {
                return Err(ConfigError::Invalid("DF_GAMEPLAY_MAIL_MODE"));
            }
        }
        let mail = match present(&get, "DF_RESEND_API_KEY") {
            Some(key) => Mail::Resend(key),
            None if profile == Profile::Development => Mail::Outbox(PathBuf::from(
                present(&get, "DF_PORTAL_OUTBOX")
                    .unwrap_or_else(|| "artifacts/tmp/hosted/outbox".to_owned()),
            )),
            None => return Err(ConfigError::Missing("DF_RESEND_API_KEY")),
        };
        let mut database: tokio_postgres::Config = required(&get, "DF_PORTAL_DATABASE_URL")?
            .parse()
            .map_err(|_| ConfigError::Invalid("DF_PORTAL_DATABASE_URL"))?;
        database.connect_timeout(Duration::from_secs(5));
        let transport = match required(&get, "DF_GAMEPLAY_DATABASE_TLS")?.as_str() {
            "loopback-plaintext" if profile != Profile::Production => {
                PostgresTransport::PlaintextLoopbackOnly
            }
            "verify-full" => PostgresTransport::VerifiedTls {
                ca_file: PathBuf::from(required(&get, "DF_PORTAL_DATABASE_CA_FILE")?),
            },
            _ => return Err(ConfigError::Invalid("DF_GAMEPLAY_DATABASE_TLS")),
        };
        check_transport(&database, &transport)
            .map_err(|_| ConfigError::Invalid("DF_GAMEPLAY_DATABASE_TLS"))?;
        let migrate_database = present(&get, "DF_PORTAL_MIGRATOR_DATABASE_URL")
            .map(|value| value.parse::<tokio_postgres::Config>())
            .transpose()
            .map_err(|_| ConfigError::Invalid("DF_PORTAL_MIGRATOR_DATABASE_URL"))?;
        if let Some(migrator) = &migrate_database {
            if migrator.get_hosts() != database.get_hosts()
                || migrator.get_ports() != database.get_ports()
                || migrator.get_dbname() != database.get_dbname()
            {
                return Err(ConfigError::Invalid("DF_PORTAL_MIGRATOR_DATABASE_URL"));
            }
            check_transport(migrator, &transport)
                .map_err(|_| ConfigError::Invalid("DF_GAMEPLAY_DATABASE_TLS"))?;
        }
        let networks = present(&get, "DF_PORTAL_TRUSTED_PROXIES").unwrap_or_else(|| {
            if profile == Profile::Development {
                String::new()
            } else {
                "127.0.0.1/32,::1/128".to_owned()
            }
        });
        let networks: Vec<&str> = networks
            .split(',')
            .filter(|network| !network.is_empty())
            .collect();
        let proxies = df_portal::TrustedProxies::parse(&networks)
            .map_err(|_| ConfigError::Invalid("DF_PORTAL_TRUSTED_PROXIES"))?;
        Ok(Self {
            profile,
            listen,
            origin,
            bundle: PathBuf::from(required(&get, "DF_PORTAL_BUNDLE_DIR")?),
            database,
            transport,
            migrate_database,
            hosts,
            identity,
            mail,
            campaign_keys,
            trial_email_key,
            proxies,
            stripe,
        })
    }
}

fn identity(
    get: &impl Fn(&str) -> Option<String>,
    profile: Profile,
) -> Result<Identity, ConfigError> {
    match (
        present(get, "DF_WORKOS_CLIENT_ID"),
        present(get, "DF_WORKOS_API_KEY"),
    ) {
        (Some(client_id), Some(api_key)) => Ok(Identity::WorkOs { client_id, api_key }),
        (None, None) if profile == Profile::Development => Ok(Identity::Local),
        (None, None) => Err(ConfigError::Missing(
            "DF_WORKOS_CLIENT_ID/DF_WORKOS_API_KEY",
        )),
        _ => Err(ConfigError::Invalid(
            "DF_WORKOS_CLIENT_ID/DF_WORKOS_API_KEY",
        )),
    }
}

fn reject_development_ports(
    get: &impl Fn(&str) -> Option<String>,
    profile: Profile,
) -> Result<(), ConfigError> {
    if let Some(fixture) = present(get, "DF_FIXTURE_BUILD") {
        let allowed = matches!(fixture.as_str(), "off" | "false" | "0")
            || (profile == Profile::Demo
                && fixture == "demo-exception:df-transport-fixture --gameplay-demo")
            || profile == Profile::Development;
        if !allowed {
            return Err(ConfigError::DevelopmentOnly("DF_FIXTURE_BUILD"));
        }
    }
    if profile == Profile::Development {
        return Ok(());
    }
    for name in [
        "DF_DEV_ACCOUNTS",
        "DF_GAMEPLAY_DEV_CONTROL",
        "DF_GAMEPLAY_DEV_CONTROL_PUBLIC",
        "DF_GAMEPLAY_DEV_OVERRIDES",
        "DF_DEV_CONTROL",
        "DF_DEVCTL_ENABLED",
    ] {
        if present(get, name).is_some_and(|value| !matches!(value.as_str(), "off" | "false" | "0"))
        {
            return Err(ConfigError::DevelopmentOnly(name));
        }
    }
    if present(get, "DF_PORTAL_OUTBOX").is_some() {
        return Err(ConfigError::DevelopmentOnly("DF_PORTAL_OUTBOX"));
    }
    for name in ["DF_PORTAL_FAKE_PORTS", "DF_PORTAL_FAKE_CHECKOUT"] {
        if present(get, name).is_some_and(|value| !matches!(value.as_str(), "off" | "false" | "0"))
        {
            return Err(ConfigError::DevelopmentOnly(name));
        }
    }
    if present(get, "DF_GAMEPLAY_PROVIDER_MODE").is_some_and(|value| value.starts_with("fake")) {
        return Err(ConfigError::DevelopmentOnly("DF_GAMEPLAY_PROVIDER_MODE"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
