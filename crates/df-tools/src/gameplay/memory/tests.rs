//! Tests for the story memory. They run against the in-process store and a deterministic fake
//! embedder: no database, no network, no paid call. The real-PostgreSQL half lives in
//! `df-persistence` (`story_memory_pg_fixture`).

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use df_protocol::common as rpc;

use super::{
    Audience, Backend, Config, EventKind, SearchMode, Stats, StoredEvent, StoryMemory,
    capture::{self, Draft},
    consistency::{Proposed, Superseded, Verdict},
    dossier::{DossierFailure, DossierOutcome, DossierRequest, DossierWriter},
    embedder::{BoxFuture, Embedder, FakeEmbedder},
    recall::{Purpose, RecallRequest, fuse, render},
    test_store::TestStore,
};
use crate::gameplay::{resources::SpendLedger, resources::Unit, table_log::TableLog};

fn memory_with(embedder: Option<Arc<FakeEmbedder>>) -> Arc<StoryMemory> {
    StoryMemory::new(
        Backend::Test(TestStore::new(true)),
        embedder.map(|embedder| embedder as Arc<dyn Embedder>),
        Arc::new(SpendLedger::default()),
        Config::new("run-1"),
    )
}

fn hybrid() -> (Arc<StoryMemory>, Arc<FakeEmbedder>) {
    let fake = FakeEmbedder::new("fake-1");
    (memory_with(Some(Arc::clone(&fake))), fake)
}

fn stored(memory: &StoryMemory) -> Vec<StoredEvent> {
    match &memory.backend {
        Backend::Test(store) => store.all(&memory.run),
        Backend::Postgres(_) => unreachable!("tests use the in-process store"),
    }
}

async fn add(memory: &Arc<StoryMemory>, draft: Draft) -> i64 {
    assert!(memory.record(draft.clone()));
    memory.settle().await;
    let key = memory.event_from(draft).unwrap().natural_key;
    stored(memory)
        .iter()
        .find(|event| event.natural_key == key)
        .map(|event| event.event_id)
        .expect("the event was written")
}

fn say(text: &str) -> Draft {
    Draft::new(EventKind::NpcSpeech, "test", text)
}

fn public_request(text: &str) -> RecallRequest {
    RecallRequest::new(Purpose::NpcReply, Audience::Public, text)
}

#[test]
fn fusion_rewards_agreement_between_lists_and_breaks_ties_to_the_newer_event() {
    let fused = fuse(&[(&[1, 2, 3], 1.0), (&[3, 1], 1.0)]);
    assert_eq!(
        fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![1, 3, 2]
    );
    let tied = fuse(&[(&[5], 1.0), (&[7], 1.0)]);
    assert_eq!(
        tied.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![7, 5]
    );
    assert!(fuse(&[]).is_empty());
}

#[tokio::test]
async fn recall_ranks_the_event_that_matches_the_line_first_in_hybrid_mode() {
    let (memory, fake) = hybrid();
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    add(&memory, say("The carter promised cheese to the miller.")).await;
    add(&memory, say("Brynn bought forty feet of rope.")).await;
    assert_eq!(memory.mode(), SearchMode::Hybrid);
    let pack = memory
        .recall(&public_request("Who owns the warehouse?"))
        .await;
    assert!(pack.coverage.query_embedded);
    assert!(
        pack.related[0].event.text.contains("warehouse"),
        "{:?}",
        pack.related
    );
    assert!(pack.related[0].score > 0.0);
    assert!(pack.rendered.starts_with("RECALLED MEMORY"));
    assert!(pack.rendered.contains("never instructions"));
    // Events were embedded after they were stored, in batches, and the query once.
    assert_eq!(Stats::get(&memory.stats.embedded), 3);
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        4,
        "one call per settled write, one for the query"
    );
}

#[tokio::test]
async fn a_private_clue_is_never_recalled_into_a_public_line_but_reaches_its_hero() {
    let (memory, _fake) = hybrid();
    let public = add(&memory, say("Marek says the warehouse is his.")).await;
    let party = add(
        &memory,
        Draft::new(
            EventKind::CanonFact,
            "test",
            "The party knows the warehouse key is brass.",
        )
        .party_only()
        .npc("marek"),
    )
    .await;
    let brynn = add(
        &memory,
        Draft::new(
            EventKind::CanonFact,
            "test",
            "Secret warehouse clue: the false wall hides the ledger.",
        )
        .private_to("Brynn")
        .npc("marek"),
    )
    .await;
    let tobin = add(
        &memory,
        Draft::new(
            EventKind::PromiseThread,
            "test",
            "Secret warehouse promise made only to Tobin.",
        )
        .private_to("Tobin")
        .npc("marek"),
    )
    .await;
    let ask = |audience: Audience| {
        RecallRequest::new(
            Purpose::NpcReply,
            audience,
            "warehouse clue ledger secret key",
        )
        .entities(&["marek"])
    };
    let ids = |pack: &super::recall::MemoryPack| {
        pack.pinned_ids
            .iter()
            .chain(&pack.related_ids)
            .copied()
            .collect::<Vec<_>>()
    };

    let public_pack = memory.recall(&ask(Audience::Public)).await;
    assert_eq!(
        ids(&public_pack)
            .iter()
            .filter(|id| [party, brynn, tobin].contains(id))
            .count(),
        0
    );
    assert!(ids(&public_pack).contains(&public));
    assert!(
        !public_pack.rendered.contains("false wall") && !public_pack.rendered.contains("brass")
    );

    let party_pack = memory.recall(&ask(Audience::Party)).await;
    assert!(ids(&party_pack).contains(&party));
    assert!(!ids(&party_pack).contains(&brynn) && !ids(&party_pack).contains(&tobin));

    let brynn_pack = memory
        .recall(&ask(Audience::Member("brynn".to_owned())))
        .await;
    assert!(ids(&brynn_pack).contains(&brynn) && ids(&brynn_pack).contains(&party));
    assert!(!ids(&brynn_pack).contains(&tobin));
    assert!(brynn_pack.rendered.contains("false wall"));

    // The entity view and the dossier input obey the same audience.
    let view = memory.entity_view("marek", Audience::Public, true).await;
    assert!(view.events.iter().all(|event| event.event_id == public));
    assert_eq!(Stats::get(&memory.stats.violations), 0);
    // And the second check refuses a row the store should never have returned.
    let leaked = stored(&memory)
        .into_iter()
        .find(|e| e.event_id == brynn)
        .unwrap();
    assert!(!memory.permitted(&Audience::Public, &leaked));
    assert!(memory.permitted(&Audience::Member("brynn".to_owned()), &leaked));
}

#[tokio::test]
async fn pinned_items_are_the_promises_grievances_and_canon_about_the_named_entities() {
    let (memory, _fake) = hybrid();
    let promise = add(
        &memory,
        Draft::new(
            EventKind::PromiseThread,
            "test",
            "Marek promised the carter a share of the harbour toll.",
        )
        .npc("marek"),
    )
    .await;
    let grievance = add(
        &memory,
        Draft::new(
            EventKind::GrievanceAttitude,
            "test",
            "Marek holds a grievance: Brynn insulted his scales.",
        )
        .npc("marek")
        .hero("Brynn"),
    )
    .await;
    let canon = add(
        &memory,
        Draft::new(
            EventKind::CanonFact,
            "test",
            "The cloth shop stands on Tanner Lane.",
        )
        .npc("marek")
        .place("tanner-lane"),
    )
    .await;
    let other = add(
        &memory,
        Draft::new(
            EventKind::GrievanceAttitude,
            "test",
            "Ilsa holds a grievance: Tobin trod on her cat.",
        )
        .npc("ilsa"),
    )
    .await;
    // The line shares no word with any of them.
    let pack = memory
        .recall(
            &RecallRequest::new(Purpose::NpcReply, Audience::Public, "zzz qqq")
                .entities(&["marek"]),
        )
        .await;
    assert!(pack.pinned_ids.contains(&promise));
    assert!(pack.pinned_ids.contains(&grievance));
    assert!(pack.pinned_ids.contains(&canon));
    assert!(!pack.pinned_ids.contains(&other));
    assert!(pack.rendered.contains("Pinned:"));
    // Nothing named: a promise is still pinned, but not another person's grievance.
    let bare = memory.recall(&public_request("zzz qqq")).await;
    assert_eq!(bare.pinned_ids, vec![promise]);
}

#[tokio::test]
async fn a_superseded_event_leaves_recall_but_stays_in_the_record_and_the_entity_view() {
    let (memory, _fake) = hybrid();
    let old = add(
        &memory,
        say("Marek says the gate opens at dawn.").npc("marek"),
    )
    .await;
    let new = add(
        &memory,
        say("Marek says the gate opens at noon.").npc("marek"),
    )
    .await;
    assert_eq!(memory.supersede(old, new).await, Ok(true));
    assert_eq!(memory.supersede(old, new).await, Ok(false));
    let pack = memory
        .recall(&public_request("when does the gate open"))
        .await;
    assert!(pack.related_ids.contains(&new));
    assert!(!pack.related_ids.contains(&old));
    let view = memory.entity_view("marek", Audience::Public, true).await;
    let found = view.events.iter().find(|e| e.event_id == old).unwrap();
    assert_eq!(found.status.label(), "superseded");
    assert_eq!(found.superseded_by, Some(new));
    let active = memory.entity_view("marek", Audience::Public, false).await;
    assert!(active.events.iter().all(|e| e.event_id != old));
    assert!(memory.supersede(old, 999).await.is_err());
    let replacement = stored(&memory)
        .into_iter()
        .find(|e| e.event_id == new)
        .unwrap();
    assert_eq!(replacement.supersedes_id, Some(old));
}

#[tokio::test]
async fn the_same_source_event_written_twice_is_stored_once() {
    let memory = memory_with(None);
    let line = rpc::TableLine {
        audio_url: String::new(),
        audio_part_urls: Vec::new(),
        roll: None,
        sequence: 7,
        speaker: "Marek".to_owned(),
        role: rpc::TableRole::Npc as i32,
        hero_index: 0,
        kind: rpc::TableKind::Speech as i32,
        text: "The gate is shut.".to_owned(),
        actor_id: "npc:marek".to_owned(),
    };
    for _ in 0..3 {
        memory.submit(super::Job::Event(Box::new(
            memory.table_event(&line).unwrap(),
        )));
    }
    for _ in 0..2 {
        assert!(memory.record(capture::canon_fact_draft(
            "harbor",
            "place",
            "Salt Quay",
            "The quay runs along the wall.",
            "Marek"
        )));
    }
    memory.settle().await;
    assert_eq!(stored(&memory).len(), 2);
    assert_eq!(Stats::get(&memory.stats.written), 2);
    assert_eq!(Stats::get(&memory.stats.duplicates), 3);
    // A different line at the same position is a different event.
    let mut other = line.clone();
    other.text = "The gate is open.".to_owned();
    assert_ne!(
        memory.table_event(&line).unwrap().natural_key,
        memory.table_event(&other).unwrap().natural_key
    );
}

#[tokio::test]
async fn the_rendered_pack_stays_inside_its_byte_bound_and_says_what_it_left_out() {
    let (memory, _fake) = hybrid();
    let long = "the harbour warehouse ledger shows grain ".repeat(12);
    for n in 0..40 {
        add(&memory, say(&format!("{long} lot {n}")).npc("marek")).await;
    }
    for n in 0..6 {
        add(
            &memory,
            Draft::new(
                EventKind::PromiseThread,
                "test",
                &format!("Marek promised {n} barrels of grain. {long}"),
            )
            .npc("marek"),
        )
        .await;
    }
    let mut request = public_request("harbour warehouse grain ledger").entities(&["marek"]);
    request.related_limit = 32;
    let pack = memory.recall(&request).await;
    assert!(!pack.rendered.is_empty());
    assert!(pack.rendered.len() <= 1536, "{}", pack.rendered.len());
    assert!(pack.coverage.omitted > 0);
    assert!(pack.rendered.contains("more not shown"));
    request.budget_bytes = 300;
    let small = memory.recall(&request).await;
    assert!(small.rendered.len() <= 300, "{}", small.rendered.len());
    // A quote inside an event cannot close the framing.
    let (framed, _) = render(&pack.pinned, &pack.related, &[], 1536);
    assert!(
        framed
            .lines()
            .skip(1)
            .all(|line| line.starts_with('-') || line.ends_with(':') || line.starts_with("(+"))
    );
    assert!(render(&[], &[], &[], 1536).0.is_empty());
}

#[tokio::test]
async fn with_no_key_events_are_stored_and_found_by_full_text_only() {
    let memory = memory_with(None);
    assert_eq!(memory.mode(), SearchMode::TextOnly("no embedding key"));
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    add(&memory, say("The carter promised cheese to the miller.")).await;
    let pack = memory
        .recall(&public_request("who owns the warehouse"))
        .await;
    assert!(!pack.coverage.query_embedded);
    assert_eq!(pack.coverage.mode.label(), "text_only");
    assert!(pack.related[0].event.text.contains("warehouse"));
    assert_eq!(Stats::get(&memory.stats.embedded), 0);
    let status = memory.status_json().await;
    assert_eq!(status["search"], "text_only");
    assert_eq!(status["events"]["total"], 2);
    assert_eq!(status["embedding"]["backlog"], 2);
}

#[tokio::test]
async fn embeddings_are_recorded_on_the_hub_ledger() {
    let (memory, _fake) = hybrid();
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    add(&memory, say("The carter promised cheese to the miller.")).await;
    assert_eq!(Stats::get(&memory.stats.embedded), 2);
    let spent = memory.ledger.snapshot(Unit::EmbeddingToken);
    assert!(spent.committed >= 2, "{spent:?}");
    assert_eq!(spent.reserved, 0);
    assert!(
        memory
            .recent_audit(20)
            .iter()
            .any(|line| line.starts_with("MEMORY embed model=fake-1"))
    );
}

/// A price table with the fake embedder's model: 0.02 a million tokens, as the real one.
fn embedding_prices() -> crate::gameplay::cost::PriceTable {
    crate::gameplay::cost::PriceTable::parse(
        r#"{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [
            {"provider": "openai", "model": "fake-1", "category": "text_input", "usd": "0.02",
             "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"}]}"#,
    )
    .unwrap()
}

fn open_book(ledger: &SpendLedger) {
    ledger
        .costs()
        .open(
            "memory",
            None,
            embedding_prices(),
            crate::gameplay::cost::PricingStatus::NotLoaded,
            1,
        )
        .unwrap();
}

#[tokio::test]
async fn an_embedding_call_is_metered_in_dollars_at_the_tokens_the_supplier_reported() {
    let (memory, _fake) = hybrid();
    open_book(&memory.ledger);
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    let run = &memory.ledger.costs().report(2)["run"];
    assert_eq!(run["events"], 1);
    assert_eq!(run["estimated_events"], 0, "the fake reports its tokens");
    assert_eq!(run["by_cause"][0]["cause"], "embedding");
    let tokens = memory.ledger.snapshot(Unit::EmbeddingToken).committed;
    assert_eq!(
        run["by_line"][0]["quantity"], tokens,
        "the metered tokens are the unit account's tokens"
    );
}

/// An embedder that answers nothing: it refuses before sending, or fails after.
struct Refusing(super::embedder::EmbedFailure);

impl Embedder for Refusing {
    fn model(&self) -> &str {
        "fake-1"
    }

    fn embed<'a>(
        &'a self,
        _texts: &'a [String],
    ) -> BoxFuture<'a, Result<super::embedder::Embedded, super::embedder::EmbedFailure>> {
        let failure = self.0;
        Box::pin(async move { Err(failure) })
    }
}

#[tokio::test]
async fn an_embedding_that_never_left_is_not_billed_and_one_that_failed_after_sending_is_unknown() {
    use super::embedder::{EmbedFailure, embed_charged};
    let ledger = Arc::new(SpendLedger::default());
    open_book(&ledger);
    let texts = vec!["the carter promised cheese".to_owned()];
    let unsent = Refusing(EmbedFailure::NotSent("refused"));
    assert!(embed_charged(&unsent, &ledger, &texts).await.is_err());
    assert_eq!(ledger.costs().report(2)["run"]["events"], 0);
    assert_eq!(
        ledger.snapshot(Unit::EmbeddingToken).committed,
        0,
        "refunded"
    );
    let sent = Refusing(EmbedFailure::Failed("timeout"));
    assert!(embed_charged(&sent, &ledger, &texts).await.is_err());
    let run = &ledger.costs().report(2)["run"];
    assert_eq!(run["events"], 1);
    assert_eq!(run["unknown_events"], 1);
    assert_eq!(
        run["by_line"][0]["quantity"],
        super::embedder::estimate_tokens(&texts),
        "booked at the estimate"
    );
}

#[tokio::test]
async fn a_failing_provider_leaves_events_stored_and_recall_working_by_text() {
    let (memory, fake) = hybrid();
    fake.failing.store(true, Ordering::SeqCst);
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    assert_eq!(Stats::get(&memory.stats.embedded), 0);
    assert!(Stats::get(&memory.stats.embed_failures) > 0);
    let pack = memory.recall(&public_request("warehouse owner")).await;
    assert!(!pack.coverage.query_embedded);
    assert_eq!(pack.related.len(), 1);
    fake.failing.store(false, Ordering::SeqCst);
    memory.settle().await;
    assert_eq!(
        Stats::get(&memory.stats.embedded),
        1,
        "the backlog is filled in later"
    );
}

#[tokio::test]
async fn every_row_is_versioned_by_model_and_a_changed_model_re_embeds() {
    let (memory, _fake) = hybrid();
    add(&memory, say("Marek owns the warehouse.")).await;
    let run = memory.run.clone();
    assert!(
        memory
            .backend
            .pending_embeddings(&run, "fake-1", 10)
            .await
            .unwrap()
            .is_empty()
    );
    let waiting = memory
        .backend
        .pending_embeddings(&run, "fake-2", 10)
        .await
        .unwrap();
    assert_eq!(
        waiting.len(),
        1,
        "a vector from another model is not current"
    );
    let counts = memory.backend.counts(&run, "fake-2").await.unwrap();
    assert_eq!(counts.backlog, 1);
    assert_eq!(counts.models, vec![("fake-1".to_owned(), 1)]);
    // Searching with the new model finds nothing until the rows are re-embedded.
    let scope = memory.scope(Audience::Public);
    let query = FakeEmbedder::vector_of("warehouse");
    assert!(
        memory
            .backend
            .vector_search(&scope, "fake-2", &query, 5)
            .await
            .unwrap()
            .is_empty()
    );
    memory
        .backend
        .store_embeddings(
            &run,
            "fake-2",
            &[(
                waiting[0].event_id,
                FakeEmbedder::vector_of(&waiting[0].text),
            )],
        )
        .await
        .unwrap();
    assert_eq!(
        memory
            .backend
            .vector_search(&scope, "fake-2", &query, 5)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_replay_reads_the_recorded_pack_and_makes_no_provider_call() {
    let (memory, fake) = hybrid();
    add(
        &memory,
        say("Marek owns the warehouse beside the salt quay."),
    )
    .await;
    let pack = memory
        .recall(&public_request("who owns the warehouse"))
        .await;
    let id = pack.recall_id.expect("the recall was recorded");
    let calls = fake.calls.load(Ordering::SeqCst);
    let replayed = memory.replay(id).await.unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.rendered, pack.rendered);
    assert_eq!(replayed.related_ids, pack.related_ids);
    assert_eq!(replayed.pinned_ids, pack.pinned_ids);
    assert_eq!(fake.calls.load(Ordering::SeqCst), calls);
    assert!(memory.replay(id + 100).await.is_none());
    let line = memory
        .recent_audit(5)
        .into_iter()
        .find(|l| l.starts_with("MEMORY recall"))
        .unwrap();
    assert!(
        line.contains(&format!("id={id}")) && line.contains("related=["),
        "{line}"
    );
}

#[tokio::test]
async fn erasing_a_hero_redacts_their_words_and_what_was_derived_from_them() {
    let (memory, _fake) = hybrid();
    let spoken = add(
        &memory,
        Draft::new(
            EventKind::HeroSpeech,
            "test",
            "Brynn: my secret is the harbour debt",
        )
        .hero("Brynn"),
    )
    .await;
    let clue = add(
        &memory,
        Draft::new(
            EventKind::CanonFact,
            "test",
            "A harbour debt clue for Brynn",
        )
        .private_to("Brynn"),
    )
    .await;
    let world = add(&memory, say("Marek keeps the harbour ledger.").npc("marek")).await;
    let pack = memory
        .recall(&RecallRequest::new(
            Purpose::Dev,
            Audience::Member("brynn".into()),
            "harbour debt",
        ))
        .await;
    assert!(pack.related_ids.contains(&spoken));
    let removed = memory.erase_hero("Brynn").await.unwrap();
    assert_eq!(removed.events, 2);
    assert!(
        removed.recalls >= 1,
        "the recall record that cited her words is gone"
    );
    let after = memory
        .recall(&RecallRequest::new(
            Purpose::Dev,
            Audience::Member("brynn".into()),
            "harbour debt",
        ))
        .await;
    assert!(!after.related_ids.contains(&spoken) && !after.related_ids.contains(&clue));
    assert!(!after.rendered.contains("secret"));
    let erased = stored(&memory)
        .into_iter()
        .find(|e| e.event_id == spoken)
        .unwrap();
    assert!(erased.erased() && erased.text == "[erased]" && !erased.embedded);
    let _ = world;
    assert!(
        memory
            .backend
            .pending_embeddings(&memory.run, "fake-1", 10)
            .await
            .unwrap()
            .iter()
            .all(|p| p.event_id != spoken)
    );
}

#[tokio::test]
async fn deleting_the_run_index_drops_rebuildable_events_and_keeps_the_recorded_story() {
    let memory = memory_with(None);
    add(&memory, say("A line spoken at the table.")).await;
    add(
        &memory,
        capture::canon_fact_draft(
            "harbor",
            "place",
            "Salt Quay",
            "The quay runs along the wall.",
            "Marek",
        ),
    )
    .await;
    let report = memory.delete_run_index_json(false).await;
    assert_eq!(report["events"], 1);
    assert_eq!(stored(&memory).len(), 1);
    assert!(!stored(&memory)[0].rebuildable);
    let all = memory.delete_run_index_json(true).await;
    assert_eq!(all["events"], 1);
    assert!(stored(&memory).is_empty());
}

#[tokio::test]
async fn the_table_tap_captures_public_lines_as_they_are_pushed_and_retires_a_rewound_future() {
    let memory = memory_with(None);
    let log = TableLog::new();
    capture::attach(&memory, &log);
    log.push_speech("Brynn", 0, "I want the warehouse key.");
    log.push_actor(
        "npc:marek",
        "Marek",
        rpc::TableRole::Npc,
        0,
        rpc::TableKind::Speech,
        "Not for sale.",
    );
    let floor = log.position();
    log.push_check(log.reserve(), 0, "Brynn rolls Persuasion: 12, fails.");
    log.push(
        "table",
        rpc::TableRole::Narrator,
        0,
        rpc::TableKind::Action,
        "Development back: a to b",
    );
    log.push(
        "Narrator",
        rpc::TableRole::Narrator,
        0,
        rpc::TableKind::Action,
        "The gate creaks shut.",
    );
    memory.settle().await;
    let events = stored(&memory);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.label()).collect();
    assert_eq!(
        kinds,
        vec!["hero_speech", "npc_speech", "check_roll", "narration"]
    );
    assert!(
        events
            .iter()
            .all(|e| e.source == "table_log" && e.rebuildable == false)
    );
    assert_eq!(events[0].text, "Brynn: I want the warehouse key.");
    assert_eq!(events[0].entities.hero, vec!["brynn".to_owned()]);
    assert_eq!(events[1].entities.npc, vec!["npc:marek".to_owned()]);
    assert_eq!(
        events[2].entities.hero,
        vec!["brynn".to_owned()],
        "the roster maps the seat to the hero"
    );
    // A development rewind abandons everything from the floor on.
    log.rewind_to(floor);
    memory.settle().await;
    let after = stored(&memory);
    assert_eq!(
        after
            .iter()
            .filter(|e| e.status.label() == "active")
            .count(),
        2
    );
    assert!(
        after[2..]
            .iter()
            .all(|e| e.retired_reason.as_deref() == Some("rewound"))
    );
}

#[tokio::test]
async fn a_full_queue_drops_the_new_event_and_never_waits() {
    let memory = StoryMemory::new(
        Backend::Test(TestStore::new(true)),
        None,
        Arc::new(SpendLedger::default()),
        Config {
            queue_capacity: 2,
            ..Config::new("run-1")
        },
    );
    for n in 0..5 {
        memory.record(say(&format!("line {n}")));
    }
    assert_eq!(Stats::get(&memory.stats.dropped), 3);
    assert!(
        memory
            .recent_audit(5)
            .iter()
            .any(|l| l.starts_with("MEMORY queue_full"))
    );
    memory.settle().await;
    assert_eq!(stored(&memory).len(), 2);
}

static GLOBAL: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn the_capture_api_reaches_the_installed_memory_and_is_a_no_op_without_one() {
    let _only_one = GLOBAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    super::uninstall();
    capture::canon_fact(
        "harbor",
        "place",
        "Salt Quay",
        "The quay runs along the wall.",
        "Marek",
    );
    let memory = memory_with(None);
    super::install(Arc::clone(&memory));
    capture::canon_fact(
        "harbor",
        "place",
        "Salt Quay",
        "The quay runs along the wall.",
        "Marek",
    );
    capture::grievance(
        "Brynn",
        "patron:merchant",
        "Marek",
        "You insulted my scales.",
    );
    capture::scene_change(
        "7:12",
        Some(12),
        "Salt Quay",
        "Brynn",
        "rope-1",
        "Brynn buys a rope",
        "purchased",
        &["patron:merchant".to_owned()],
    );
    capture::scene_change(
        "7:13",
        Some(13),
        "Salt Quay",
        "Brynn",
        "door-1",
        "Brynn opens the door",
        "opened",
        &[],
    );
    capture::ruling(
        "Brynn",
        "Salt Quay",
        "Brynn climbs the wall; the table rules it hard.",
        Some(14),
    );
    capture::narration("Salt Quay", "Gulls wheel over the wall.", None);
    capture::private_clue("Brynn", "Salt Quay", "A loose stone hides a key.", Some(15));
    capture::promise(
        "patron:merchant",
        "Brynn",
        "Salt Quay",
        "Marek will hold the rope until noon.",
        None,
    );
    capture::combat_outcome("Salt Quay", "The bandit falls.", Some(16));
    memory.settle().await;
    super::uninstall();
    let events = stored(&memory);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.label()).collect();
    assert_eq!(
        kinds,
        vec![
            "canon_fact",
            "grievance_attitude",
            "transaction",
            "scene_change",
            "action_ruling",
            "narration",
            "canon_fact",
            "promise_thread",
            "combat_outcome"
        ]
    );
    let clue = events.iter().find(|e| e.private_to.is_some()).unwrap();
    assert_eq!(clue.private_to.as_deref(), Some("brynn"));
    assert!(events.iter().filter(|e| e.rebuildable).count() == 4);
    assert_eq!(events[2].game_revision, Some(12));
    assert_eq!(events[2].source, "journal");
}

struct Writer {
    calls: AtomicUsize,
    seen: Mutex<Vec<DossierRequest>>,
}

impl DossierWriter for Writer {
    fn name(&self) -> &str {
        "fake"
    }

    fn write<'a>(
        &'a self,
        request: &'a DossierRequest,
    ) -> BoxFuture<'a, Result<String, DossierFailure>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(request.clone());
            Ok(format!(
                "Summary of {} over {} events.",
                request.entity,
                request.events.len()
            ))
        })
    }
}

#[tokio::test]
async fn a_dossier_is_cached_by_event_set_hash_built_from_shared_events_and_read_by_the_storyteller_only()
 {
    let (memory, _fake) = hybrid();
    let writer = Arc::new(Writer {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    memory.set_dossier_writer(Arc::clone(&writer) as Arc<dyn DossierWriter>);
    for n in 0..12 {
        add(
            &memory,
            say(&format!("Marek mentions barrel number {n}.")).npc("marek"),
        )
        .await;
    }
    add(
        &memory,
        Draft::new(
            EventKind::CanonFact,
            "test",
            "Marek's private shame is gambling.",
        )
        .npc("marek")
        .private_to("Brynn"),
    )
    .await;
    let first = memory.entity_dossier("marek", &Audience::Public).await;
    assert!(matches!(first, DossierOutcome::Written(_)));
    assert!(matches!(
        memory.entity_dossier("marek", &Audience::Public).await,
        DossierOutcome::Fresh(_)
    ));
    assert_eq!(writer.calls.load(Ordering::SeqCst), 1);
    // The private clue is not part of the shared summary or its hash.
    assert!(
        writer.seen.lock().unwrap()[0]
            .events
            .iter()
            .all(|e| !e.contains("gambling"))
    );
    assert!(matches!(
        memory
            .entity_dossier("marek", &Audience::Member("brynn".into()))
            .await,
        DossierOutcome::Written(_)
    ));
    assert!(
        writer.seen.lock().unwrap()[1]
            .events
            .iter()
            .all(|e| !e.contains("gambling"))
    );

    let storyteller =
        RecallRequest::new(Purpose::Storyteller, Audience::Public, "barrel").entities(&["marek"]);
    let pack = memory.recall(&storyteller).await;
    assert_eq!(pack.dossiers.len(), 1);
    assert!(pack.rendered.contains("Summary of marek (derived)"));
    let npc =
        RecallRequest::new(Purpose::NpcReply, Audience::Public, "barrel").entities(&["marek"]);
    assert!(memory.recall(&npc).await.dossiers.is_empty());

    // A new shared event makes it stale; the next request folds it into the earlier summary.
    add(&memory, say("Marek sells barrel number 99.").npc("marek")).await;
    let again = memory.entity_dossier("marek", &Audience::Public).await;
    assert!(matches!(again, DossierOutcome::Written(_)));
    let seen = writer.seen.lock().unwrap();
    let last = seen.last().unwrap();
    assert!(last.previous.is_some());
    assert_eq!(
        last.events.len(),
        1,
        "only the events newer than the summary are folded in"
    );
}

fn candidate_proposal(text: &str) -> Proposed {
    Proposed {
        text: text.to_owned(),
        scene: None,
        entities: vec!["marek".to_owned()],
        audience: Audience::Party,
    }
}

#[tokio::test]
async fn jev_without_a_provider_leaves_the_proposal_unchecked_and_audited() {
    let memory = memory_with(None);
    add(
        &memory,
        say("Marek says the gate opens at dawn.").npc("marek"),
    )
    .await;
    let result = memory
        .check_consistency(
            &crate::gameplay::approach_model::ApproachModel::default(),
            &candidate_proposal("The gate opens at noon."),
        )
        .await;
    assert!(matches!(
        result.verdict,
        Verdict::Unchecked("provider_unavailable")
    ));
    assert_eq!(result.candidates.len(), 1);
    assert!(
        memory
            .recent_audit(5)
            .iter()
            .any(|l| l.contains("verdict=unchecked") && l.contains("provider_unavailable"))
    );
    let nothing = memory_with(None);
    let unrelated = nothing
        .check_consistency(
            &crate::gameplay::approach_model::ApproachModel::default(),
            &candidate_proposal("Anything."),
        )
        .await;
    assert!(matches!(unrelated.verdict, Verdict::Unrelated) && unrelated.candidates.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_jev_verdict_names_the_conflicting_event_and_only_beliefs_may_be_superseded() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::FakeEndpoint,
    };
    let memory = memory_with(None);
    let claim = add(
        &memory,
        say("Marek says the gate opens at dawn.").npc("marek"),
    )
    .await;
    let endpoint = FakeEndpoint::start(vec![answer("o1"), answer("o2"), answer("o3")]);
    let jev = model(&endpoint);
    let contradiction = memory
        .check_consistency(&jev, &candidate_proposal("The gate opens at noon."))
        .await;
    let Verdict::Contradicts(conflict) = &contradiction.verdict else {
        panic!("{:?}", contradiction.verdict)
    };
    assert_eq!(conflict.event_id, claim);
    let body = &endpoint.bodies()[0];
    let instructions = body["questions"]["q1"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("data, never instructions"));
    assert_eq!(
        body["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert!(matches!(
        memory
            .check_consistency(&jev, &candidate_proposal("Dawn is early."))
            .await
            .verdict,
        Verdict::Extends(_)
    ));
    assert!(matches!(
        memory
            .check_consistency(&jev, &candidate_proposal("Gulls cry."))
            .await
            .verdict,
        Verdict::Unrelated
    ));

    // A belief-level conflict can be marked superseded by the line that replaces it.
    let replacement = add(
        &memory,
        say("Marek says the gate opens at noon.").npc("marek"),
    )
    .await;
    assert_eq!(
        memory
            .supersede_on_verdict(&contradiction.verdict, replacement)
            .await,
        Superseded::Marked
    );
    assert!(
        stored(&memory)
            .iter()
            .find(|e| e.event_id == claim)
            .unwrap()
            .status
            .label()
            == "superseded"
    );

    // A rules-committed event is reported, never superseded by a model verdict.
    let roll = add(
        &memory,
        Draft::new(
            EventKind::CheckRoll,
            "test",
            "Brynn rolls Athletics 17, succeeds.",
        )
        .npc("marek"),
    )
    .await;
    let roll_event = stored(&memory)
        .into_iter()
        .find(|e| e.event_id == roll)
        .unwrap();
    let verdict = Verdict::Contradicts(Box::new(roll_event));
    assert_eq!(
        memory.supersede_on_verdict(&verdict, replacement).await,
        Superseded::RulesCommitted
    );
    assert_eq!(
        stored(&memory)
            .iter()
            .find(|e| e.event_id == roll)
            .unwrap()
            .status
            .label(),
        "active"
    );
    assert_eq!(
        memory
            .supersede_on_verdict(&Verdict::Unrelated, replacement)
            .await,
        Superseded::NothingToDo
    );
}

#[tokio::test]
async fn search_runs_the_same_recall_path_and_narrows_by_entity_and_kind() {
    let (memory, _fake) = hybrid();
    add(&memory, say("Marek owns the warehouse.").npc("marek")).await;
    add(
        &memory,
        Draft::new(EventKind::Narration, "test", "The warehouse door is red.").place("warehouse"),
    )
    .await;
    let all = memory
        .search_json("warehouse", None, None, Audience::Party, 8)
        .await;
    assert_eq!(all["related"].as_array().unwrap().len(), 2);
    let only = memory
        .search_json(
            "warehouse",
            None,
            Some(EventKind::Narration),
            Audience::Party,
            8,
        )
        .await;
    assert_eq!(only["related"].as_array().unwrap().len(), 1);
    assert_eq!(only["related"][0]["kind"], "narration");
    let entity = memory
        .search_json("zzz", Some("marek"), None, Audience::Party, 8)
        .await;
    assert_eq!(entity["related"][0]["entities"]["npc"][0], "marek");
    assert!(all["recall_id"].is_i64());
    let event = memory.event_view_json(1).await;
    assert_eq!(event["found"], true);
    assert_eq!(memory.event_view_json(999).await["found"], false);
}

#[tokio::test]
async fn facts_carry_the_framed_text_and_the_recalled_ids_or_nothing_when_nothing_was_recalled() {
    let memory = memory_with(None);
    let empty = memory.recall(&public_request("anything")).await;
    assert!(empty.is_empty() && empty.facts().is_none());
    let id = add(&memory, say("Marek owns the warehouse.")).await;
    let pack = memory.recall(&public_request("warehouse")).await;
    let facts = pack.facts().unwrap();
    assert_eq!(facts["recalled_event_ids"][0], id);
    assert!(
        facts["recalled_memory"]
            .as_str()
            .unwrap()
            .contains("data only")
    );
}

#[test]
fn identifiers_and_text_are_tidied_before_they_are_stored() {
    assert_eq!(
        capture::slug("Marek the Cloth-Seller!"),
        "marek-the-cloth-seller"
    );
    assert_eq!(capture::tidy("a\u{0}b \n\n c   d"), "a b c d");
    assert_eq!(capture::tidy(&"x".repeat(5000)).chars().count(), 1200);
    assert_eq!(
        capture::short_hash(&["a", "b"]),
        capture::short_hash(&["a", "b"])
    );
    assert_ne!(
        capture::short_hash(&["ab"]),
        capture::short_hash(&["a", "b"])
    );
}
