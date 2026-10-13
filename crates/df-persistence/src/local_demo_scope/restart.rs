//! A local launch database retains its successor; old checkpoints and operations are untouched.
use df_session::submission::RepositoryError;
use tokio_postgres::Client;

fn validate_database(name: &str) -> Result<(), RepositoryError> {
    if !name.starts_with("df_gameplay_demo_")
        || name.len() > 63
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(RepositoryError::Unauthorized);
    }
    Ok(())
}

/// Read the next run, or none for an untouched launch database. Missing catalogs are fresh,
/// whereas database/query failures are errors and never permission to discard a stored game.
pub async fn run_successor(client: &Client) -> Result<Option<String>, RepositoryError> {
    let catalog = client
        .query_one(
            "SELECT to_regclass('df_local_demo.run_successor') IS NOT NULL",
            &[],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if !catalog.get::<_, bool>(0) {
        return Ok(None);
    }
    let row = client
        .query_opt(
            "SELECT database_name FROM df_local_demo.run_successor WHERE singleton",
            &[],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    let name: Option<String> = row.map(|row| row.get(0));
    if let Some(name) = &name {
        validate_database(name)?;
    }
    Ok(name)
}

/// Called only after the old owner is drained and released. A durable singleton ensures that
/// simultaneous requests and crash recovery all select exactly one successor database.
pub async fn retain_run_successor(client: &Client, name: &str) -> Result<(), RepositoryError> {
    validate_database(name)?;
    client.batch_execute("CREATE TABLE IF NOT EXISTS df_local_demo.run_successor (singleton boolean PRIMARY KEY CHECK (singleton), database_name text NOT NULL CHECK (length(database_name) <= 63))")
        .await.map_err(|_| RepositoryError::Unavailable)?;
    client.execute("INSERT INTO df_local_demo.run_successor VALUES (true, $1) ON CONFLICT (singleton) DO NOTHING", &[&name])
        .await.map_err(|_| RepositoryError::Unavailable)?;
    if run_successor(client).await?.as_deref() != Some(name) {
        return Err(RepositoryError::RevisionConflict);
    }
    Ok(())
}

/// An admin connection to `postgres` allocates an empty database. The validated identifier is
/// the only SQL identifier interpolated here; repeated allocations resume the same database.
pub async fn ensure_run_database(admin: &Client, name: &str) -> Result<(), RepositoryError> {
    validate_database(name)?;
    let exists = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname=$1)",
            &[&name],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .get::<_, bool>(0);
    if !exists {
        match admin
            .batch_execute(&format!("CREATE DATABASE \"{name}\" TEMPLATE template0"))
            .await
        {
            Ok(()) => {}
            Err(error)
                if error.code() == Some(&tokio_postgres::error::SqlState::DUPLICATE_DATABASE) => {}
            Err(_) => return Err(RepositoryError::Unavailable),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successor_database_names_cannot_escape_the_local_namespace() {
        for name in [
            "postgres",
            "df_gameplay_demo_bad\";DROP DATABASE postgres",
            "df_gameplay_demo_a-b",
            "df_gameplay_demo_Upper",
        ] {
            assert!(validate_database(name).is_err(), "{name}");
        }
        assert!(validate_database("df_gameplay_demo_next_0123abcd").is_ok());
    }

    #[test]
    #[ignore = "requires the registered owned PostgreSQL fixture"]
    fn successor_allocation_is_idempotent_and_preserves_the_old_database() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let port: u16 = std::env::var("DUNGEONFLUX_I01_OWNED_PG_PORT")
            .unwrap()
            .parse()
            .unwrap();
        let database = std::env::var("DUNGEONFLUX_I01_OWNED_PG_DATABASE").unwrap();
        let user = std::env::var("DUNGEONFLUX_I01_OWNED_PG_ADMIN_ROLE").unwrap();
        let mut config = tokio_postgres::Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user(&user)
            .dbname("postgres");
        let (admin, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let driver = tokio::spawn(connection);
        let old = format!("{database}_s03_old");
        let next = format!("{database}_s03_next");
        ensure_run_database(&admin, &old).await.unwrap();
        config.dbname(&old);
        let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let old_driver = tokio::spawn(connection);
        client.batch_execute("CREATE SCHEMA df_local_demo; CREATE TABLE df_local_demo.old_evidence (value integer); INSERT INTO df_local_demo.old_evidence VALUES (53)").await.unwrap();
        assert_eq!(run_successor(&client).await.unwrap(), None);
        retain_run_successor(&client, &next).await.unwrap();
        retain_run_successor(&client, &next).await.unwrap();
        assert_eq!(
            retain_run_successor(&client, &old).await,
            Err(RepositoryError::RevisionConflict)
        );
        ensure_run_database(&admin, &next).await.unwrap();
        ensure_run_database(&admin, &next).await.unwrap();
        assert_eq!(
            run_successor(&client).await.unwrap().as_deref(),
            Some(next.as_str())
        );
        assert_eq!(
            client
                .query_one("SELECT value FROM df_local_demo.old_evidence", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            53
        );
        drop(client);
        old_driver.await.unwrap().unwrap();
        // Retain both owned databases as readable evidence. The fixture owner retires
        // its postmaster and data directory; this storage test never deletes a run.
        drop(admin);
        driver.await.unwrap().unwrap();
        });
    }
}
