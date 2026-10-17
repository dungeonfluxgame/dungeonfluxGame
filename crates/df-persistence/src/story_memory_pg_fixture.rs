//! The story memory against a real PostgreSQL, with pgvector when it is installed.
//!
//! Runs only when `DUNGEONFLUX_STORY_MEMORY_PG_PORT` names a scratch cluster on 127.0.0.1 (and
//! optionally `_DATABASE`, default `postgres`, and `_USER`, default `memtest`); otherwise it
//! prints a skip message and passes. It never touches the game's own cluster: it works in the
//! `df_story` schema under a unique run id and removes its rows. When the cluster has no
//! pgvector, the semantic checks are skipped with a message and the full-text ones still run.

use crate::story_memory::{
    Audience, DossierRow, EMBEDDING_DIMENSIONS, Entities, EventKind, NewEvent, RecallRecord,
    Relevance, Scope, Status, StoryMemoryDb, Visibility,
};
use tokio_postgres::{Config, NoTls};

fn scratch_config() -> Option<Config> {
    let port: u16 = std::env::var("DUNGEONFLUX_STORY_MEMORY_PG_PORT")
        .ok()?
        .parse()
        .ok()?;
    let database = std::env::var("DUNGEONFLUX_STORY_MEMORY_PG_DATABASE")
        .unwrap_or_else(|_| "postgres".to_owned());
    let user =
        std::env::var("DUNGEONFLUX_STORY_MEMORY_PG_USER").unwrap_or_else(|_| "memtest".to_owned());
    // The game's own development cluster is off limits.
    if port == 0 || port == 55517 || port == 55439 {
        return None;
    }
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .dbname(&database)
        .user(&user);
    Some(config)
}

fn event(run: &str, key: &str, kind: EventKind, text: &str) -> NewEvent {
    NewEvent {
        run_id: run.to_owned(),
        natural_key: key.to_owned(),
        source: "pg_fixture".to_owned(),
        source_epoch: Some("boot".to_owned()),
        source_seq: Some(1),
        game_revision: None,
        kind,
        visibility: Visibility::Public,
        private_to: None,
        text: text.to_owned(),
        scene: Some("harbor".to_owned()),
        entities: Entities {
            npc: vec!["marek".to_owned()],
            ..Entities::default()
        },
        rebuildable: false,
        supersedes: None,
    }
}

fn axis(position: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[position] = 1.0;
    vector
}

async fn raw(config: &Config) -> tokio_postgres::Client {
    let (client, connection) = config.connect(NoTls).await.expect("raw connection");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

#[test]
fn story_memory_on_a_real_postgres() {
    let Some(config) = scratch_config() else {
        eprintln!(
            "SKIPPED story_memory_on_a_real_postgres: set DUNGEONFLUX_STORY_MEMORY_PG_PORT to a scratch \
             PostgreSQL (not the game's cluster) to run it"
        );
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let db = StoryMemoryDb::connect(&config)
            .await
            .expect("connect and migrate");
        // Migrations are idempotent: a second start changes nothing and fails nothing.
        let db_again = StoryMemoryDb::connect(&config)
            .await
            .expect("second migration");
        drop(db_again);
        let vector = db.vector_enabled();
        if !vector {
            eprintln!(
                "SKIPPED the semantic half: pgvector unavailable ({}); full-text checks still run",
                db.vector_reason().unwrap_or("unknown")
            );
        }
        let run = format!(
            "pgtest-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let public = event(
            &run,
            "k-public",
            EventKind::NpcSpeech,
            "Marek owns the warehouse by the salt quay.",
        );
        let mut party = event(
            &run,
            "k-party",
            EventKind::CanonFact,
            "The party knows the warehouse key is brass.",
        );
        party.visibility = Visibility::Party;
        let mut private = event(
            &run,
            "k-private",
            EventKind::CanonFact,
            "Secret warehouse clue: the false wall hides a ledger.",
        );
        private.visibility = Visibility::Private;
        private.private_to = Some("brynn".to_owned());
        let mut rebuildable = event(
            &run,
            "k-lore",
            EventKind::CanonFact,
            "The quay runs along the harbour wall.",
        );
        rebuildable.rebuildable = true;
        rebuildable.entities.hero = vec!["brynn".to_owned()];

        let first = db
            .insert_events(&[
                public.clone(),
                party.clone(),
                private.clone(),
                rebuildable.clone(),
            ])
            .await
            .unwrap();
        assert!(first.iter().all(|row| row.created));
        let again = db
            .insert_events(&[
                public.clone(),
                party.clone(),
                private.clone(),
                rebuildable.clone(),
            ])
            .await
            .unwrap();
        assert!(
            again.iter().all(|row| !row.created),
            "a replay adds nothing"
        );
        assert_eq!(
            first.iter().map(|r| r.event_id).collect::<Vec<_>>(),
            again.iter().map(|r| r.event_id).collect::<Vec<_>>()
        );
        let [public_id, party_id, private_id, lore_id] = [
            first[0].event_id,
            first[1].event_id,
            first[2].event_id,
            first[3].event_id,
        ];

        // Audience is part of the query: full text.
        let ids = |found: &[crate::story_memory::Scored]| {
            found.iter().map(|s| s.event_id).collect::<Vec<_>>()
        };
        let public_scope = Scope::active(&run, Audience::Public);
        let found = db
            .text_search(&public_scope, "warehouse clue ledger key", 10)
            .await
            .unwrap();
        assert_eq!(ids(&found), vec![public_id]);
        let party_found = db
            .text_search(
                &Scope::active(&run, Audience::Party),
                "warehouse clue ledger key",
                10,
            )
            .await
            .unwrap();
        assert!(ids(&party_found).contains(&party_id) && !ids(&party_found).contains(&private_id));
        let member = Scope::active(&run, Audience::Member("brynn".to_owned()));
        let member_found = db
            .text_search(&member, "warehouse clue ledger key", 10)
            .await
            .unwrap();
        assert!(ids(&member_found).contains(&private_id));
        let other = Scope::active(&run, Audience::Member("tobin".to_owned()));
        assert!(
            !ids(&db
                .text_search(&other, "warehouse clue ledger", 10)
                .await
                .unwrap())
            .contains(&private_id)
        );
        // Punctuation and operators in player text change nothing.
        assert!(
            db.text_search(&public_scope, "\"warehouse\" & !quay | ' ; --", 10)
                .await
                .is_ok()
        );

        let selected = db
            .select_events(
                &public_scope,
                &Relevance {
                    entities: vec!["marek".to_owned()],
                    scene: None,
                    unattached: false,
                },
                10,
            )
            .await
            .unwrap();
        assert_eq!(
            selected.iter().map(|e| e.event_id).collect::<Vec<_>>(),
            vec![lore_id, public_id]
        );
        assert!(selected.iter().all(|e| e.visibility == Visibility::Public));
        let by_ids = db
            .events_by_ids(&public_scope, &[private_id, public_id])
            .await
            .unwrap();
        assert_eq!(
            by_ids.len(),
            1,
            "an id outside the audience is not returned"
        );

        if vector {
            let pending = db.pending_embeddings(&run, "test-model", 10).await.unwrap();
            assert_eq!(pending.len(), 4);
            let stored = db
                .store_embeddings(
                    &run,
                    "test-model",
                    &[
                        (public_id, axis(0)),
                        (party_id, axis(1)),
                        (private_id, axis(2)),
                        (lore_id, axis(3)),
                    ],
                )
                .await
                .unwrap();
            assert_eq!(stored, 4);
            assert!(
                db.pending_embeddings(&run, "test-model", 10)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                db.pending_embeddings(&run, "other-model", 10)
                    .await
                    .unwrap()
                    .len(),
                4,
                "a changed model re-embeds"
            );
            let nearest = db
                .vector_search(&public_scope, "test-model", &axis(0), 5)
                .await
                .unwrap();
            assert_eq!(nearest[0].event_id, public_id);
            assert!((nearest[0].score - 1.0).abs() < 1e-6);
            let leaked = db
                .vector_search(&public_scope, "test-model", &axis(2), 5)
                .await
                .unwrap();
            assert!(
                !ids(&leaked).contains(&private_id),
                "a private vector is not even a candidate"
            );
            let own = db
                .vector_search(&member, "test-model", &axis(2), 5)
                .await
                .unwrap();
            assert_eq!(own[0].event_id, private_id);
            assert!(
                db.vector_search(&public_scope, "other-model", &axis(0), 5)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                db.store_embeddings(&run, "m", &[(public_id, vec![0.0; 3])])
                    .await
                    .is_err()
            );
        }

        // Supersession links, status, and nothing deleted.
        let newer = db
            .insert_events(&[event(
                &run,
                "k-newer",
                EventKind::NpcSpeech,
                "Marek sold the warehouse to the carter.",
            )])
            .await
            .unwrap()[0]
            .event_id;
        assert!(db.supersede(&run, public_id, newer).await.unwrap());
        assert!(!db.supersede(&run, public_id, newer).await.unwrap());
        assert!(db.supersede(&run, public_id, 987_654_321).await.is_err());
        let active = db
            .text_search(&public_scope, "warehouse", 10)
            .await
            .unwrap();
        assert!(!ids(&active).contains(&public_id) && ids(&active).contains(&newer));
        let mut with_old = public_scope.clone();
        with_old.include_superseded = true;
        let all = db.entity_events(&with_old, "marek", 0, 10).await.unwrap();
        let old = all.iter().find(|e| e.event_id == public_id).unwrap();
        assert_eq!(
            (old.status, old.superseded_by),
            (Status::Superseded, Some(newer))
        );
        assert_eq!(
            all.iter()
                .find(|e| e.event_id == newer)
                .unwrap()
                .supersedes_id,
            Some(public_id)
        );
        let hash_input = db
            .entity_event_ids(&with_old, "marek")
            .await
            .unwrap()
            .unwrap();
        assert!(
            hash_input
                .iter()
                .any(|(id, status)| *id == public_id && *status == Status::Superseded)
        );

        // The table refuses to be rewritten or emptied outside maintenance.
        let client = raw(&config).await;
        let refused = client
            .execute(
                "UPDATE df_story.story_memory SET body = 'changed' WHERE event_id = $1",
                &[&newer],
            )
            .await
            .unwrap_err();
        assert!(
            refused
                .as_db_error()
                .unwrap()
                .message()
                .contains("immutable"),
            "{refused:?}"
        );
        let refused = client
            .execute(
                "DELETE FROM df_story.story_memory WHERE event_id = $1",
                &[&newer],
            )
            .await
            .unwrap_err();
        assert!(
            refused
                .as_db_error()
                .unwrap()
                .message()
                .contains("append-only"),
            "{refused:?}"
        );
        assert!(
            client
                .execute(
                    "UPDATE df_story.story_memory SET status = 'active' WHERE event_id = $1",
                    &[&public_id]
                )
                .await
                .is_err()
        );
        let counts = db.counts(&run, "test-model").await.unwrap();
        assert_eq!(counts.total, 5);
        assert_eq!(counts.rebuildable, 1);

        // Recall and dossier records.
        let record = RecallRecord {
            run_id: run.clone(),
            purpose: "npc_reply".to_owned(),
            scene: Some("harbor".to_owned()),
            speaker: Some("marek".to_owned()),
            addressee: Some("brynn".to_owned()),
            audience: "public".to_owned(),
            query_text: "who owns the warehouse".to_owned(),
            mode: if vector { "hybrid" } else { "text_only" }.to_owned(),
            embedding_model: vector.then(|| "test-model".to_owned()),
            query_vector_digest: vector.then(|| "a".repeat(64)),
            pinned_ids: vec![lore_id],
            related_ids: vec![newer],
            scores: vec![0.5],
            pack_text: "RECALLED MEMORY ...".to_owned(),
        };
        let recall_id = db.log_recall(&record).await.unwrap();
        assert_eq!(db.recall(&run, recall_id).await.unwrap(), Some(record));
        let dossier = DossierRow {
            entity_id: "marek".to_owned(),
            audience: "public".to_owned(),
            event_set_hash: "b".repeat(64),
            covered_ids: vec![newer],
            summary: "Marek sold the warehouse.".to_owned(),
            writer: "test".to_owned(),
        };
        db.put_dossier(&run, &dossier).await.unwrap();
        db.put_dossier(&run, &dossier).await.unwrap();
        assert_eq!(
            db.dossier(&run, "marek", "public").await.unwrap(),
            Some(dossier)
        );

        // Erasure: the hero's private clue is redacted, the vector and the derived records go.
        let removed = db.erase_hero(&run, "brynn").await.unwrap();
        assert_eq!(removed.events, 1);
        assert_eq!(
            removed.recalls, 1,
            "the recall record addressed to her is dropped"
        );
        let erased = db.operator_event(&run, private_id).await.unwrap().unwrap();
        assert!(erased.erased() && erased.text == "[erased]" && !erased.embedded);
        assert!(
            db.text_search(&member, "false hides ledger", 10)
                .await
                .unwrap()
                .is_empty()
        );
        if vector {
            assert!(
                db.pending_embeddings(&run, "test-model", 10)
                    .await
                    .unwrap()
                    .iter()
                    .all(|p| p.event_id != private_id)
            );
        }

        // A rewind retires a source's future without deleting it.
        let retired = db
            .retire_source_since(&run, "pg_fixture", "boot", 1, "rewound")
            .await
            .unwrap();
        assert!(retired >= 3);
        assert!(
            db.text_search(&Scope::active(&run, Audience::Party), "quay", 10)
                .await
                .unwrap()
                .is_empty()
        );

        // Dropping the index: rebuildable rows first, then everything.
        let removed = db.delete_run(&run, true).await.unwrap();
        assert_eq!(removed.events, 1);
        assert_eq!(db.counts(&run, "m").await.unwrap().total, 4);
        let removed = db.delete_run(&run, false).await.unwrap();
        assert_eq!(removed.events, 4);
        assert_eq!(db.counts(&run, "m").await.unwrap().total, 0);
    });
}
