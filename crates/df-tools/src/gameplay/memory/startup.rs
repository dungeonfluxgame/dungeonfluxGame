//! Start-up: connect to the run's database, apply the story memory migrations, choose the search
//! mode from what is available, install the table tap and the global handle, and say what was
//! chosen in one `MEMORY` line.
//!
//! Memory is a help to the Dungeon Master, never a gate on play: any failure here turns it off
//! with the reason in the line, and the game carries on.

use std::sync::Arc;

use df_persistence::story_memory::StoryMemoryDb;

use super::{
    Backend, Config, StoryMemory, backfill::Sources, capture, dossier::StorytellerWriter,
    embedder::OpenAiEmbedder, install,
};
use crate::gameplay::{resources::Hub, storyteller::Storyteller, table_log::TableLog};

/// What start-up made.
pub(in crate::gameplay) struct Started {
    pub(in crate::gameplay) memory: Option<Arc<StoryMemory>>,
    /// The one start-up line.
    pub(in crate::gameplay) line: String,
}

fn usable_key(value: Result<String, std::env::VarError>) -> Result<String, &'static str> {
    match value {
        Err(_) => Err("OPENAI_API_KEY not set"),
        Ok(key) if key.trim().is_empty() => Err("OPENAI_API_KEY empty"),
        Ok(key) if key.len() > 8192 || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) => {
            Err("OPENAI_API_KEY not usable")
        }
        Ok(key) => Ok(key),
    }
}

/// The run's memory, or why there is none. `sources` lets the backfill and `dev memory reindex`
/// read canon, the social ledger and the decision journal; `storyteller` writes dossiers.
pub(in crate::gameplay) async fn start(
    database: &tokio_postgres::Config,
    run_id: &str,
    hub: &Hub,
    table_log: &Arc<TableLog>,
    sources: Option<Sources>,
    storyteller: Option<Arc<Storyteller>>,
    runtime: &tokio::runtime::Handle,
) -> Started {
    if !crate::gameplay::media_policy::gates()
        .allows(crate::gameplay::media_policy::Capability::Memory)
    {
        return Started {
            memory: None,
            line: "MEMORY off (DF_GAMEPLAY_MEMORY=off)".to_owned(),
        };
    }
    let db = match StoryMemoryDb::connect(database).await {
        Ok(db) => db,
        Err(error) => {
            return Started {
                memory: None,
                line: format!("MEMORY off ({error})"),
            };
        }
    };
    let model = std::env::var("DF_GAMEPLAY_MEMORY_MODEL")
        .ok()
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| df_providers::EMBEDDING_MODEL_SMALL.to_owned());
    let mut why_text_only = None;
    let embedder: Option<Arc<dyn super::Embedder>> =
        match usable_key(std::env::var("OPENAI_API_KEY")) {
            Ok(key) => match OpenAiEmbedder::new(&key, &model) {
                Ok(embedder) => Some(Arc::new(embedder)),
                Err(reason) => {
                    why_text_only = Some(reason.to_owned());
                    None
                }
            },
            Err(reason) => {
                why_text_only = Some(reason.to_owned());
                None
            }
        };
    if !db.vector_enabled() {
        why_text_only = Some(format!(
            "vector extension unavailable: {}",
            db.vector_reason().unwrap_or("unknown")
        ));
    }
    let memory = StoryMemory::new(
        Backend::Postgres(Box::new(db)),
        embedder,
        Arc::clone(hub.ledger()),
        Config::new(run_id),
    );
    memory.spawn_workers(runtime);
    capture::attach(&memory, table_log);
    if let Some(storyteller) = storyteller.filter(|storyteller| storyteller.is_on()) {
        memory.set_dossier_writer(Arc::new(StorytellerWriter(storyteller)));
    }
    if let Some(sources) = sources {
        memory.set_sources(sources);
        let _offered = memory.backfill();
    }
    install(Arc::clone(&memory));
    let counts = memory
        .backend
        .counts(&memory.run, memory.model().as_deref().unwrap_or(""))
        .await
        .unwrap_or_default();
    let search = match (&memory.embedder, why_text_only) {
        (Some(_), None) => format!("hybrid model={model}"),
        (_, reason) => format!(
            "text_only ({}; events are stored and searched by full text)",
            reason.unwrap_or_else(|| "no embedder".to_owned())
        ),
    };
    Started {
        line: format!(
            "MEMORY on run={} backend=postgres search={search} spend=recorded_not_capped events={} backlog={}",
            memory.run, counts.total, counts.backlog
        ),
        memory: Some(memory),
    }
}

#[cfg(test)]
mod tests {
    use super::usable_key;

    #[test]
    fn a_missing_empty_or_unprintable_key_is_named_not_used() {
        assert_eq!(
            usable_key(Err(std::env::VarError::NotPresent)),
            Err("OPENAI_API_KEY not set")
        );
        assert_eq!(usable_key(Ok("  ".to_owned())), Err("OPENAI_API_KEY empty"));
        assert_eq!(
            usable_key(Ok("a b".to_owned())),
            Err("OPENAI_API_KEY not usable")
        );
        assert_eq!(
            usable_key(Ok("sk-test".to_owned())),
            Ok("sk-test".to_owned())
        );
    }
}
