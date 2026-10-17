//! Focused tests for the story memory capture boundary.

use std::sync::Arc;

use df_persistence::story_memory::StoredEvent;
use df_protocol::common as rpc;

use super::super::{Backend, Config, StoryMemory, test_store::TestStore};
use crate::gameplay::{resources::SpendLedger, table_log::TableLog};

fn memory() -> Arc<StoryMemory> {
    StoryMemory::new(
        Backend::Test(TestStore::new(true)),
        None,
        Arc::new(SpendLedger::default()),
        Config::new("capture-tests"),
    )
}

fn events(memory: &StoryMemory) -> Vec<StoredEvent> {
    match &memory.backend {
        Backend::Test(store) => store.all(&memory.run),
        Backend::Postgres(_) => unreachable!("capture tests use the in-process store"),
    }
}

#[tokio::test]
async fn table_lines_keep_the_current_scene_key() {
    let memory = memory();
    memory.set_scene("warehouse-scene");
    let log = TableLog::new();
    super::attach(&memory, &log);

    log.push_speech("Brynn", 0, "I ask about the warehouse.");
    memory.settle().await;

    let captured = events(&memory);
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].scene.as_deref(), Some("warehouse-scene"));
}

#[tokio::test]
async fn an_established_lore_fact_is_recorded_and_sets_the_scene_for_table_lines() {
    let memory = memory();
    let log = TableLog::new();
    super::attach(&memory, &log);

    assert!(memory.record(super::canon_fact_draft(
        "warehouse-scene",
        "place",
        "Warehouse",
        "Tomas keeps the warehouse.",
        "Tomas",
    )));
    memory.set_scene("warehouse-scene");
    log.push_actor(
        "npc:tomas",
        "Tomas",
        rpc::TableRole::Npc,
        0,
        rpc::TableKind::Speech,
        "I keep this warehouse.",
    );
    memory.settle().await;

    let captured = events(&memory);
    let fact = captured
        .iter()
        .find(|event| event.kind.label() == "canon_fact")
        .expect("established lore is captured");
    assert!(fact.text.contains("Tomas keeps the warehouse."));
    assert_eq!(fact.scene.as_deref(), Some("warehouse-scene"));
    assert!(fact.entities.npc.iter().any(|entity| entity == "tomas"));
    assert!(
        captured
            .iter()
            .filter(|event| event.source == "table_log")
            .all(|event| event.scene.as_deref() == Some("warehouse-scene"))
    );
}
