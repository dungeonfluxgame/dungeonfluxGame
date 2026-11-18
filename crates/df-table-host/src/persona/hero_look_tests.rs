//! A seated hero's look reaches the briefs of the plates that draw them: their look studio's
//! design, details included, in place of the persona's job; a character who is no seated hero is
//! drawn by their job as before.
//!
//! TODO(DEV-12): `move act` and a walk on a host with a fake image backend: the move plate's and
//! each sight's prompt carry the hero's look.

use df_playground::look::wire::{BodyRegion, DetailKind, LookDetail, LookDraft};

use super::declare::{DeclaredAction, declared_scene};
use super::hero_look::{hero_look, hero_looks};
use crate::look::catalog::built_in_studio;
use crate::movement::brief::LookSheets;
use crate::seat::book::Holder;
use crate::seat::key::KeyHash;
use crate::test_support::{Running, STARTER_FILES, start};

const AURA: &str = "a blazing golden aura of light surrounding his whole body, with sparks";

/// A host with seat 1 holding a locked dwarf fighter whose look has the golden aura; answers the
/// hero's character id (`hero-<id>`).
async fn hero_with_an_aura() -> (Running, String) {
    let running = start(&STARTER_FILES).await;
    let key = KeyHash::of("seat-key-for-the-hero-look-brief-test");
    let mut draft = built_in_studio()
        .settle("dwarf", &LookDraft::default())
        .unwrap();
    draft.details.push(LookDetail {
        kind: DetailKind::Other,
        place: BodyRegion::WholeBody,
        what: AURA.to_owned(),
    });
    let hero = {
        let mut open = running.state.seats().open().await.unwrap();
        let books = open.books();
        books.seat(&key, 1, Holder::Guest, "Ann", 0);
        let card = books.add_hero(1, "Thorin", "dwarf", "fighter").unwrap();
        books.choose_hero(&key, card.id).unwrap();
        books.lock_hero(card.id, None, Some(draft)).unwrap();
        card.id
    };
    (running, crate::creation::rules::sheet_id(hero))
}

#[tokio::test]
async fn a_seated_heros_look_details_reach_the_brief_and_anyone_else_keeps_their_job() {
    let (running, hero) = hero_with_an_aura().await;
    let state = &running.state;
    let look = hero_look(state, &hero)
        .await
        .expect("a locked hero has a look");
    assert!(look.contains(AURA), "{look}");
    assert!(!look.contains("Thorin"), "never a name: {look}");
    assert_eq!(hero_look(state, "innkeeper").await, None);

    let looks = hero_looks(state, &[hero.clone(), "innkeeper".to_owned()]).await;
    let sheets = LookSheets::from_looks(looks).unwrap();
    assert!(
        sheets
            .look_of(&hero)
            .is_some_and(|look| look.contains(AURA))
    );
    assert_eq!(
        sheets.look_of("innkeeper"),
        None,
        "the brief keeps their job"
    );
    let logged: Vec<String> = state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PERSONA hero_look "))
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].starts_with(&format!("PERSONA hero_look id={hero} chars=")));

    // The declared picture describes the hero by that look.
    let action = DeclaredAction {
        doing: "sets a board on the easel".to_owned(),
        pose: None,
        items_in_use: Vec::new(),
    };
    let scene = declared_scene(sheets.look_of(&hero).unwrap(), &action, &[], 1);
    assert!(scene.contains(AURA), "{scene}");
}
