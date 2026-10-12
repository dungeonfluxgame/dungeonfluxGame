#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/gameplay_browser/view_delivery.rs"]
mod view_delivery;
// `view_delivery`'s own tests merge generated art with the display's real merge.
#[allow(dead_code)]
#[path = "../src/live_art.rs"]
mod live_art;
// PLAY-V09: the presentation module's scene plate layout uses the bubble placement types.
#[allow(dead_code)]
#[path = "../src/plate_layout.rs"]
mod plate_layout;
#[allow(dead_code)]
#[path = "../src/presentation.rs"]
mod presentation;

use df_client::{
    cache::CacheScope, connection::RpcConnection, connection_views::ConnectionViewAcceptance,
    revisions::ViewAcceptance,
};
use df_protocol::common as rpc;
use df_types::{ClientBindingId, RecoveryEpoch, RunId, SessionId, SessionRevision};
use prost::Message;
use view_delivery::{GameplayViews, ViewDeliveryError, ViewRole, ViewScope};

fn scope(session: u8, run: u8, binding: u8, role: ViewRole) -> ViewScope {
    ViewScope::from_wire(
        Some(&rpc::SessionId {
            value: Some(vec![session; 16]),
        }),
        Some(&rpc::RunId {
            value: Some(vec![run; 16]),
        }),
        Some(&rpc::ClientBindingId {
            value: Some(vec![binding; 16]),
        }),
        role,
    )
    .expect("nonzero fixture scope")
}

fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}

fn frame(epoch: u64, sequence: u64, narration: &str) -> rpc::ViewMessage {
    let message = rpc::ViewMessage {
        revision: Some(rpc::SessionRevision {
            epoch: Some(rpc::RecoveryEpoch { value: Some(epoch) }),
            sequence: Some(sequence),
        }),
        audience: Some(rpc::view_message::Audience::Player(
            rpc::PlayerGameplayView {
                narration: narration.to_owned(),
                scene: Some(rpc::GameplayScene {
                    scene_asset: "assets/ui/scenes/mara-harbor-v4.png".to_owned(),
                    ..Default::default()
                }),
                journey: Some(rpc::JourneyView {
                    phase: rpc::JourneyPhase::Opening as i32,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    };
    rpc::ViewMessage::decode(message.encode_to_vec().as_slice()).unwrap()
}

#[test]
fn replaced_live_generation_and_disposed_owner_cannot_deliver_queued_newer_frames() {
    let scope = scope(1, 2, 3, ViewRole::Player);
    let old = RpcConnection::new(());
    let old_generation = old.generation();
    let mut views = GameplayViews::new(scope, old_generation.clone());
    let initial = frame(2, 7, "retained current view");
    assert_eq!(
        views.accept(&old_generation, scope, initial.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );

    let replacement = RpcConnection::new(());
    let current_generation = replacement.generation();
    assert!(views.reconnect(current_generation.clone()));
    assert!(old_generation.is_active());
    assert_eq!(
        views.accept(
            &old_generation,
            scope,
            frame(9, 99, "late retired callback")
        ),
        Ok(ConnectionViewAcceptance::ObsoleteGeneration)
    );
    assert_eq!(views.current(), Some(&initial));

    let current = frame(2, 8, "replacement view");
    assert_eq!(
        views.accept(&current_generation, scope, current.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    drop(replacement);
    assert!(!current_generation.is_active());
    assert_eq!(
        views.accept(
            &current_generation,
            scope,
            frame(9, 999, "queued after disposal")
        ),
        Ok(ConnectionViewAcceptance::ObsoleteGeneration)
    );
    assert!(!views.reconnect(current_generation));
    assert_eq!(views.current(), Some(&current));
}

#[test]
fn reconnect_retains_wire_view_and_durable_recovery_watermark() {
    let scope = scope(1, 2, 3, ViewRole::Player);
    let old = RpcConnection::new(());
    let mut views = GameplayViews::new(scope, old.generation());
    assert_eq!(
        views.accept(&old.generation(), scope, frame(2, 99, "before recovery")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    drop(old);
    let owner = RpcConnection::new(());
    let generation = owner.generation();
    assert!(views.reconnect(generation.clone()));
    assert_eq!(views.current_revision(), Some(revision(2, 99)));
    let recovered = frame(3, 0, "current recovered view");
    assert_eq!(
        views.accept(&generation, scope, recovered.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    for stale in [
        frame(2, u64::MAX, "old epoch"),
        frame(1, u64::MAX, "older epoch"),
    ] {
        assert_eq!(
            views.accept(&generation, scope, stale),
            Ok(ConnectionViewAcceptance::View(ViewAcceptance::Stale {
                current: revision(3, 0),
            }))
        );
        assert_eq!(views.current(), Some(&recovered));
        assert_eq!(views.current_revision(), Some(revision(3, 0)));
    }
    assert_eq!(
        views.accept(&generation, scope, frame(3, 0, "changed duplicate")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Duplicate {
            current: revision(3, 0),
        }))
    );
    // Receipt/busy redraws borrow this exact retained wire revision for next input.
    assert_eq!(views.current(), Some(&recovered));
    assert_eq!(views.current_revision(), Some(revision(3, 0)));
}

#[test]
fn cross_scope_and_invalid_wire_frames_cannot_seed_or_replace_private_view() {
    let permitted = scope(1, 2, 3, ViewRole::Player);
    assert_eq!(
        permitted.cache_scope(),
        CacheScope {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            binding: ClientBindingId::from_bytes(&[3; 16]).unwrap(),
        }
    );
    let owner = RpcConnection::new(());
    let generation = owner.generation();
    let mut views = GameplayViews::new(permitted, generation.clone());
    for other in [
        scope(4, 2, 3, ViewRole::Player),
        scope(1, 4, 3, ViewRole::Player),
        scope(1, 2, 4, ViewRole::Player),
        scope(1, 2, 3, ViewRole::Display),
    ] {
        assert_eq!(
            views.accept(&generation, other, frame(9, 99, "other scope")),
            Err(ViewDeliveryError::WrongScope)
        );
        assert_eq!(views.current(), None);
        assert_eq!(views.current_revision(), None);
    }
    let current = frame(3, 8, "permitted private view");
    assert_eq!(
        views.accept(&generation, permitted, current.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    assert_eq!(
        views.accept(&generation, permitted, frame(3, 7, "old same-epoch frame")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Stale {
            current: revision(3, 8),
        }))
    );
    assert_eq!(
        views.accept(
            &generation,
            scope(1, 2, 4, ViewRole::Player),
            frame(9, 99, "wrong binding")
        ),
        Err(ViewDeliveryError::WrongScope)
    );
    assert_eq!(views.current(), Some(&current));
    let mut absent_revision = frame(9, 99, "missing revision");
    absent_revision.revision = None;
    let mut absent_sequence = frame(9, 99, "missing sequence");
    absent_sequence.revision.as_mut().unwrap().sequence = None;
    let mut absent_epoch = frame(9, 99, "missing epoch");
    absent_epoch.revision.as_mut().unwrap().epoch = None;
    for invalid in [
        absent_revision,
        absent_sequence,
        absent_epoch,
        frame(0, 99, "zero epoch"),
    ] {
        assert_eq!(
            views.accept(&generation, permitted, invalid),
            Err(ViewDeliveryError::InvalidRevision)
        );
        assert_eq!(views.current(), Some(&current));
        assert_eq!(views.current_revision(), Some(revision(3, 8)));
    }
    let mut wrong_audience = frame(9, 99, "wrong audience");
    wrong_audience.audience = Some(rpc::view_message::Audience::Display(Default::default()));
    assert_eq!(
        views.accept(&generation, permitted, wrong_audience),
        Err(ViewDeliveryError::WrongAudience)
    );
    assert_eq!(views.current(), Some(&current));
    let mut missing_journey = frame(9, 99, "incomplete projection");
    if let Some(rpc::view_message::Audience::Player(player)) = missing_journey.audience.as_mut() {
        player.journey = None;
    }
    assert_eq!(
        views.accept(&generation, permitted, missing_journey),
        Err(ViewDeliveryError::InvalidProjection)
    );
    assert_eq!(views.current(), Some(&current));
}

#[test]
fn display_projection_and_scope_use_the_same_generated_contract() {
    let scope = scope(1, 2, 3, ViewRole::Display);
    let owner = RpcConnection::new(());
    let generation = owner.generation();
    let mut views = GameplayViews::new(scope, generation.clone());
    let mut display = frame(1, 0, "public display");
    display.audience = Some(rpc::view_message::Audience::Display(
        rpc::DisplayGameplayView {
            journey: Some(rpc::JourneyView::default()),
            narration: "public display".to_owned(),
            ..Default::default()
        },
    ));
    assert_eq!(
        views.accept(&generation, scope, display.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    assert_eq!(views.current(), Some(&display));
    assert_eq!(
        ViewScope::from_wire(None, None, None, ViewRole::Display),
        Err(ViewDeliveryError::InvalidScope)
    );
}

#[test]
fn replacing_authorized_scope_starts_with_no_previous_private_view() {
    let old_scope = scope(1, 2, 3, ViewRole::Player);
    let old = RpcConnection::new(());
    let old_generation = old.generation();
    let mut views = GameplayViews::new(old_scope, old_generation.clone());
    assert_eq!(
        views.accept(&old_generation, old_scope, frame(9, 99, "old private view")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    drop(old);
    let new_scope = scope(4, 5, 6, ViewRole::Player);
    let new = RpcConnection::new(());
    views = GameplayViews::new(new_scope, new.generation());
    assert_eq!(views.scope(), new_scope);
    assert_eq!(views.current(), None);
    assert_eq!(views.current_revision(), None);
    assert_eq!(
        views.accept(
            &old_generation,
            old_scope,
            frame(9, 100, "old queued private view")
        ),
        Ok(ConnectionViewAcceptance::ObsoleteGeneration)
    );
    assert_eq!(views.current(), None);
    assert_eq!(
        views.accept(
            &new.generation(),
            new_scope,
            frame(1, 0, "new permitted scope")
        ),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
}

fn talking(sequence: u64, presentation_sequence: u64, replying: &str) -> rpc::ViewMessage {
    let mut message = frame(1, sequence, "The courier waits.");
    if let Some(rpc::view_message::Audience::Player(player)) = message.audience.as_mut()
        && let Some(journey) = player.journey.as_mut()
    {
        journey.presentation_sequence = presentation_sequence;
        journey.reply_speaker = replying.to_owned();
    }
    message
}

fn current_reply_speaker(views: &GameplayViews) -> String {
    match views.current().and_then(|view| view.audience.as_ref()) {
        Some(rpc::view_message::Audience::Player(player)) => player
            .journey
            .as_ref()
            .map(|journey| journey.reply_speaker.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[test]
fn table_talk_reaches_the_screen_without_a_new_game_revision() {
    let scope = scope(1, 2, 3, ViewRole::Player);
    let connection = RpcConnection::new(());
    let generation = connection.generation();
    let mut views = GameplayViews::new(scope, generation.clone());

    assert_eq!(
        views.accept(&generation, scope, talking(7, 3, "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    // The same revision again, nothing said: still a duplicate.
    assert!(matches!(
        views.accept(&generation, scope, talking(7, 3, "")),
        Ok(ConnectionViewAcceptance::View(
            ViewAcceptance::Duplicate { .. }
        ))
    ));
    // A hero speaks and a reply starts: the revision is the same but the room must see it.
    assert_eq!(
        views.accept(&generation, scope, talking(7, 4, "Courier")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    assert_eq!(current_reply_speaker(&views), "Courier");
    // The reply lands, still the same revision.
    assert_eq!(
        views.accept(&generation, scope, talking(7, 5, "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    assert_eq!(current_reply_speaker(&views), "");
    // An older revision is still stale, and a newer one still supersedes the talk.
    assert!(matches!(
        views.accept(&generation, scope, talking(6, 9, "Courier")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Stale { .. }))
    ));
    assert_eq!(
        views.accept(&generation, scope, talking(8, 0, "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    assert_eq!(
        views.current_revision(),
        Some(revision(1, 8)),
        "the newer revision is the current one"
    );
}

fn display_frame(sequence: u64, narration: &str, backdrop: &str) -> rpc::ViewMessage {
    let mut view = frame(1, sequence, narration);
    view.audience = Some(rpc::view_message::Audience::Display(
        rpc::DisplayGameplayView {
            narration: narration.to_owned(),
            scene: Some(rpc::GameplayScene {
                live_backdrop: backdrop.to_owned(),
                ..Default::default()
            }),
            journey: Some(rpc::JourneyView::default()),
            ..Default::default()
        },
    ));
    view
}

/// Copies the backdrop only, as the display's real merge does for the art fields.
fn merge_backdrop(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    let (
        Some(rpc::view_message::Audience::Display(shown)),
        Some(rpc::view_message::Audience::Display(new)),
    ) = (shown.audience.as_mut(), new.audience.as_ref())
    else {
        return false;
    };
    let (Some(shown), Some(new)) = (shown.scene.as_mut(), new.scene.as_ref()) else {
        return false;
    };
    let changed = shown.live_backdrop != new.live_backdrop;
    shown.live_backdrop.clone_from(&new.live_backdrop);
    changed
}

#[test]
fn an_equal_revision_changes_the_shown_view_only_through_the_art_merge() {
    let display = scope(1, 2, 3, ViewRole::Display);
    let owner = RpcConnection::new(());
    let generation = owner.generation();
    let mut views = GameplayViews::new(display, generation.clone());
    assert_eq!(
        views.accept(&generation, display, display_frame(5, "shown", "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    // The same revision with other content stays a duplicate and is not shown.
    let newer_content = display_frame(5, "different narration", "/gameplay/imagery/x");
    assert_eq!(
        views.accept(&generation, display, newer_content.clone()),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Duplicate {
            current: revision(1, 5)
        }))
    );
    // Its art alone is merged into the view on screen; the narration is not.
    assert!(views.accept_art_update(&generation, display, &newer_content, merge_backdrop));
    assert_eq!(
        views.current(),
        Some(&display_frame(5, "shown", "/gameplay/imagery/x"))
    );
    assert_eq!(views.current_revision(), Some(revision(1, 5)));
    assert!(!views.accept_art_update(&generation, display, &newer_content, merge_backdrop));
    // Another revision, another scope role and a retired connection are refused.
    assert!(!views.accept_art_update(
        &generation,
        display,
        &display_frame(6, "later", "/gameplay/imagery/y"),
        merge_backdrop
    ));
    let player = scope(1, 2, 3, ViewRole::Player);
    assert!(!views.accept_art_update(
        &generation,
        player,
        &display_frame(5, "shown", "/gameplay/imagery/z"),
        merge_backdrop
    ));
    drop(owner);
    assert!(!views.accept_art_update(
        &generation,
        display,
        &display_frame(5, "shown", "/gameplay/imagery/z"),
        merge_backdrop
    ));
}

fn talking_display(sequence: u64, presentation_sequence: u64, sprite: &str) -> rpc::ViewMessage {
    let mut view = display_frame(sequence, "The courier waits.", "");
    if let Some(rpc::view_message::Audience::Display(display)) = view.audience.as_mut() {
        if let Some(scene) = display.scene.as_mut() {
            scene.cast = vec![rpc::SceneActor {
                id: "merchant".to_owned(),
                live_sprite: sprite.to_owned(),
                ..Default::default()
            }];
        }
        if let Some(journey) = display.journey.as_mut() {
            journey.presentation_sequence = presentation_sequence;
        }
    }
    view
}

/// Copies the cast's generated sprites only, as the display's real merge does for the art fields.
fn merge_sprite(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    let (
        Some(rpc::view_message::Audience::Display(shown)),
        Some(rpc::view_message::Audience::Display(new)),
    ) = (shown.audience.as_mut(), new.audience.as_ref())
    else {
        return false;
    };
    let (Some(shown), Some(new)) = (shown.scene.as_mut(), new.scene.as_ref()) else {
        return false;
    };
    let mut changed = false;
    for (old, new) in shown.cast.iter_mut().zip(&new.cast) {
        changed |= old.live_sprite != new.live_sprite;
        old.live_sprite.clone_from(&new.live_sprite);
    }
    changed
}

fn shown_sprite(views: &GameplayViews) -> String {
    match views.current().and_then(|view| view.audience.as_ref()) {
        Some(rpc::view_message::Audience::Display(display)) => display
            .scene
            .as_ref()
            .and_then(|scene| scene.cast.first())
            .map(|actor| actor.live_sprite.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Live defect: a hero's typed line or an NPC's reply moves the table-talk counter without moving
/// the game revision, and the view of that moment is kept as the one on screen. Art that
/// finished afterwards was merged into the stored view beneath it and never showed.
#[test]
fn art_that_arrives_after_table_talk_reaches_the_view_on_screen() {
    let display = scope(1, 2, 3, ViewRole::Display);
    let owner = RpcConnection::new(());
    let generation = owner.generation();
    let mut views = GameplayViews::new(display, generation.clone());
    assert_eq!(
        views.accept(&generation, display, talking_display(4, 0, "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    // The line is spoken: same revision, the counter moves, the view on screen is replaced.
    assert_eq!(
        views.accept(&generation, display, talking_display(4, 3, "")),
        Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
    );
    // The face finishes: the same revision and counter, now with art.
    let with_art = talking_display(4, 3, "/gameplay/imagery/a");
    assert!(matches!(
        views.accept(&generation, display, with_art.clone()),
        Ok(ConnectionViewAcceptance::View(
            ViewAcceptance::Duplicate { .. }
        ))
    ));
    assert!(views.accept_art_update(&generation, display, &with_art, merge_sprite));
    assert_eq!(shown_sprite(&views), "/gameplay/imagery/a");
    // A further reply (the counter moves again) carries the art itself: still shown.
    views
        .accept(
            &generation,
            display,
            talking_display(4, 4, "/gameplay/imagery/a"),
        )
        .unwrap();
    assert_eq!(shown_sprite(&views), "/gameplay/imagery/a");
    // A newer revision replaces everything, including the art it carries.
    views
        .accept(
            &generation,
            display,
            talking_display(5, 4, "/gameplay/imagery/b"),
        )
        .unwrap();
    assert_eq!(shown_sprite(&views), "/gameplay/imagery/b");
    // And art after that, with no table talk at all, still lands.
    let later = talking_display(5, 4, "/gameplay/imagery/c");
    assert!(views.accept_art_update(&generation, display, &later, merge_sprite));
    assert_eq!(shown_sprite(&views), "/gameplay/imagery/c");
}
