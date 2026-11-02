//! Real tables for the tests that read the real projection: a `TableState` driven through its
//! public effects and the `Attachments` its owners would publish, with no stand-in for either.
//! The two contract tests (`fixture_contract_tests`, `machine_wire_tests`) share them, so the
//! playground and the wire are held to the same projections.

use df_table::effects::TableEffect;
use df_table::element::ElementId;
use df_table::identity::{
    EvidenceRef, FactId, SceneId, SeatId, SessionRevision, TableId, TableScope,
};
use df_table::outcome::{Decline, DeclineCode};
use df_table::ports::Audience;
use df_table::private_data::{Narration, Secret};
use df_table::projection::{
    Attachments, AudienceProjection, NonEmptyText, OfferEntry, OfferId, ProjectionStatus,
    StoryLine, project,
};
use df_table::table_state::{
    CastEntry, CastKind, ConditionName, Controller, DisplayName, Placement, TableSnapshot,
    TableState, Visibility, Vitals,
};
use df_table::testkit::correlation;

pub(crate) const SCENE: u64 = 10;
pub(crate) const PIP_SEAT: u64 = 1;
pub(crate) const VEX_SEAT: u64 = 2;

pub(crate) fn text(words: &str) -> NonEmptyText {
    NonEmptyText::new(words).expect("not blank")
}

fn name(words: &str) -> DisplayName {
    DisplayName::new(words).expect("a valid display name")
}

fn place(x: u32, y: u32) -> Placement {
    Placement::from_milli_units(x * 1000, y * 1000).expect("on the canvas")
}

fn hero(id: &str, display: &str, seat: u64, x: u32) -> CastEntry {
    CastEntry::new(
        ElementId::new(id).expect("a valid element id"),
        CastKind::Party,
        Controller::Seat {
            seat: SeatId::new(seat),
        },
        SceneId::new(SCENE),
        name(display),
        Vitals::new(7, 9).expect("valid vitals"),
    )
    .at(place(x, 20))
}

fn goblin() -> CastEntry {
    CastEntry::new(
        ElementId::new("goblin").expect("a valid element id"),
        CastKind::Npc,
        Controller::Engine,
        SceneId::new(SCENE),
        name("Goblin"),
        Vitals::new(12, 19).expect("valid vitals"),
    )
    .at(place(70, 30))
}

fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
}

fn apply(table: &mut TableState, turn: u64, effects: &[TableEffect]) {
    table
        .apply(&scope(), correlation(turn), effects)
        .expect("the effects apply");
}

/// Seats 1 and 2 in scene 10: Pip (prone) and Vex (hidden from the party) and a seen goblin, after
/// two commits.
pub(crate) fn table_with_two_heroes() -> TableState {
    let mut table = TableState::seated(4, &[SeatId::new(PIP_SEAT), SeatId::new(VEX_SEAT)])
        .expect("a valid table");
    apply(
        &mut table,
        1,
        &[TableEffect::SceneEntered {
            scene: SceneId::new(SCENE),
        }],
    );
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", PIP_SEAT, 30)
                    .with_conditions(vec![ConditionName::new("prone").expect("a name")]),
            },
            TableEffect::CastEntered {
                entry: hero("vex", "Vex", VEX_SEAT, 50).with_visibility(Visibility::Hidden),
            },
            TableEffect::CastEntered { entry: goblin() },
        ],
    );
    table
}

/// What the owners published, for a table at revision `at`: a place, a spoken public line, a
/// private line, a private fact, two offers (one disabled with its reason) and a decline with
/// alternatives for Pip.
pub(crate) fn published(at: SessionRevision) -> Attachments {
    let mut attachments = Attachments::new();
    attachments
        .set_place(SceneId::new(SCENE), text("The cellar stairs"))
        .expect("room for a place");
    attachments
        .push_public(
            StoryLine::new(text("Pip chose attack: done."))
                .spoken_by(name("Pip"))
                .with_narration(Narration::new("Pip swings at the goblin and it yelps."))
                .with_evidence(EvidenceRef::from_bytes(&[5; 16]).expect("nonzero")),
        )
        .expect("room for a line");
    attachments
        .push_private(
            SeatId::new(PIP_SEAT),
            StoryLine::new(text("You notice a loose stone.")),
        )
        .expect("room for a private line");
    attachments
        .set_private_fact(
            SeatId::new(PIP_SEAT),
            FactId::from_bytes(&[6; 16]).expect("nonzero"),
            Secret::new("The loose stone hides a key.".to_owned()),
        )
        .expect("room for a fact");
    attachments
        .set_notice(
            SeatId::new(PIP_SEAT),
            Decline::new(DeclineCode::NotYourTurn, "It isn't your turn.")
                .expect("a reason")
                .with_alternative("Wait for Vex")
                .expect("an alternative")
                .with_alternative("Search the room")
                .expect("an alternative"),
            at,
        )
        .expect("room for a notice");
    attachments
        .set_seat_offers(
            SeatId::new(PIP_SEAT),
            vec![
                OfferEntry::new(
                    OfferId::new("look-around").expect("an id"),
                    text("Look around"),
                )
                .explained(text("Take a slow look at the cellar.")),
                OfferEntry::new(
                    OfferId::new("open-the-chest").expect("an id"),
                    text("Open the chest"),
                )
                .in_group(text("Investigate"))
                .unavailable(text("The chest is locked.")),
                OfferEntry::new(
                    OfferId::new("ask-the-guard").expect("an id"),
                    text("Ask the guard"),
                )
                .in_group(text("Investigate")),
            ],
        )
        .expect("room for offers");
    attachments
        .set_activity(ProjectionStatus::Telling)
        .expect("room for an activity");
    attachments
}

/// The snapshot and attachments of the table above.
pub(crate) fn busy_table() -> (TableSnapshot, Attachments) {
    let snapshot = table_with_two_heroes().snapshot();
    let attachments = published(snapshot.revision());
    (snapshot, attachments)
}

/// The same table, paused: every audience waits on the host.
pub(crate) fn paused_table() -> (TableSnapshot, Attachments) {
    let mut table = table_with_two_heroes();
    apply(&mut table, 3, &[TableEffect::TablePaused]);
    let snapshot = table.snapshot();
    let attachments = published(snapshot.revision());
    (snapshot, attachments)
}

/// A table that has had no commit and has said nothing: the state a view must still fill.
pub(crate) fn empty_table() -> (TableSnapshot, Attachments) {
    let table = TableState::seated(4, &[SeatId::new(PIP_SEAT)]).expect("a valid table");
    (table.snapshot(), Attachments::new())
}

/// The projection for an audience, which the tests expect to exist.
pub(crate) fn projection_of(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
    audience: Audience,
) -> AudienceProjection {
    project(snapshot, attachments, audience).expect("the audience is at the table")
}

/// The shared screen's audience.
pub(crate) fn shared() -> Audience {
    Audience::Public
}

/// One seat's audience.
pub(crate) fn seat(number: u64) -> Audience {
    Audience::Seat(SeatId::new(number))
}
