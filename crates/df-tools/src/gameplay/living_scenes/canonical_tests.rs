use super::*;
use crate::gameplay::{
    hero_identity::Identity, imagery::tests::DirectionFixture, startup_configuration::ImageryOff,
};
use std::path::PathBuf;

fn fact_decision(
    place: &str,
    person: &str,
    kind: crate::gameplay::npc_reply::local_lore::FactKind,
    name: &str,
    content: &str,
) -> crate::gameplay::npc_reply::fact_backfill::FactDecision {
    use crate::gameplay::npc_reply::{
        fact_backfill::{Decision, FactDecision, Reason},
        local_lore::LocalFact,
    };
    FactDecision {
        scene: "market".to_owned(),
        place: place.to_owned(),
        npc_id: person.to_owned(),
        speaker: "Bram Weller".to_owned(),
        question_hash: "a".repeat(64),
        decision: Decision::Backfill,
        reason: Reason::HarmlessMissingDetail,
        fact: Some(LocalFact {
            kind,
            name: name.to_owned(),
            content: content.to_owned(),
            established_by: "Bram Weller".to_owned(),
        }),
        answer: None,
    }
}

#[tokio::test]
async fn committed_room_canon_reaches_the_fake_and_changes_the_reference_once() {
    use crate::gameplay::npc_reply::{
        fact_backfill::{Decision, Reason},
        local_lore::{FactKind, LocalLore},
    };
    let lore = Arc::new(LocalLore::in_memory());
    let mut canonical = references("hs66-room-canon");
    canonical.lore = Some(lore.clone());
    room(&mut canonical, "layout");
    let original_binding = selected_room_binding(&canonical).to_owned();
    let jobs = canonical.take_jobs();
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS66/fake-room-canon")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&cache);
    let inner = inner(fixture.imagery.clone(), canonical);
    for job in jobs {
        let Job::Canonical(work) = job else {
            panic!("canonical room")
        };
        let result = inner.render_canonical(&work).await;
        assert!(result.is_ok(), "{result:?}");
        inner.finish_canonical(work, result);
    }
    assert!(!fixture.prompt().contains("The Last Stitch"));
    let proposal = fact_decision(
        "same-place",
        "bram",
        FactKind::History,
        "Shop sign",
        "Bram Weller's shop is called The Last Stitch, as its sign says.",
    );
    // A model proposal that has not crossed commit never reaches the picture source.
    {
        let mut state = locked(&inner.state);
        room(&mut state.canonical, "layout");
        assert_eq!(selected_room_binding(&state.canonical), original_binding);
        assert!(state.canonical.take_jobs().is_empty());
    }
    let foreign = fact_decision(
        "other-place",
        "other-person",
        FactKind::History,
        "Other shop",
        "The other shop is called FOREIGN_SHOP.",
    );
    lore.commit_fact_decision(foreign.clone(), foreign.fact.clone())
        .unwrap();
    let mut blank = proposal.clone();
    blank.decision = Decision::LeaveBlank;
    blank.reason = Reason::Secret;
    blank.fact = None;
    lore.commit_fact_decision(blank, None).unwrap();
    {
        let mut state = locked(&inner.state);
        room(&mut state.canonical, "layout");
        assert_eq!(selected_room_binding(&state.canonical), original_binding);
        assert!(state.canonical.take_jobs().is_empty());
    }
    lore.commit_fact_decision(proposal.clone(), proposal.fact.clone())
        .unwrap();
    let base_revision = room_revision(&{
        let mut authored = model();
        authored.weather = "layout".to_owned();
        authored
    })
    .unwrap();
    let jobs = {
        let mut state = locked(&inner.state);
        room(&mut state.canonical, "layout");
        assert_ne!(selected_room_binding(&state.canonical), original_binding);
        let jobs = state.canonical.take_jobs();
        assert_eq!(jobs.len(), 1);
        jobs
    };
    for job in jobs {
        let Job::Canonical(work) = job else {
            panic!("canonical room")
        };
        let result = inner.render_canonical(&work).await;
        assert!(result.is_ok(), "{result:?}");
        inner.finish_canonical(work, result);
    }
    let prompt = fixture.prompt();
    assert!(
        prompt.contains("The Last Stitch, as its sign says."),
        "{prompt}"
    );
    assert!(prompt.contains("visible sign carries the recorded name exactly"));
    assert!(!prompt.contains("No signage and no writing"));
    assert!(!prompt.contains("FOREIGN_SHOP"));
    assert!(!prompt.contains("question_hash"));
    assert!(!prompt.contains("harmless_missing_detail"));
    assert!(!prompt.contains("protected_constraints"));
    assert_eq!(
        inner.canonical_room_status_at("same-place", &base_revision),
        ReferenceStatus::Ready
    );
    assert!(
        inner
            .canonical_room_reference_at("same-place", &base_revision)
            .await
            .is_some()
    );
    {
        let mut state = locked(&inner.state);
        room(&mut state.canonical, "layout");
        assert!(state.canonical.take_jobs().is_empty());
    }
    assert_eq!(fixture.calls(), 2);
}

#[test]
fn person_canon_uses_the_exact_journal_owner_and_excludes_history() {
    use crate::gameplay::npc_reply::local_lore::{FactKind, LocalLore};
    let lore = Arc::new(LocalLore::in_memory());
    let mut canonical = references("hs66-person-canon");
    canonical.lore = Some(lore.clone());
    for decision in [
        fact_decision(
            "same-place",
            "bram",
            FactKind::Person,
            "Appearance",
            "Bram wears a blue wool cap.",
        ),
        fact_decision(
            "same-place",
            "tessa",
            FactKind::Person,
            "Other appearance",
            "Tessa wears a red scarf.",
        ),
        fact_decision(
            "same-place",
            "bram",
            FactKind::History,
            "History",
            "Bram once lived beside the river.",
        ),
    ] {
        lore.commit_fact_decision(decision.clone(), decision.fact.clone())
            .unwrap();
    }
    let canon = canonical.committed_canon(None, Some("bram"));
    assert_eq!(canon.len(), 1);
    assert_eq!(canon[0]["content"], "Bram wears a blue wool cap.");
    assert_eq!(canon[0]["person_id"], "bram");
    assert!(!serde_json::to_string(&canon).unwrap().contains("reason"));
}

#[test]
fn cast_presentation_is_bound_and_missing_values_are_audited() {
    let mut canonical = references("hs66-cast-presentation");
    let mut member = Member {
        id: "hs66-presentation".to_owned(),
        role: CharacterRole::Contact,
        life_stage: LifeStage::Adult,
        gender: Some(pack::Gender::They),
        age: Some(pack::Age::Adult),
        name: "Oren Vale".to_owned(),
        sprite: String::new(),
        dossier_id: None,
        opposes: false,
        starting_attitude: None,
    };
    canonical.admit_cast(&member);
    let target = Target::Npc(member.id.clone());
    let first = canonical.entries[&target].work.binding.clone();
    member.gender = Some(pack::Gender::She);
    canonical.admit_cast(&member);
    assert_ne!(canonical.entries[&target].work.binding, first);
    let second = canonical.entries[&target].work.binding.clone();
    member.age = Some(pack::Age::Elder);
    canonical.admit_cast(&member);
    assert_ne!(canonical.entries[&target].work.binding, second);
    for (gender, age) in [
        (None, Some(pack::Age::Adult)),
        (Some(pack::Gender::They), None),
    ] {
        member.gender = gender;
        member.age = age;
        canonical.admit_cast(&member);
        assert!(
            !crate::gameplay::actor::audit_lines_containing(
                "IMAGERY presentation_missing person=hs66-presentation"
            )
            .is_empty()
        );
    }
}

#[tokio::test]
async fn custom_seats_use_full_public_design_and_only_their_approved_references() {
    use hero_identity_fixture::{LOOKS, checkpoint, portrait_asset, portrait_bytes};
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS5/canonical-custom-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&cache);
    let inner = inner(fixture.imagery.clone(), References::default());
    inner.observe_checkpoint(&checkpoint(true));
    let jobs = locked(&inner.state).canonical.take_jobs();
    assert_eq!(jobs.len(), 2, "one canonical image per accepted seat");
    let portrait_root = std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("artifacts/tmp/play/imagery-cache"));
    std::fs::create_dir_all(&portrait_root).unwrap();
    std::fs::write(
        portrait_root.join(format!("{}.jpg", portrait_asset().key.as_str())),
        portrait_bytes(),
    )
    .unwrap();
    for (index, job) in jobs.into_iter().enumerate() {
        let Job::Canonical(work) = job else {
            panic!("canonical work")
        };
        let Source::Hero {
            portrait,
            public_design,
            locator,
            ..
        } = &work.source
        else {
            panic!("hero source")
        };
        assert_eq!(portrait.is_some(), index == 0);
        assert!(public_design.contains(if index == 0 { "dragonborn" } else { "tiefling" }));
        assert!(locator.contains(if index == 0 { "Brakka" } else { "Sigrun" }));
        let key = inner.render_canonical(&work).await.unwrap();
        assert_eq!(fixture.image_count(), usize::from(index == 0));
        let prompt = fixture.prompt();
        let design: serde_json::Value = serde_json::from_str(public_design).unwrap();
        let appearance = design["public_appearance"]
            .as_str()
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let appearance: serde_json::Value = serde_json::from_str(appearance).unwrap();
        assert_eq!(appearance["public_visual_details"], LOOKS[index]);
        assert!(prompt.contains(public_design));
        assert!(prompt.contains("actual_gear"));
        assert!(prompt.contains(if index == 0 { "greatsword" } else { "longbow" }));
        assert!(!prompt.contains("SEALED_STORY_NEVER_IMAGE"));
        assert!(!prompt.contains("PRIVATE_ORIGIN_NEVER_IMAGE"));
        assert!(!prompt.contains("SCENERY_NEVER_CANONICAL"));
        assert!(!prompt.contains("long braided beard with plain iron rings"));
        inner.finish_canonical(work, Ok(key));
    }
    assert_eq!(fixture.calls(), 2);
    inner.observe_checkpoint(&checkpoint(true));
    assert!(locked(&inner.state).canonical.take_jobs().is_empty());
    // Mutable inventory is not a new identity or permission to buy another canonical image.
    let accepted = checkpoint(true);
    let mut changed = accepted.state().clone();
    changed.inventory[0].quantity += 1;
    inner.observe_checkpoint(&hero_identity_fixture::recheckpoint(
        accepted.basis(),
        changed,
    ));
    assert!(locked(&inner.state).canonical.take_jobs().is_empty());
}

#[tokio::test]
async fn legacy_quickstart_uses_only_its_written_look_without_a_stock_reference() {
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS5/canonical-legacy-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&cache);
    let inner = inner(fixture.imagery.clone(), References::default());
    let checkpoint = hero_identity_fixture::checkpoint(false);
    let mut saved = checkpoint.state().clone();
    for character in &mut saved.characters {
        character
            .choices
            .retain(|choice| !choice.offer.as_str().starts_with("identity-"));
    }
    let saved = crate::gameplay::model::checkpoint(checkpoint.basis(), saved).unwrap();
    inner.observe_checkpoint(&saved);
    let Job::Canonical(work) = locked(&inner.state).canonical.take_jobs().remove(0) else {
        panic!("canonical hero")
    };
    inner.render_canonical(&work).await.unwrap();
    assert_eq!(fixture.image_count(), 0);
    assert!(
        fixture
            .prompt()
            .contains("long braided beard with plain iron rings")
    );
}

#[test]
fn accepted_personal_art_reaches_party_sheet_combat_and_live_faces_without_stock_fallbacks() {
    let inner = inner(
        Imagery::disabled(ImageryOff::Qualification),
        References::default(),
    );
    let committed = hero_identity_fixture::checkpoint(true);
    inner.observe_checkpoint(&committed);
    let party = committed
        .state()
        .characters
        .iter()
        .enumerate()
        .map(|(index, hero)| rpc::PartyMemberView {
            name: ["Brakka", "Sigrun"][index].to_owned(),
            participant_id: hero.entity.as_bytes().to_vec(),
            character_ready: true,
            side: 1,
            portrait: "portrait-hero-1".to_owned(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut journey = rpc::JourneyView {
        party: party.clone(),
        own_character: Some(rpc::CharacterSheet {
            name: "Sigrun".to_owned(),
            ..Default::default()
        }),
        combat: Some(rpc::CombatView {
            participants: party,
            ..Default::default()
        }),
        ..Default::default()
    };
    {
        let state = locked(&inner.state);
        apply_hero_faces(&state, Some(&mut journey));
    }
    assert_eq!(
        journey.party[0].portrait,
        format!("/gameplay/creator-portrait/{}", "4".repeat(64))
    );
    assert!(
        journey.party[1].portrait.is_empty(),
        "no photo without an approved or ready personal image"
    );
    let jobs = locked(&inner.state).canonical.take_jobs();
    for (index, job) in jobs.into_iter().enumerate() {
        let Job::Canonical(work) = job else {
            panic!("hero");
        };
        inner.finish_canonical(
            work,
            Ok(ImageKey::parse(&["a", "b"][index].repeat(64)).unwrap()),
        );
    }
    apply_hero_faces(&locked(&inner.state), Some(&mut journey));
    for (index, member) in journey.party.iter().enumerate() {
        // An approved creator portrait remains the hero's accepted depiction after
        // a generated reference becomes ready. Other heroes use their ready reference.
        let path = if index == 0 {
            format!("/gameplay/creator-portrait/{}", "4".repeat(64))
        } else {
            format!("/gameplay/imagery/{}", ["a", "b"][index].repeat(64))
        };
        assert_eq!(member.portrait, path);
        assert_eq!(journey.live_faces[index].sprite, path);
        assert_eq!(journey.live_faces[index].battle_sprite, path);
        assert_eq!(
            journey.combat.as_ref().unwrap().participants[index].portrait,
            path
        );
        assert_eq!(
            df_render::art_contract::player_hero_image(&path),
            Some(path.as_str())
        );
    }
    assert_eq!(
        journey.own_character.unwrap().portrait_uri,
        journey.party[1].portrait
    );
    for stock in [
        "portrait-hero-1",
        "figure-hero-2",
        "sprite-hero-3",
        "assets/ui/generated/figure-hero-1-documentary-photo",
    ] {
        assert!(
            df_render::art_contract::fetch_target(stock).is_none(),
            "{stock}"
        );
    }
}

fn model() -> SceneModel {
    SceneModel {
        services: Vec::new(),
        witnessed_thefts: Vec::new(),
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: Vec::new(),
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    }
}

fn references(scope: &str) -> References {
    References {
        scope: scope.to_owned(),
        ..Default::default()
    }
}

fn room(references: &mut References, revision: &str) {
    let mut authored = model();
    authored.weather = revision.to_owned();
    references.admit_room("same-place", &authored, &[], true);
}

fn selected_room_binding(references: &References) -> &str {
    let target = references.selected_rooms.get("same-place").unwrap();
    &references.entries.get(target).unwrap().work.binding
}

#[tokio::test]
async fn authored_marek_identity_uses_approved_portrait_and_full_body_photos() {
    use crate::gameplay::{
        adventure::scene_model::StartingAttitude,
        pack::{Age, CastRole, Gender, SceneCastMember},
    };

    let mut authored = model();
    authored.people.push(ScenePerson {
            passive_perception: 10,
            persona: None,
            id: "merchant".to_owned(),
            name: "Marek".to_owned(),
            role: "cloth merchant".to_owned(),
            description: "Marek, a short, round middle-aged man with a neatly trimmed beard, wears a rich velvet cap and a measuring ribbon around his neck.".to_owned(),
            relation_to_speaker: "shopkeeper".to_owned(),
            position: "behind the wooden counter".to_owned(),
            aliases: vec!["Marek".to_owned()],
            is_background: false,
            present_in: vec!["opening".to_owned()],
            attitude: StartingAttitude::Friendly,
            attitudes: Default::default(),
            knowledge: Vec::new(),
            wants: String::new(),
            coin_cp: 0,
        });
    let cast = [SceneCastMember {
        id: "merchant".to_owned(),
        name: "Marek".to_owned(),
        role: CastRole::Patron,
        sprite: "assets/ui/generated/sprite-patron-merchant-documentary-photo".to_owned(),
        portrait: "assets/ui/generated/portrait-patron-merchant-documentary-photo".to_owned(),
        voice: "warm".to_owned(),
        gender: Some(Gender::He),
        age: Some(Age::Adult),
        stat_block: None,
    }];

    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS37/fixture-cache/canonical-reference-tests/marek-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&cache);
    let mut canonical = references("shop-run");
    canonical.admit_room("mareks-cloth-shop", &authored, &cast, true);
    let work = canonical
        .take_jobs()
        .into_iter()
        .find_map(|job| match job {
            Job::Canonical(work) if work.target == Target::Npc("merchant".to_owned()) => Some(work),
            _ => None,
        })
        .expect("Marek identity job");
    let Source::Npc {
        person,
        identity_photo,
        identity_figure_photo,
        ..
    } = &work.source
    else {
        panic!("Marek NPC identity source");
    };

    assert!(person.description.contains("short, round middle-aged man"));
    assert_eq!(identity_photo.as_deref(), Some("portrait-patron-merchant"));
    assert_eq!(
        identity_figure_photo.as_deref(),
        Some("sprite-patron-merchant")
    );
    let store = ReferenceStore::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
    );
    assert!(store.load(identity_photo.as_deref().unwrap()).is_ok());
    assert!(
        store
            .load(identity_figure_photo.as_deref().unwrap())
            .is_ok()
    );

    let inner = inner(fixture.imagery.clone(), canonical);
    inner.render_canonical(&work).await.unwrap();
    assert_eq!(fixture.calls(), 1);
    // Only the two real photographs are sent; there is no blank carrier before them.
    assert_eq!(fixture.image_count(), 2);
    let prompt = fixture.prompt();
    assert!(prompt.contains("Image 1 is the approved documentary face portrait"));
    assert!(prompt.contains("Image 2 is the approved full-body photograph"));
}

fn inner(imagery: Imagery, canonical: References) -> Inner {
    let timing = Timing::from_interval(Duration::from_secs(30));
    let mut state = State::new(timing);
    state.canonical = canonical;
    Inner {
        imagery,
        references: ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(state),
        wake: watch::channel(0).0,
        origin: Instant::now(),
    }
}

#[test]
fn run_pack_layout_and_entity_identity_are_distinct_and_unrelated_observations_dedupe() {
    let mut first = references("session-a/run-a/pack-a");
    room(&mut first, "layout-a");
    let first_binding = selected_room_binding(&first).to_owned();
    assert_eq!(first.take_jobs().len(), 1);
    room(&mut first, "layout-a");
    assert!(first.take_jobs().is_empty());
    room(&mut first, "layout-b");
    assert_ne!(selected_room_binding(&first), first_binding);
    let mut second = references("session-a/run-b/pack-a");
    room(&mut second, "layout-a");
    assert_ne!(selected_room_binding(&second), first_binding);
    second.scope = "session-a/run-a/pack-b".to_owned();
    room(&mut second, "layout-a");
    assert_ne!(selected_room_binding(&second), first_binding);
    let source = Source::Hero {
        name: "Test hero".to_owned(),
        identity: Box::new(Identity::from_menus(&[]).unwrap()),
        species: "orc".to_owned(),
        gear: vec!["a greatsword"],
        public_design: String::new(),
        portrait: None,
        locator: "Test hero".to_owned(),
    };
    first.select(
        Target::Hero(EntityId::from_bytes(&[1; 16]).unwrap()),
        source.clone(),
        "same-name/same-appearance",
    );
    first.select(
        Target::Hero(EntityId::from_bytes(&[2; 16]).unwrap()),
        source,
        "same-name/same-appearance",
    );
    let bindings = first
        .entries
        .iter()
        .filter(|(target, _)| matches!(target, Target::Hero(_)))
        .map(|(_, entry)| entry.work.binding.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        bindings.len(),
        2,
        "same display name must never merge durable entities"
    );
}

#[test]
fn stale_run_completion_cannot_replace_current_room_and_failure_is_terminal_for_the_binding() {
    let mut canonical = references("run-a");
    room(&mut canonical, "layout-a");
    let Job::Canonical(old) = canonical.take_jobs().remove(0) else {
        panic!("canonical job");
    };
    let mut replacement = references("run-b");
    room(&mut replacement, "layout-b");
    let inner = inner(Imagery::disabled(ImageryOff::Qualification), replacement);
    inner.finish_canonical(old, Ok(ImageKey::parse(&"a".repeat(64)).unwrap()));
    assert_eq!(
        inner.canonical_room_status("same-place"),
        ReferenceStatus::Pending
    );
    assert!(inner.canonical_room_key("same-place").is_none());
    let Job::Canonical(current) = locked(&inner.state).canonical.take_jobs().remove(0) else {
        panic!("canonical job");
    };
    inner.finish_canonical(current, Err(ImageFailure::Provider));
    assert_eq!(
        inner.canonical_room_status("same-place"),
        ReferenceStatus::Failed(ImageFailure::Provider)
    );
    room(&mut locked(&inner.state).canonical, "layout-b");
    assert!(
        locked(&inner.state).canonical.take_jobs().is_empty(),
        "ordinary updates cannot retry paid failures"
    );
}

#[tokio::test]
async fn owned_completion_exposes_runtime_bytes_and_identical_recovery_uses_the_cache() {
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS37/fixture-cache/canonical-reference-tests/runtime-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::new(&cache);
    let mut canonical = references("runtime-test-run");
    room(&mut canonical, "accepted-layout");
    let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
        panic!("canonical job");
    };
    let inner = Arc::new(inner(fixture.imagery.clone(), canonical));
    let mut tasks = JoinSet::new();
    inner.spawn(&mut tasks, Job::Canonical(work.clone()));
    inner.finish(tasks.join_next().await.unwrap().unwrap());
    let key = inner.canonical_room_key("same-place").unwrap();
    for _ in 0..3 {
        room(&mut locked(&inner.state).canonical, "accepted-layout");
        assert!(
            locked(&inner.state).canonical.take_jobs().is_empty(),
            "reobserving accepted authoring preserves ready work"
        );
        assert_eq!(inner.canonical_room_key("same-place"), Some(key.clone()));
    }
    let loaded = inner.canonical_room_reference("same-place").await.unwrap();
    let Lookup::Image { bytes, format } = fixture.imagery.image(key.as_str()).await else {
        panic!("runtime bytes");
    };
    assert_eq!(loaded.sha256, ReferenceImage::new(format, bytes).sha256);
    assert_eq!(inner.render_canonical(&work).await.unwrap(), key);
    assert_eq!(fixture.calls(), 1, "cache hit must not pay twice");
}

#[tokio::test]
async fn combat_creature_missing_pending_and_late_ready_share_real_cached_art() {
    // The cache is written during the test, so it is a scratch copy of the committed art.
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/gameplay/fixtures/creature-art");
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS37/fixture-cache/creature-art-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&cache).unwrap();
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), cache.join(entry.file_name())).unwrap();
    }
    let fixture = DirectionFixture::new(&cache);
    let canonical = references("combat-creature-run");
    let inner = Arc::new(inner(fixture.imagery.clone(), canonical));
    let mut journey = rpc::JourneyView {
        combat: Some(rpc::CombatView {
            participants: vec![
                rpc::PartyMemberView {
                    name: "Skeleton 1".to_owned(),
                    creature_id: "skeleton".to_owned(),
                    side: 2,
                    ..Default::default()
                },
                rpc::PartyMemberView {
                    name: "Skeleton 2".to_owned(),
                    creature_id: "skeleton".to_owned(),
                    side: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let jobs = {
        let mut state = locked(&inner.state);
        apply_creature_faces(&mut state.canonical, Some(&mut journey));
        assert!(
            journey.live_faces.is_empty(),
            "pending art cannot fabricate a URL"
        );
        let jobs = state.canonical.take_jobs();
        assert_eq!(jobs.len(), 1, "one request per creature identity");
        apply_creature_faces(&mut state.canonical, Some(&mut journey));
        assert!(state.canonical.take_jobs().is_empty());
        jobs
    };
    let mut tasks = JoinSet::new();
    for job in jobs {
        inner.spawn(&mut tasks, job);
    }
    inner.finish(tasks.join_next().await.unwrap().unwrap());
    apply_creature_faces(&mut locked(&inner.state).canonical, Some(&mut journey));
    assert_eq!(journey.live_faces.len(), 2);
    let key = locked(&inner.state)
        .canonical
        .npc("combat-creature:skeleton")
        .unwrap();
    assert!(matches!(
        fixture.imagery.image(key.as_str()).await,
        Lookup::Image { .. }
    ));
    assert!(
        journey
            .live_faces
            .iter()
            .all(|face| face.sprite == key.path() && face.battle_sprite == key.path())
    );
}

#[test]
fn same_place_authored_layouts_coexist_and_select_the_exact_current_room() {
    let mut canonical = references("same-run-pack");
    let morning = model();
    let mut evening = morning.clone();
    evening.time_of_day = "evening".to_owned();
    let morning_revision = room_revision_with_context(&morning, None).unwrap();
    let evening_revision = room_revision_with_context(&evening, None).unwrap();
    canonical.admit_room_with_asset_and_context(
        "shared-place",
        &morning,
        &[],
        true,
        (None, None, None),
    );
    canonical.admit_room_with_asset_and_context(
        "shared-place",
        &evening,
        &[],
        true,
        (None, None, None),
    );
    assert_eq!(canonical.entries.len(), 2);
    let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
    let work = locked(&inner.state).canonical.take_jobs();
    assert_eq!(work.len(), imagery::MAXIMUM_IN_FLIGHT);
    for (index, job) in work.into_iter().enumerate() {
        let Job::Canonical(work) = job else {
            panic!("canonical work");
        };
        inner.finish_canonical(
            work,
            Ok(ImageKey::parse(&format!("{:064x}", index + 1)).unwrap()),
        );
    }
    let state = locked(&inner.state);
    let first = state
        .canonical
        .room_at("shared-place", &morning_revision)
        .unwrap();
    let second = state
        .canonical
        .room_at("shared-place", &evening_revision)
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(state.canonical.room("shared-place"), Some(second));
    drop(state);
    locked(&inner.state)
        .canonical
        .admit_room_with_asset_and_context("shared-place", &morning, &[], true, (None, None, None));
    assert_eq!(inner.canonical_room_key("shared-place"), Some(first));
    assert!(
        locked(&inner.state).canonical.take_jobs().is_empty(),
        "returning to accepted room reuses its pack"
    );
}

#[test]
fn room_reference_revision_ignores_cast_absent_from_the_room_prompt() {
    let original = model();
    let mut changed_cast = original.clone();
    changed_cast.people.push(ScenePerson {
        passive_perception: 10,
        persona: None,
        id: "merchant".to_owned(),
        name: "Marek".to_owned(),
        role: "cloth merchant".to_owned(),
        description: "Marek folds blue linen behind the counter.".to_owned(),
        relation_to_speaker: "shopkeeper".to_owned(),
        position: "behind the wooden counter".to_owned(),
        aliases: vec!["Marek".to_owned()],
        is_background: false,
        present_in: vec!["room".to_owned()],
        attitude: crate::gameplay::adventure::scene_model::StartingAttitude::Friendly,
        attitudes: Default::default(),
        knowledge: Vec::new(),
        wants: "Sell suitable clothing at a fair price.".to_owned(),
        coin_cp: 0,
    });

    assert_eq!(
        room_revision_with_context(&original, None),
        room_revision_with_context(&changed_cast, None),
        "cast changes belong to character references; the canonical room is empty"
    );
    let brightwater = imagery::RoomArchitectureContext {
        name: "Brightwater".to_owned(),
        character: "pale stone houses and striped awnings".to_owned(),
    };
    let eastmere = imagery::RoomArchitectureContext {
        name: "Eastmere".to_owned(),
        character: "red brick houses and timber awnings".to_owned(),
    };
    assert_ne!(
        room_revision_with_context(&original, Some(&brightwater)),
        room_revision_with_context(&original, Some(&eastmere)),
        "a changed city architecture context regenerates the room"
    );
}

#[test]
fn actual_room_admission_preserves_terminal_provider_outcomes_on_reobservation() {
    for failure in [
        ImageFailure::Provider,
        ImageFailure::Refused,
        ImageFailure::Interrupted,
    ] {
        let mut canonical = references("terminal-admission-run");
        room(&mut canonical, "unchanged-authored-layout");
        let binding = selected_room_binding(&canonical).to_owned();
        let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
            panic!("canonical work");
        };
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
        inner.finish_canonical(work, Err(failure));
        for _ in 0..3 {
            room(
                &mut locked(&inner.state).canonical,
                "unchanged-authored-layout",
            );
            assert_eq!(
                inner.canonical_room_status("same-place"),
                ReferenceStatus::Failed(failure)
            );
            assert_eq!(
                selected_room_binding(&locked(&inner.state).canonical),
                binding
            );
            assert!(
                locked(&inner.state).canonical.take_jobs().is_empty(),
                "accepted unchanged authoring cannot redispatch a terminal paid job"
            );
        }
    }
}

#[test]
fn canonical_dispatch_obeys_the_existing_media_capacity_and_cancellation_is_terminal() {
    let mut canonical = references("capacity-run");
    for place in ["one", "two", "three"] {
        canonical.admit_room(place, &model(), &[], true);
    }
    assert_eq!(canonical.take_jobs_up_to(1).len(), 1);
    assert!(canonical.take_jobs_up_to(0).is_empty());
    let inner = Arc::new(inner(
        Imagery::disabled(ImageryOff::Qualification),
        canonical,
    ));
    drop(WorkerLifetime(inner.clone()));
    assert_eq!(
        inner.canonical_room_status("one"),
        ReferenceStatus::Failed(ImageFailure::Interrupted)
    );
    assert!(locked(&inner.state).canonical.take_jobs().is_empty());
}

#[test]
fn a_created_place_is_admitted_like_the_shop_with_a_room_photo_and_each_person_by_this_path() {
    use crate::gameplay::{
        place,
        place_creation::{self, tests as made},
    };
    let record = place_creation::build(
        &made::request(),
        &made::place_slot().to_string(),
        &made::people_slot().to_string(),
        &made::things_slot().to_string(),
    )
    .unwrap();
    let scene = place::scene_of(&record, &pack::scene(pack::SceneKey::Opening)).unwrap();
    let mut canonical = references("run-with-a-created-place");
    canonical.observe_room(&scene);
    // The room is the place's own, the current one, and each person has an identity to make.
    let room = canonical
        .selected_rooms
        .get("place-market-street")
        .cloned()
        .expect("the street's room is admitted");
    assert_eq!(canonical.current_room, Some(room));
    let people: BTreeSet<String> = canonical
        .entries
        .keys()
        .filter_map(|target| match target {
            Target::Npc(id) => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        people,
        [
            "market-street-hobb",
            "market-street-wenna",
            "market-street-yara"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    assert_eq!(canonical.current_npcs, people);
    // What the photograph is asked for is the street's own layout, never the shop's.
    let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
    let context = room_architecture_context("place-market-street");
    let prompt = imagery::canonical_room_with_context(
        ContentRating::Restrained,
        model,
        "place-market-street",
        context.as_ref(),
        &[],
        "binding",
    )
    .unwrap();
    assert!(prompt.as_str().contains("spice stall"));
    assert!(!prompt.as_str().contains("wooden counter"));
    let hobb = model
        .people
        .iter()
        .find(|person| person.name == "Hobb")
        .unwrap();
    let person = imagery::canonical_npc(
        ContentRating::Restrained,
        hobb,
        (None, None, &[] as &[_]).into(),
        false,
        false,
        "b",
    )
    .unwrap();
    assert!(person.as_str().contains("broad carter"));
}
#[test]
fn rejected_room_can_be_readmitted_after_cooldown_once_without_replacing_its_binding() {
    let mut canonical = references("room-retry-run");
    room(&mut canonical, "stable-layout");
    let binding = selected_room_binding(&canonical).to_owned();
    let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
        panic!("room job");
    };
    let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
    inner.finish_canonical(work, Err(ImageFailure::BadOutput));
    let mut state = locked(&inner.state);
    room(&mut state.canonical, "stable-layout");
    assert!(
        state.canonical.take_jobs().is_empty(),
        "no immediate paid retry loop"
    );
    let target = state.canonical.selected_rooms["same-place"].clone();
    state.canonical.entries.get_mut(&target).unwrap().started =
        Some(Instant::now() - ROOM_RETRY_COOLDOWN);
    room(&mut state.canonical, "stable-layout");
    room(&mut state.canonical, "stable-layout");
    let jobs = state.canonical.take_jobs();
    assert_eq!(jobs.len(), 1);
    let Job::Canonical(work) = &jobs[0] else {
        panic!("room job");
    };
    assert!(work.regenerate);
    assert_eq!(work.binding, binding);
}

#[tokio::test]
async fn authored_place_admission_skips_room_generation_but_keeps_people_and_direct_references() {
    let mut scene = pack::scene(pack::SceneKey::Opening);
    scene.asset = format!("/assets/ui/generated/{}.webp", art_contract::HARBOR_PHOTO);
    let model: SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../../content/merchant-shop/scene-model.json"
    ))
    .unwrap();
    scene.setting = Some(crate::gameplay::adventure::setting::SceneSetting {
        place_id: "HS37-authored-quay".into(),
        name: "Working quay".into(),
        short: "Weathered boards beside the water".into(),
        kind: crate::gameplay::adventure::setting::SettingKind::Quay,
        exterior: true,
        art_key: scene.asset.clone(),
        model: Some(model.clone()),
    });
    let place = scene.setting.as_ref().unwrap().place_id.clone();
    let context = room_architecture_context(&place);
    let revision = room_revision_with_context(&model, context.as_ref()).unwrap();
    let mut canonical = references("HS37-authored-admission");
    canonical.observe_room(&scene);
    // An ahead admission without another asset cannot resurrect room generation.
    canonical.admit_room(&place, &model, &scene.cast, false);
    let jobs = canonical.take_jobs_up_to(model.people.len() + 1);
    assert_eq!(
        jobs.len(),
        model.people.len() + 1,
        "NPC work plus one local photo load"
    );
    assert!(
        !canonical
            .entries
            .values()
            .any(|entry| matches!(entry.work.source, Source::Room { .. }))
    );
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/authored-registration")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&cache);
    let inner = inner(fixture.imagery.clone(), canonical);
    assert_eq!(
        inner.canonical_room_status(&place),
        ReferenceStatus::Pending
    );
    for job in jobs {
        let Job::Canonical(work) = job else {
            panic!("canonical work")
        };
        if matches!(work.source, Source::ReviewedRoom { .. }) {
            let key = inner.render_canonical(&work).await.unwrap();
            inner.finish_canonical(work, Ok(key));
        } else {
            assert!(matches!(work.source, Source::Npc { .. }));
        }
    }
    assert_eq!(
        fixture.calls(),
        0,
        "authored room registration never invokes a provider"
    );
    let registered = inner.canonical_room_key(&place).unwrap();
    assert!(matches!(
        fixture.imagery.image(registered.as_str()).await,
        Lookup::Image { .. }
    ));
    assert_eq!(inner.canonical_room_status(&place), ReferenceStatus::Ready);
    assert_eq!(
        inner.canonical_room_status_at(&place, &revision),
        ReferenceStatus::Ready
    );
    let expected = inner.load(scene.asset).await.unwrap();
    assert_eq!(
        inner.canonical_room_reference(&place).await.unwrap().sha256,
        expected.sha256
    );
    assert_eq!(
        inner
            .canonical_room_reference_at(&place, &revision)
            .await
            .unwrap()
            .sha256,
        expected.sha256
    );
}

#[test]
fn authored_room_retires_a_queued_generation_and_generic_reference_still_generates() {
    let mut canonical = references("HS37-authored-replacement");
    canonical.admit_room_with_asset(
        "place",
        &model(),
        &[],
        true,
        Some(art_contract::ROOM_PHOTO),
        None,
    );
    assert!(
        canonical
            .entries
            .values()
            .any(|entry| matches!(entry.work.source, Source::Room { .. }))
    );
    canonical.admit_room_with_asset(
        "place",
        &model(),
        &[],
        true,
        Some(art_contract::HARBOR_PHOTO),
        None,
    );
    let jobs = canonical.take_jobs();
    assert_eq!(jobs.len(), 1);
    assert!(
        matches!(&jobs[0], Job::Canonical(work) if matches!(work.source, Source::ReviewedRoom { .. }))
    );
    assert_eq!(canonical.authored_rooms.len(), 1);
}

#[test]
fn prepared_room_is_resolvable_without_changing_current_room_or_duplicating_work() {
    let mut canonical = references("prepared-room-run");
    canonical.admit_room("shop", &model(), &[], true);
    let current = canonical.current_room.clone();
    canonical.admit_room("street", &model(), &[], false);
    canonical.admit_room("street", &model(), &[], false);
    assert_eq!(canonical.current_room, current);
    assert_eq!(canonical.entries.len(), 2);
    assert!(canonical.selected_rooms.contains_key("street"));
    let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
    let jobs = locked(&inner.state).canonical.take_jobs();
    for job in jobs {
        let Job::Canonical(work) = job else {
            panic!("room job");
        };
        if matches!(&work.target, Target::Room { place, .. } if place == "street") {
            // Drop the test's state lock before reporting a completion.
            let key = ImageKey::parse(&"b".repeat(64)).unwrap();
            let mut state = locked(&inner.state);
            state
                .canonical
                .entries
                .get_mut(&work.target)
                .unwrap()
                .status = Status::Ready(key.clone());
            assert_eq!(state.canonical.room("street"), Some(key));
        }
    }
}
