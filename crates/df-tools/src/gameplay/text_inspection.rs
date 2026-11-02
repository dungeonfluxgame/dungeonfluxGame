//! Start-up for the durable text service: connect the run's `df_text` store, install it for the
//! `dev text` surface and as the shared text runtime, then say what was chosen in one `TEXT` line.
//! When the database is not available the runtime keeps attempts in memory and the line says so;
//! text generation is never gated on the store.

use std::sync::Arc;

use df_ai::{text_attempts::AttemptStoreError, text_fixture::MemoryAttemptStore};
use df_observe::DiagnosticSink;
use df_persistence::text_attempts::PostgresTextAttempts;

use super::{
    dev_control::text::install,
    text_runtime::{self, RuntimeStore},
};

/// The one start-up line. Never names the connection string, password or host.
pub(super) async fn start(
    database: &tokio_postgres::Config,
    sink: Option<Arc<dyn DiagnosticSink>>,
) -> String {
    let run = database.get_dbname().unwrap_or_default().to_owned();
    let logging = if sink.is_some() {
        "logged to the shared telemetry"
    } else {
        "not logged: no telemetry sink"
    };
    if matches!(
        std::env::var("DF_GAMEPLAY_TEXT_ATTEMPTS"),
        Ok(value) if value.trim().eq_ignore_ascii_case("off")
    ) {
        return memory_only("DF_GAMEPLAY_TEXT_ATTEMPTS=off", sink, &run, logging);
    }
    match PostgresTextAttempts::connect(database).await {
        Ok(store) => {
            let store = Arc::new(store);
            install(store.clone(), sink.clone());
            text_runtime::install(RuntimeStore::Postgres(store), sink, &run);
            format!("TEXT attempts on (df_text schema; {logging})")
        }
        Err(error) => memory_only(failure_reason(error), sink, &run, logging),
    }
}

/// A run without the durable store still sends at most once per operation inside the process.
fn memory_only(
    reason: &str,
    sink: Option<Arc<dyn DiagnosticSink>>,
    run: &str,
    logging: &str,
) -> String {
    text_runtime::install(
        RuntimeStore::Memory(Arc::new(MemoryAttemptStore::default())),
        sink,
        run,
    );
    format!("TEXT attempts memory-only, not kept across a restart ({reason}; {logging})")
}

fn failure_reason(error: AttemptStoreError) -> &'static str {
    match error {
        AttemptStoreError::Unreachable => "database unreachable",
        AttemptStoreError::TimedOut => "database did not answer in time",
        AttemptStoreError::Rejected => "database refused the schema",
        AttemptStoreError::Invalid => "store refused the configuration",
    }
}
