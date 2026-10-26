use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

struct ConfigFixture {
    directory: TempDir,
    path: PathBuf,
}

impl ConfigFixture {
    fn new() -> Self {
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../artifacts/tmp/agents/BH-FIX-migrate-apply-reports-success-without-migrations",
        );
        fs::create_dir_all(&artifacts).expect("create lane artifacts directory");
        let directory = tempfile::Builder::new()
            .prefix("migrate-cli-")
            .tempdir_in(artifacts)
            .expect("create config fixture directory");
        let password_file = directory.path().join("database-password");
        let ca_file = directory.path().join("database-ca.pem");
        fs::write(&password_file, "synthetic-test-password").expect("write test password");
        fs::write(&ca_file, "synthetic-test-ca").expect("write test CA");
        let config = format!(
            "[instance]\nrole = \"control-plane\"\nenvironment = \"staging\"\ninstance_id = \"migrate-cli-test\"\n\
             [listen]\npublic_address = \"127.0.0.1:63310\"\ninternal_address = \"127.0.0.1:63311\"\nmetrics_address = \"127.0.0.1:63312\"\n\
             [database]\nhost = \"db.example.test\"\nport = 5432\nname = \"dungeonflux\"\nuser = \"df_migrator\"\npassword_file = {}\n\
             [database.tls]\nmode = \"verify-full\"\nca_file = {}\nserver_name = \"db.example.test\"\n\
             [paths]\ntemp_directory = {}\n\
             [telemetry]\notlp_endpoint = \"https://telemetry.example.test\"\nservice_name = \"df-server-test\"\nspool_directory = {}\n",
            toml_string(&password_file),
            toml_string(&ca_file),
            toml_string(directory.path()),
            toml_string(directory.path()),
        );
        let path = directory.path().join("server.toml");
        fs::write(&path, config).expect("write valid server config");
        Self { directory, path }
    }
}

fn toml_string(path: &Path) -> String {
    let path = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{path}\"")
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_df-server"))
        .args(arguments)
        .output()
        .expect("run df-server")
}

#[test]
fn migration_verbs_report_unavailable_after_valid_config_load() {
    let fixture = ConfigFixture::new();
    let config_path = fixture.path.to_str().expect("config path is UTF-8");

    for action in ["apply", "status", "plan"] {
        let output = run(&["migrate", action, "--config", config_path]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(!output.status.success(), "{action} unexpectedly succeeded");
        assert!(stderr.contains("migration runner unavailable"), "{stderr}");
        assert!(!stdout.contains("config valid"), "{stdout}");
    }

    let output = run(&["config", "check", "--config", config_path]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("migration runner unavailable"));

    let invalid_path = fixture.directory.path().join("invalid.toml");
    fs::write(&invalid_path, "invalid = true").expect("write invalid config");
    let invalid = run(&[
        "migrate",
        "apply",
        "--config",
        invalid_path.to_str().expect("invalid config path is UTF-8"),
    ]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("config refused:"));
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("migration runner unavailable"));

    let missing = run(&[
        "migrate",
        "apply",
        "--config",
        fixture
            .directory
            .path()
            .join("missing.toml")
            .to_str()
            .expect("missing config path is UTF-8"),
    ]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("config refused:"));
    assert!(!String::from_utf8_lossy(&missing.stderr).contains("migration runner unavailable"));
}
