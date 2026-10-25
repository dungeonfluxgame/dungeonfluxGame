//! Load-time validation of a parsed [`ServerConfig`]. Every refusal is a typed variant naming the
//! offending key, never a fallback to a default (hosted startup must not guess).

use std::{fmt, io, net::IpAddr, path::Path, path::PathBuf};

use super::{Environment, ProvidersMode, Role, ServerConfig, TlsMode};
use crate::deployment::InstanceIdRefusal;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Unreadable {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    Parse {
        message: String,
    },
    InvalidInstanceId {
        reason: &'static str,
    },
    /// A key or value that only local development may use, in a hosted environment.
    LocalOnlyKey {
        key: &'static str,
    },
    /// `environment = "local"` or `[local_dev]` in a build without the `local-dev` feature.
    LocalDevNotCompiled,
    RoleRequires {
        role: Role,
        key: &'static str,
    },
    RoleForbids {
        role: Role,
        key: &'static str,
    },
    PlaintextToRemoteHost,
    TlsRequires {
        key: &'static str,
    },
    LiveProvidersNeedAllowLive,
    NotAbsolute {
        key: &'static str,
    },
    Empty {
        key: &'static str,
    },
    OutOfRange {
        key: &'static str,
        minimum: u64,
        maximum: u64,
    },
    Inconsistent {
        rule: &'static str,
    },
    ReferencedFileUnreadable {
        key: &'static str,
    },
    /// A string under a secret-looking name, in a section another owner validates.
    InlineSecret {
        section: &'static str,
        key: String,
    },
    /// `/metrics` is unauthenticated, so a hosted process must not bind it to every interface.
    WildcardMetricsAddress,
    /// The effective settings could not be rendered.
    ListingFailed {
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, kind } => {
                write!(formatter, "cannot read {}: {kind}", path.display())
            }
            Self::Parse { message } => write!(formatter, "configuration does not parse: {message}"),
            Self::InvalidInstanceId { reason } => {
                write!(formatter, "instance.instance_id is invalid: {reason}")
            }
            Self::LocalOnlyKey { key } => write!(formatter, "{key} is for local development only"),
            Self::LocalDevNotCompiled => {
                write!(
                    formatter,
                    "this build has no local-dev feature; local keys are refused"
                )
            }
            Self::RoleRequires { role, key } => write!(formatter, "role {role:?} requires {key}"),
            Self::RoleForbids { role, key } => write!(formatter, "role {role:?} forbids {key}"),
            Self::PlaintextToRemoteHost => write!(
                formatter,
                "database.tls.mode loopback-plaintext needs a literal loopback address or a socket path"
            ),
            Self::TlsRequires { key } => write!(formatter, "database.tls requires {key}"),
            Self::LiveProvidersNeedAllowLive => {
                write!(
                    formatter,
                    "live providers in staging need providers.allow_live = true"
                )
            }
            Self::NotAbsolute { key } => write!(formatter, "{key} must be an absolute path"),
            Self::Empty { key } => write!(formatter, "{key} must not be empty"),
            Self::OutOfRange {
                key,
                minimum,
                maximum,
            } => {
                write!(formatter, "{key} must be between {minimum} and {maximum}")
            }
            Self::Inconsistent { rule } => write!(formatter, "inconsistent settings: {rule}"),
            Self::ReferencedFileUnreadable { key } => {
                write!(
                    formatter,
                    "the file named by {key} is missing, empty or too large"
                )
            }
            Self::InlineSecret { section, key } => write!(
                formatter,
                "[{section}] {key} looks like an inline secret; name a file with a _file key"
            ),
            Self::WildcardMetricsAddress => write!(
                formatter,
                "listen.metrics_address must name one interface in a hosted environment"
            ),
            Self::ListingFailed { message } => {
                write!(
                    formatter,
                    "the effective settings cannot be listed: {message}"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

fn in_range(key: &'static str, value: u64, minimum: u64, maximum: u64) -> Result<(), ConfigError> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::OutOfRange {
            key,
            minimum,
            maximum,
        })
    }
}

fn not_empty(key: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        Err(ConfigError::Empty { key })
    } else {
        Ok(())
    }
}

/// A path is absolute on the host or, for container paths, begins with a slash.
fn absolute(key: &'static str, path: &Path) -> Result<(), ConfigError> {
    if path.is_absolute() || path.to_string_lossy().starts_with('/') {
        Ok(())
    } else {
        Err(ConfigError::NotAbsolute { key })
    }
}

/// Plaintext is allowed only to a literal loopback IP (no name resolution) or a socket path.
fn is_loopback_literal_or_socket(host: &str) -> bool {
    host.starts_with('/')
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

impl ServerConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        self.instance_id()
            .map_err(|refusal| ConfigError::InvalidInstanceId {
                reason: match refusal {
                    InstanceIdRefusal::Empty => "empty",
                    InstanceIdRefusal::TooLong => "longer than 64 bytes",
                    InstanceIdRefusal::InvalidCharacter => {
                        "letters, digits, underscore and hyphen only"
                    }
                },
            })?;
        self.validate_environment()?;
        self.validate_role()?;
        self.validate_database()?;
        self.validate_telemetry()?;
        self.validate_runtime()?;
        self.validate_owned_sections()
    }

    fn validate_environment(&self) -> Result<(), ConfigError> {
        let environment = self.instance.environment;
        let local_dev_compiled = cfg!(feature = "local-dev");
        if (environment == Environment::Local || self.local_dev.is_some()) && !local_dev_compiled {
            return Err(ConfigError::LocalDevNotCompiled);
        }
        if environment.is_hosted() {
            if self.local_dev.is_some() {
                return Err(ConfigError::LocalOnlyKey { key: "local_dev" });
            }
            if self.database.tls.mode == TlsMode::LoopbackPlaintext {
                return Err(ConfigError::LocalOnlyKey {
                    key: "database.tls.mode",
                });
            }
            if self.listen.metrics_address.ip().is_unspecified() {
                return Err(ConfigError::WildcardMetricsAddress);
            }
            absolute("paths.temp_directory", &self.paths.temp_directory)?;
            absolute("telemetry.spool_directory", &self.telemetry.spool_directory)?;
            absolute("database.password_file", &self.database.password_file)?;
        }
        Ok(())
    }

    fn validate_role(&self) -> Result<(), ConfigError> {
        let role = self.instance.role;
        match (role, self.listen.public_address.is_some()) {
            (Role::ControlPlane, false) => {
                return Err(ConfigError::RoleRequires {
                    role,
                    key: "listen.public_address",
                });
            }
            (Role::CampaignRuntime | Role::Egress, true) => {
                return Err(ConfigError::RoleForbids {
                    role,
                    key: "listen.public_address",
                });
            }
            _ => {}
        }
        match (role, &self.providers) {
            (Role::Egress, None) => Err(ConfigError::RoleRequires {
                role,
                key: "providers",
            }),
            (Role::ControlPlane | Role::CampaignRuntime, Some(_)) => {
                Err(ConfigError::RoleForbids {
                    role,
                    key: "providers",
                })
            }
            (Role::Egress, Some(providers)) => {
                let environment = self.instance.environment;
                if environment == Environment::Production && providers.mode == ProvidersMode::Fake {
                    return Err(ConfigError::LocalOnlyKey {
                        key: "providers.mode",
                    });
                }
                if environment == Environment::Staging
                    && providers.mode == ProvidersMode::Live
                    && !providers.allow_live
                {
                    return Err(ConfigError::LiveProvidersNeedAllowLive);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn validate_database(&self) -> Result<(), ConfigError> {
        let database = &self.database;
        not_empty("database.host", &database.host)?;
        not_empty("database.name", &database.name)?;
        not_empty("database.user", &database.user)?;
        in_range("database.port", u64::from(database.port), 1, 65_535)?;
        in_range("database.pool_size", u64::from(database.pool_size), 1, 64)?;
        in_range(
            "database.statement_timeout_ms",
            u64::from(database.statement_timeout_ms),
            1,
            600_000,
        )?;
        match database.tls.mode {
            TlsMode::VerifyFull => {
                let ca_file = database
                    .tls
                    .ca_file
                    .as_deref()
                    .ok_or(ConfigError::TlsRequires { key: "ca_file" })?;
                if self.instance.environment.is_hosted() {
                    absolute("database.tls.ca_file", ca_file)?;
                }
                let server_name = database
                    .tls
                    .server_name
                    .as_deref()
                    .ok_or(ConfigError::TlsRequires { key: "server_name" })?;
                not_empty("database.tls.server_name", server_name)
            }
            TlsMode::LoopbackPlaintext => {
                if !is_loopback_literal_or_socket(&database.host) {
                    return Err(ConfigError::PlaintextToRemoteHost);
                }
                if database.tls.ca_file.is_some() || database.tls.server_name.is_some() {
                    return Err(ConfigError::Inconsistent {
                        rule: "loopback-plaintext names no ca_file or server_name",
                    });
                }
                Ok(())
            }
        }
    }

    fn validate_telemetry(&self) -> Result<(), ConfigError> {
        let telemetry = &self.telemetry;
        let endpoint = &telemetry.otlp_endpoint;
        let hosted = self.instance.environment.is_hosted();
        let scheme_ok =
            endpoint.starts_with("https://") || (!hosted && endpoint.starts_with("http://"));
        if !scheme_ok {
            return Err(ConfigError::Inconsistent {
                rule: "telemetry.otlp_endpoint is an https URL (http only for a local run)",
            });
        }
        let authority = endpoint
            .split_once("://")
            .map_or("", |(_, rest)| rest.split('/').next().unwrap_or_default());
        if authority.contains('@') {
            return Err(ConfigError::Inconsistent {
                rule: "telemetry.otlp_endpoint carries no credentials",
            });
        }
        not_empty("telemetry.service_name", &telemetry.service_name)?;
        in_range(
            "telemetry.spool_bytes_maximum",
            telemetry.spool_bytes_maximum,
            1 << 20,
            1 << 40,
        )?;
        in_range(
            "telemetry.push_interval_seconds",
            u64::from(telemetry.push_interval_seconds),
            1,
            3600,
        )
    }

    /// The sections other owners validate must still not carry a secret inline.
    fn validate_owned_sections(&self) -> Result<(), ConfigError> {
        for (section, table) in [
            ("objects", &self.objects),
            ("identity", &self.identity),
            ("payments", &self.payments),
        ] {
            if let Some(key) = table.as_ref().and_then(inline_secret) {
                return Err(ConfigError::InlineSecret { section, key });
            }
        }
        Ok(())
    }

    fn validate_runtime(&self) -> Result<(), ConfigError> {
        self.validate_runtime_ranges()?;
        self.validate_runtime_relations()
    }

    fn validate_runtime_ranges(&self) -> Result<(), ConfigError> {
        let runtime = &self.runtime;
        let (lease, meter, presence) = (&runtime.lease, &runtime.meter, &runtime.presence);
        let (wait, owner, drain) = (&runtime.blocking_wait, &runtime.owner, &runtime.drain);
        let within =
            |key, value: u32, minimum, maximum| in_range(key, u64::from(value), minimum, maximum);
        within(
            "runtime.maximum_seats",
            u32::from(runtime.maximum_seats),
            1,
            16,
        )?;
        within("runtime.lease.seconds", lease.seconds, 1, 3600)?;
        within(
            "runtime.lease.renew_every_seconds",
            lease.renew_every_seconds,
            1,
            3600,
        )?;
        within(
            "runtime.lease.safety_margin_seconds",
            lease.safety_margin_seconds,
            0,
            3600,
        )?;
        within(
            "runtime.lease.placing_seconds",
            lease.placing_seconds,
            1,
            3600,
        )?;
        within(
            "runtime.meter.segment_roll_seconds",
            meter.segment_roll_seconds,
            1,
            3600,
        )?;
        within(
            "runtime.meter.backdate_window_seconds",
            meter.backdate_window_seconds,
            1,
            3600,
        )?;
        within("runtime.presence.window_ms", presence.window_ms, 1, 600_000)?;
        within(
            "runtime.presence.idle_release_seconds",
            presence.idle_release_seconds,
            1,
            86_400,
        )?;
        within(
            "runtime.presence.away_end_hours",
            presence.away_end_hours,
            1,
            168,
        )?;
        within(
            "runtime.blocking_wait.gated_grace_ms",
            wait.gated_grace_ms,
            0,
            600_000,
        )?;
        within(
            "runtime.blocking_wait.outage_grace_ms",
            wait.outage_grace_ms,
            0,
            600_000,
        )?;
        within("runtime.owner.stall_seconds", owner.stall_seconds, 1, 3600)?;
        within(
            "runtime.owner.stall_exit_seconds",
            owner.stall_exit_seconds,
            1,
            3600,
        )?;
        within(
            "runtime.money.reconstruction_deadline_seconds",
            runtime.money.reconstruction_deadline_seconds,
            1,
            600,
        )?;
        within(
            "runtime.drain.effect_wait_seconds",
            drain.effect_wait_seconds,
            1,
            600,
        )?;
        within(
            "runtime.drain.maximum_seconds",
            drain.maximum_seconds,
            1,
            600,
        )?;
        within("runtime.drain.edge_seconds", drain.edge_seconds, 1, 600)
    }

    fn validate_runtime_relations(&self) -> Result<(), ConfigError> {
        let runtime = &self.runtime;
        let (lease, meter, presence) = (&runtime.lease, &runtime.meter, &runtime.presence);
        let (wait, owner, drain) = (&runtime.blocking_wait, &runtime.owner, &runtime.drain);
        if lease.renew_every_seconds + lease.safety_margin_seconds >= lease.seconds {
            return Err(ConfigError::Inconsistent {
                rule: "a renewal plus the safety margin fits inside the lease",
            });
        }
        if owner.stall_seconds >= owner.stall_exit_seconds {
            return Err(ConfigError::Inconsistent {
                rule: "the stall bound is shorter than the stall exit",
            });
        }
        if drain.effect_wait_seconds >= drain.maximum_seconds {
            return Err(ConfigError::Inconsistent {
                rule: "the effect wait is shorter than the maximum drain",
            });
        }
        if u64::from(presence.window_ms) >= u64::from(presence.idle_release_seconds) * 1000 {
            return Err(ConfigError::Inconsistent {
                rule: "the presence window is shorter than the idle release",
            });
        }
        let longest_stop_ms = presence.window_ms.max(wait.gated_grace_ms);
        let minimum_backdate =
            u64::from(longest_stop_ms.div_ceil(1000)) + u64::from(lease.renew_every_seconds);
        if u64::from(meter.backdate_window_seconds) < minimum_backdate {
            return Err(ConfigError::Inconsistent {
                rule: "the back-date window covers the longest stop grace plus one renewal",
            });
        }
        Ok(())
    }
}

/// The first key of an owner-validated section that looks like an inline secret: a string
/// value under a name that mentions a secret, password, token or key and does not end in `_file`.
fn inline_secret(table: &toml::Table) -> Option<String> {
    table.iter().find_map(|(key, value)| match value {
        toml::Value::String(_) if looks_secret(key) => Some(key.clone()),
        toml::Value::Table(nested) => inline_secret(nested),
        toml::Value::Array(items) => items.iter().find_map(|item| match item {
            toml::Value::Table(nested) => inline_secret(nested),
            toml::Value::String(_) if looks_secret(key) => Some(key.clone()),
            _ => None,
        }),
        _ => None,
    })
}

fn looks_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    !key.ends_with("_file")
        && (key.ends_with("_key")
            || ["secret", "password", "token", "credential", "private"]
                .iter()
                .any(|word| key.contains(word)))
}
