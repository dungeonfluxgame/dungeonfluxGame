use std::{collections::BTreeSet, io::Write as _};

use crate::config::{
    ConfigError, ProvidersMode, Role, ServerConfig, flatten, redact_owned_sections,
};

#[cfg(feature = "local-dev")]
use crate::config::{Environment, TlsMode};

const CONTROL_PLANE: &str = r#"
[instance]
role = "control-plane"
environment = "production"
instance_id = "control-a"

[listen]
public_address = "0.0.0.0:8443"
internal_address = "10.0.0.5:9443"
metrics_address = "10.0.0.5:9100"

[database]
host = "db.internal"
port = 25060
name = "dungeonflux"
user = "df_control"
password_file = "/run/secrets/database_password"

[database.tls]
mode = "verify-full"
ca_file = "/etc/ssl/database-ca.pem"
server_name = "db.internal"

[paths]
temp_directory = "/tmp"

[telemetry]
otlp_endpoint = "https://127.0.0.1:4317"
service_name = "df-control-plane"
spool_directory = "/var/lib/dungeonflux/telemetry"
"#;

const RUNTIME: &str = r#"
[instance]
role = "campaign-runtime"
environment = "production"
instance_id = "runtime-a"

[listen]
internal_address = "10.0.0.6:9443"
metrics_address = "10.0.0.6:9100"

[database]
host = "db.internal"
port = 25060
name = "dungeonflux"
user = "df_campaign_runtime"
password_file = "/run/secrets/database_password"

[database.tls]
mode = "verify-full"
ca_file = "/etc/ssl/database-ca.pem"
server_name = "db.internal"

[paths]
temp_directory = "/tmp"

[telemetry]
otlp_endpoint = "https://127.0.0.1:4317"
service_name = "df-runtime"
spool_directory = "/var/lib/dungeonflux/spool"
"#;

const EGRESS: &str = r#"
[instance]
role = "egress"
environment = "production"
instance_id = "egress-a"

[listen]
internal_address = "10.0.0.7:9443"
metrics_address = "10.0.0.7:9100"

[database]
host = "db.internal"
port = 25060
name = "dungeonflux"
user = "df_egress"
password_file = "/run/secrets/database_password"

[database.tls]
mode = "verify-full"
ca_file = "/etc/ssl/database-ca.pem"
server_name = "db.internal"

[paths]
temp_directory = "/tmp"

[telemetry]
otlp_endpoint = "https://127.0.0.1:4317"
service_name = "df-egress"
spool_directory = "/var/lib/dungeonflux/telemetry"

[providers]
mode = "live"
"#;

const LOCAL_RUNTIME: &str = r#"
[instance]
role = "campaign-runtime"
environment = "local"
instance_id = "local-a"

[listen]
internal_address = "127.0.0.1:9443"
metrics_address = "127.0.0.1:9100"

[database]
host = "127.0.0.1"
port = 55911
name = "df_local"
user = "postgres"
password_file = "password.txt"

[database.tls]
mode = "loopback-plaintext"

[paths]
temp_directory = "tmp"

[telemetry]
otlp_endpoint = "http://127.0.0.1:4317"
service_name = "df-local"
spool_directory = "spool"
"#;

fn load(text: &str) -> Result<ServerConfig, ConfigError> {
    ServerConfig::from_toml_str(text)
}

fn with(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "the fixture has no `{from}`");
    text.replacen(from, to, 1)
}

#[test]
fn a_minimal_file_loads_with_the_proposed_defaults() {
    let config = load(CONTROL_PLANE).expect("valid");
    assert_eq!(config.instance.role, Role::ControlPlane);
    assert_eq!(config.runtime.lease.seconds, 30);
    assert_eq!(config.runtime.lease.renew_every_seconds, 10);
    assert_eq!(config.runtime.lease.safety_margin_seconds, 3);
    assert_eq!(config.runtime.lease.placing_seconds, 60);
    assert_eq!(config.runtime.meter.segment_roll_seconds, 60);
    assert_eq!(config.runtime.presence.window_ms, 30_000);
    assert_eq!(config.runtime.presence.idle_release_seconds, 900);
    assert_eq!(config.runtime.presence.away_end_hours, 12);
    assert_eq!(config.runtime.maximum_seats, 6);
    assert_eq!(config.database.pool_size, 4);
    assert_eq!(config.telemetry.spool_bytes_maximum, 256 * 1024 * 1024);
}

#[test]
fn an_unknown_key_is_refused_so_a_typo_cannot_silently_keep_a_default() {
    let typo = with(
        CONTROL_PLANE,
        "[paths]",
        "[runtime.lease]\nsecnds = 20\n\n[paths]",
    );
    assert!(matches!(load(&typo), Err(ConfigError::Parse { .. })));
}

#[test]
fn a_secret_cannot_be_written_inline() {
    let inline = with(
        CONTROL_PLANE,
        "port = 25060",
        "port = 25060\npassword = \"hunter2\"",
    );
    assert!(matches!(load(&inline), Err(ConfigError::Parse { .. })));
}

#[test]
fn an_invalid_instance_id_is_refused() {
    let bad = with(CONTROL_PLANE, "control-a", "control a!");
    assert!(matches!(
        load(&bad),
        Err(ConfigError::InvalidInstanceId { .. })
    ));
}

#[test]
fn staging_and_production_reject_every_local_only_key() {
    // A build without the feature cannot even name the local keys, which is stricter.
    let local_dev_refusal = if cfg!(feature = "local-dev") {
        ConfigError::LocalOnlyKey { key: "local_dev" }
    } else {
        ConfigError::LocalDevNotCompiled
    };
    let local_dev = format!("{CONTROL_PLANE}\n[local_dev]\ndev_control = true\n");
    assert_eq!(load(&local_dev), Err(local_dev_refusal.clone()));
    let plaintext = with(
        &with(
            CONTROL_PLANE,
            "mode = \"verify-full\"",
            "mode = \"loopback-plaintext\"",
        ),
        "host = \"db.internal\"",
        "host = \"127.0.0.1\"",
    );
    let plaintext = with(
        &with(&plaintext, "ca_file = \"/etc/ssl/database-ca.pem\"\n", ""),
        "server_name = \"db.internal\"\n",
        "",
    );
    assert_eq!(
        load(&plaintext),
        Err(ConfigError::LocalOnlyKey {
            key: "database.tls.mode"
        })
    );
    let staging = with(&local_dev, "production", "staging");
    assert_eq!(load(&staging), Err(local_dev_refusal));
}

#[cfg(not(feature = "local-dev"))]
#[test]
fn a_production_build_refuses_the_local_environment_and_every_local_section() {
    assert_eq!(load(LOCAL_RUNTIME), Err(ConfigError::LocalDevNotCompiled));
    let with_section = format!("{CONTROL_PLANE}\n[local_dev]\ndev_control = false\n");
    assert_eq!(load(&with_section), Err(ConfigError::LocalDevNotCompiled));
}

#[cfg(feature = "local-dev")]
#[test]
fn a_local_environment_accepts_local_keys() {
    let config = load(&format!(
        "{LOCAL_RUNTIME}\n[local_dev]\ndev_control = true\n"
    ))
    .expect("valid");
    assert_eq!(config.instance.environment, Environment::Local);
    assert_eq!(config.database.tls.mode, TlsMode::LoopbackPlaintext);
}

#[cfg(feature = "local-dev")]
#[test]
fn plaintext_needs_a_literal_loopback_address_or_a_socket_path() {
    for accepted in [
        "127.0.0.1",
        "127.8.9.10",
        "::1",
        "[::1]",
        "/var/run/postgresql",
    ] {
        let text = with(
            LOCAL_RUNTIME,
            "host = \"127.0.0.1\"",
            &format!("host = \"{accepted}\""),
        );
        assert!(load(&text).is_ok(), "{accepted}");
    }
    for refused in ["db.internal", "localhost", "10.0.0.5", "0.0.0.0"] {
        let text = with(
            LOCAL_RUNTIME,
            "host = \"127.0.0.1\"",
            &format!("host = \"{refused}\""),
        );
        assert_eq!(
            load(&text),
            Err(ConfigError::PlaintextToRemoteHost),
            "{refused}"
        );
    }
}

#[test]
fn verified_tls_needs_a_ca_file_and_a_server_name() {
    let no_ca = with(
        CONTROL_PLANE,
        "ca_file = \"/etc/ssl/database-ca.pem\"\n",
        "",
    );
    assert_eq!(
        load(&no_ca),
        Err(ConfigError::TlsRequires { key: "ca_file" })
    );
    let no_name = with(CONTROL_PLANE, "server_name = \"db.internal\"\n", "");
    assert_eq!(
        load(&no_name),
        Err(ConfigError::TlsRequires { key: "server_name" })
    );
}

#[test]
fn each_role_requires_and_forbids_its_own_sections() {
    let control_without_public = with(CONTROL_PLANE, "public_address = \"0.0.0.0:8443\"\n", "");
    assert_eq!(
        load(&control_without_public),
        Err(ConfigError::RoleRequires {
            role: Role::ControlPlane,
            key: "listen.public_address"
        })
    );
    let runtime_with_public = with(
        RUNTIME,
        "[listen]\n",
        "[listen]\npublic_address = \"0.0.0.0:8443\"\n",
    );
    assert_eq!(
        load(&runtime_with_public),
        Err(ConfigError::RoleForbids {
            role: Role::CampaignRuntime,
            key: "listen.public_address"
        })
    );
    let egress_without_providers = with(EGRESS, "[providers]\nmode = \"live\"\n", "");
    assert_eq!(
        load(&egress_without_providers),
        Err(ConfigError::RoleRequires {
            role: Role::Egress,
            key: "providers"
        })
    );
    let runtime_with_providers = format!("{RUNTIME}\n[providers]\nmode = \"live\"\n");
    assert_eq!(
        load(&runtime_with_providers),
        Err(ConfigError::RoleForbids {
            role: Role::CampaignRuntime,
            key: "providers"
        })
    );
}

#[test]
fn live_providers_in_staging_need_an_explicit_allowance_and_production_refuses_fakes() {
    let staging_live = with(EGRESS, "production", "staging");
    assert_eq!(
        load(&staging_live),
        Err(ConfigError::LiveProvidersNeedAllowLive)
    );
    let allowed = with(
        &staging_live,
        "mode = \"live\"",
        "mode = \"live\"\nallow_live = true",
    );
    assert!(load(&allowed).is_ok());
    let staging_fake = with(&staging_live, "mode = \"live\"", "mode = \"fake\"");
    assert_eq!(
        load(&staging_fake)
            .expect("fake is fine in staging")
            .providers
            .expect("providers")
            .mode,
        ProvidersMode::Fake
    );
    let production_fake = with(EGRESS, "mode = \"live\"", "mode = \"fake\"");
    assert_eq!(
        load(&production_fake),
        Err(ConfigError::LocalOnlyKey {
            key: "providers.mode"
        })
    );
}

#[test]
fn hosted_paths_must_be_absolute() {
    let relative = with(
        CONTROL_PLANE,
        "temp_directory = \"/tmp\"",
        "temp_directory = \"tmp\"",
    );
    assert_eq!(
        load(&relative),
        Err(ConfigError::NotAbsolute {
            key: "paths.temp_directory"
        })
    );
    let password = with(
        CONTROL_PLANE,
        "/run/secrets/database_password",
        "database_password",
    );
    assert_eq!(
        load(&password),
        Err(ConfigError::NotAbsolute {
            key: "database.password_file"
        })
    );
}

#[test]
fn out_of_range_values_name_their_key() {
    for (from, to, key) in [
        ("port = 25060", "port = 0", "database.port"),
        (
            "[paths]",
            "[runtime]\nmaximum_seats = 17\n\n[paths]",
            "runtime.maximum_seats",
        ),
        (
            "[paths]",
            "[runtime.lease]\nseconds = 0\n\n[paths]",
            "runtime.lease.seconds",
        ),
        (
            "[paths]",
            "[runtime.lease]\nseconds = 3601\n\n[paths]",
            "runtime.lease.seconds",
        ),
        (
            "[paths]",
            "[runtime.meter]\nsegment_roll_seconds = 0\n\n[paths]",
            "runtime.meter.segment_roll_seconds",
        ),
        (
            "port = 25060",
            "port = 25060\npool_size = 65",
            "database.pool_size",
        ),
    ] {
        let text = with(CONTROL_PLANE, from, to);
        assert!(
            matches!(load(&text), Err(ConfigError::OutOfRange { key: found, .. }) if found == key),
            "{key}"
        );
    }
}

#[test]
fn settings_that_contradict_each_other_are_refused() {
    for (section, rule) in [
        (
            "[runtime.lease]\nseconds = 12\nrenew_every_seconds = 10\n",
            "renewal",
        ),
        (
            "[runtime.owner]\nstall_seconds = 60\nstall_exit_seconds = 60\n",
            "stall",
        ),
        (
            "[runtime.drain]\neffect_wait_seconds = 90\nmaximum_seconds = 90\n",
            "drain",
        ),
        (
            "[runtime.presence]\nwindow_ms = 900000\nidle_release_seconds = 900\n",
            "presence",
        ),
        (
            "[runtime.meter]\nbackdate_window_seconds = 20\n",
            "back-date",
        ),
    ] {
        let text = with(CONTROL_PLANE, "[paths]", &format!("{section}\n[paths]"));
        assert!(
            matches!(
                load(&text),
                Err(ConfigError::Inconsistent { .. }) | Err(ConfigError::OutOfRange { .. })
            ),
            "{rule}"
        );
    }
}

#[test]
fn a_longer_stop_grace_needs_a_longer_backdate_window() {
    let text = with(
        CONTROL_PLANE,
        "[paths]",
        "[runtime.blocking_wait]\ngated_grace_ms = 40000\n\n[paths]",
    );
    assert!(matches!(load(&text), Err(ConfigError::Inconsistent { .. })));
    let widened = with(
        &text,
        "[paths]",
        "[runtime.meter]\nbackdate_window_seconds = 60\n\n[paths]",
    );
    assert!(load(&widened).is_ok());
}

#[test]
fn sections_owned_by_other_agents_pass_through_unread() {
    let text = format!(
        "{CONTROL_PLANE}\n[objects]\nbucket = \"media\"\n[identity]\nprovider = \"workos\"\n[payments]\nmode = \"test\"\n"
    );
    let config = load(&text).expect("valid");
    assert_eq!(
        config.objects.expect("objects")["bucket"].as_str(),
        Some("media")
    );
    assert!(config.identity.is_some() && config.payments.is_some());
}

#[test]
fn an_inline_secret_in_an_owned_section_is_refused_and_a_file_reference_is_not() {
    for (section, entry, key) in [
        ("objects", "media_secret_key = \"abc\"", "media_secret_key"),
        ("identity", "api_key = \"abc\"", "api_key"),
        (
            "payments",
            "webhook_signing_secret = \"abc\"",
            "webhook_signing_secret",
        ),
        ("identity", "session_token = \"abc\"", "session_token"),
    ] {
        let text = format!("{CONTROL_PLANE}\n[{section}]\n{entry}\n");
        assert!(
            matches!(
                load(&text),
                Err(ConfigError::InlineSecret { section: found, key: ref found_key })
                    if found == section && found_key == key
            ),
            "{section}.{key}"
        );
    }
    let nested =
        format!("{CONTROL_PLANE}\n[[objects.buckets]]\nname = \"a\"\nsecret_key = \"abc\"\n");
    assert!(
        matches!(load(&nested), Err(ConfigError::InlineSecret { .. })),
        "in an array of tables"
    );
    let by_file = format!(
        "{CONTROL_PLANE}\n[identity]\napi_key_file = \"/run/secrets/workos_api_key\"\nmode = \"live\"\n"
    );
    assert!(load(&by_file).is_ok());
}

#[test]
fn the_listing_redacts_every_non_file_string_of_an_owned_section_and_renders_arrays_of_tables() {
    let text = format!(
        "{CONTROL_PLANE}\n[objects]\nendpoint = \"https://spaces.example\"\nmedia_secret_key_file = \"/run/secrets/k\"\n\
         [[objects.buckets]]\nname = \"media\"\nsize = 3\n"
    );
    let lines = load(&text).expect("valid").listing().expect("listing");
    assert!(
        lines.contains(&"objects.endpoint = \"<redacted>\"".to_string()),
        "{lines:#?}"
    );
    assert!(lines.contains(&"objects.media_secret_key_file = \"/run/secrets/k\"".to_string()));
    assert!(
        lines.contains(&"objects.buckets = [{ name = \"<redacted>\", size = 3 }]".to_string()),
        "{lines:#?}"
    );
}

#[test]
fn a_hosted_environment_needs_an_https_telemetry_endpoint_without_credentials() {
    let plain = with(
        CONTROL_PLANE,
        "https://127.0.0.1:4317",
        "http://127.0.0.1:4317",
    );
    assert!(matches!(
        load(&plain),
        Err(ConfigError::Inconsistent { .. })
    ));
    let credentials = with(
        CONTROL_PLANE,
        "https://127.0.0.1:4317",
        "https://user:pass@ingest.example:4317",
    );
    assert!(matches!(
        load(&credentials),
        Err(ConfigError::Inconsistent { .. })
    ));
    assert!(
        load(&with(
            CONTROL_PLANE,
            "https://127.0.0.1:4317",
            "https://ingest.example:4317/v1"
        ))
        .is_ok()
    );
}

#[test]
fn a_hosted_metrics_listener_must_name_one_interface() {
    for wildcard in ["0.0.0.0:9100", "[::]:9100"] {
        let text = with(CONTROL_PLANE, "10.0.0.5:9100", wildcard);
        assert_eq!(
            load(&text),
            Err(ConfigError::WildcardMetricsAddress),
            "{wildcard}"
        );
    }
}

fn temp_file(directory: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = directory.path().join(name);
    std::fs::File::create(&path)
        .expect("create")
        .write_all(content.as_bytes())
        .expect("write");
    path
}

#[test]
fn loading_a_file_checks_that_the_files_it_names_exist() {
    let directory = tempfile::tempdir().expect("directory");
    let password = temp_file(&directory, "password", "hunter2\n");
    let ca = temp_file(&directory, "ca.pem", "certificate");
    let text = with(
        &with(
            CONTROL_PLANE,
            "/run/secrets/database_password",
            &password.to_string_lossy().replace('\\', "/"),
        ),
        "/etc/ssl/database-ca.pem",
        &ca.to_string_lossy().replace('\\', "/"),
    );
    let file = temp_file(&directory, "control.toml", &text);
    // Hosted paths must be absolute; a temp path is absolute on both platforms.
    let loaded = ServerConfig::load(&file);
    assert!(loaded.is_ok(), "{loaded:?}");
    std::fs::remove_file(&password).expect("remove");
    assert_eq!(
        ServerConfig::load(&file),
        Err(ConfigError::ReferencedFileUnreadable {
            key: "database.password_file"
        })
    );
    std::fs::write(&password, "").expect("empty");
    assert_eq!(
        ServerConfig::load(&file),
        Err(ConfigError::ReferencedFileUnreadable {
            key: "database.password_file"
        })
    );
    assert!(matches!(
        ServerConfig::load(&directory.path().join("absent.toml")),
        Err(ConfigError::Unreadable { .. })
    ));
}

/// Every key set to a value that differs from its default, one file per role and environment.
const FULL_CONTROL_PLANE: &str = r#"
[instance]
role = "control-plane"
environment = "staging"
instance_id = "control-z"

[listen]
public_address = "0.0.0.0:9000"
internal_address = "10.9.0.5:9001"
metrics_address = "10.9.0.5:9002"

[database]
host = "db.example.internal"
port = 5432
name = "df_staging"
user = "df_control"
password_file = "/secrets/db"
pool_size = 7
statement_timeout_ms = 12000

[database.tls]
mode = "verify-full"
ca_file = "/secrets/ca.pem"
server_name = "db.example.internal"

[paths]
temp_directory = "/scratch"

[telemetry]
otlp_endpoint = "https://ingest.example.internal:4318"
service_name = "df-staging-control"
spool_directory = "/var/spool/df"
spool_bytes_maximum = 1048576000
push_interval_seconds = 20

[runtime]
maximum_seats = 5

[runtime.lease]
seconds = 40
renew_every_seconds = 11
safety_margin_seconds = 4
placing_seconds = 70

[runtime.meter]
segment_roll_seconds = 90
backdate_window_seconds = 50

[runtime.presence]
window_ms = 31000
idle_release_seconds = 1000
away_end_hours = 11

[runtime.blocking_wait]
gated_grace_ms = 11000
outage_grace_ms = 500
blocked_interval_free = false

[runtime.owner]
stall_seconds = 16
stall_exit_seconds = 61

[runtime.money]
reconstruction_deadline_seconds = 31

[runtime.drain]
effect_wait_seconds = 31
maximum_seconds = 91
edge_seconds = 21

[objects]
bucket = "media"

[identity]
provider = "workos"

[payments]
mode = "test"
"#;

const FULL_EGRESS: &str = r#"
[instance]
role = "egress"
environment = "staging"
instance_id = "egress-z"

[listen]
internal_address = "10.9.0.7:9001"
metrics_address = "10.9.0.7:9002"

[database]
host = "db.example.internal"
port = 5432
name = "df_staging"
user = "df_egress"
password_file = "/secrets/db"
pool_size = 3
statement_timeout_ms = 9000

[database.tls]
mode = "verify-full"
ca_file = "/secrets/ca.pem"
server_name = "db.example.internal"

[paths]
temp_directory = "/scratch"

[telemetry]
otlp_endpoint = "https://ingest.example.internal:4318"
service_name = "df-staging-egress"
spool_directory = "/var/spool/df"

[providers]
mode = "live"
allow_live = true
"#;

#[cfg(feature = "local-dev")]
const FULL_LOCAL: &str = r#"
[instance]
role = "campaign-runtime"
environment = "local"
instance_id = "local-z"

[listen]
internal_address = "127.0.0.1:9001"
metrics_address = "127.0.0.1:9002"

[database]
host = "/var/run/postgresql"
port = 5433
name = "df_local"
user = "postgres"
password_file = "secret.txt"

[database.tls]
mode = "loopback-plaintext"

[paths]
temp_directory = "tmp"

[telemetry]
otlp_endpoint = "http://127.0.0.1:4318"
service_name = "df-local-z"
spool_directory = "spool"

[local_dev]
dev_control = true
"#;

fn raw_listing(text: &str) -> Vec<String> {
    let mut table = text.parse::<toml::Table>().expect("raw toml");
    redact_owned_sections(&mut table);
    flatten(&table)
}

fn listing(text: &str) -> Vec<String> {
    load(text).expect("valid").listing().expect("listing")
}

/// A minimal file and the file that sets every key for each role and environment this build
/// can load (the local pair needs the `local-dev` feature).
fn file_pairs() -> Vec<(&'static str, &'static str)> {
    let pairs = vec![(CONTROL_PLANE, FULL_CONTROL_PLANE), (EGRESS, FULL_EGRESS)];
    #[cfg(feature = "local-dev")]
    let pairs = [pairs, vec![(LOCAL_RUNTIME, FULL_LOCAL)]].concat();
    pairs
}

#[test]
fn the_listing_of_a_fully_set_file_is_that_files_settings_plus_only_defaults() {
    for (minimal, full) in file_pairs() {
        let listed = listing(full);
        let defaults = listing(minimal);
        let written = raw_listing(full);
        for line in &written {
            assert!(
                listed.contains(line),
                "the file wrote `{line}` but the effective listing lacks it"
            );
        }
        for line in listed.iter().filter(|line| !written.contains(line)) {
            assert!(
                defaults.contains(line),
                "`{line}` is neither written by the file nor a default"
            );
        }
    }
}

#[test]
fn a_file_that_sets_every_setting_leaves_nothing_to_a_default() {
    assert_eq!(listing(FULL_CONTROL_PLANE), raw_listing(FULL_CONTROL_PLANE));
}

#[test]
fn every_defaulted_setting_differs_in_the_fully_set_file() {
    const DEFAULTED: [&str; 4] = [
        "runtime.",
        "database.pool_size",
        "database.statement_timeout_ms",
        "telemetry.spool_bytes_maximum",
    ];
    let defaults = listing(CONTROL_PLANE);
    let set = listing(FULL_CONTROL_PLANE);
    let mut checked = 0;
    for line in defaults
        .iter()
        .filter(|line| DEFAULTED.iter().any(|prefix| line.starts_with(prefix)))
    {
        assert!(
            !set.contains(line),
            "`{line}` is still the default in the fully set file"
        );
        checked += 1;
    }
    assert!(checked > 20, "the defaulted settings must be found");
}

#[test]
fn every_setting_the_server_defaults_is_set_by_some_fully_set_file() {
    let pairs = file_pairs();
    let written: BTreeSet<String> = pairs
        .iter()
        .flat_map(|(_, full)| raw_listing(full))
        .map(|line| line.split(" = ").next().expect("key").to_string())
        .collect();
    let mut minimals: Vec<&str> = pairs.iter().map(|(minimal, _)| *minimal).collect();
    minimals.push(RUNTIME);
    for minimal in minimals {
        for line in listing(minimal) {
            let key = line.split(" = ").next().expect("key");
            assert!(
                written.contains(key),
                "no fully set file writes `{key}`; add it so the listing test covers it"
            );
        }
    }
}
