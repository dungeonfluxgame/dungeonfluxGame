//! The sentinel-secret and paired-state tests of H.11 for projection: a seat never sees another
//! seat's private fact in any projection, and two states that differ only in one seat's private
//! data project identically for every other audience.

use crate::effects::TableEffect;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::Audience;
use crate::private_data::{Narration, Secret};
use crate::projection::{
    Attachments, ProjectionStatus, StoryLine, project, project_seat, project_shared,
};
use crate::projection_tests::{evidence, fact_id, json_of, public_line, text};
use crate::table_state::{CastChange, TableState, Visibility};
use crate::table_state_tests::{apply, creature, element, hero, scene, scope, seat};
use crate::testkit::{SENTINEL_PUBLIC_FACT, SENTINEL_SECRET, correlation};

const SEATS: [u64; 3] = [1, 2, 3];
const HIDDEN_NAME: &str = "SENTINEL-HIDDEN-Ogre";

fn secret_of(kind: &str, seat_number: u64) -> String {
    format!("{SENTINEL_SECRET}-{kind}-seat{seat_number}")
}

/// Everything one seat alone may read.
#[derive(Clone, Default)]
struct SeatSecrets {
    fact: Option<String>,
    line: Option<String>,
    line_evidence: Option<u8>,
    notice: Option<String>,
    readout: Option<String>,
}

impl SeatSecrets {
    fn full(seat_number: u64) -> Self {
        Self {
            fact: Some(secret_of("fact", seat_number)),
            line: Some(secret_of("line", seat_number)),
            line_evidence: Some(u8::try_from(seat_number).expect("a small number") + 100),
            notice: Some(secret_of("notice", seat_number)),
            readout: Some(secret_of("readout", seat_number)),
        }
    }
}

fn table_of_three(ogre_turn_effects: &[TableEffect]) -> TableState {
    let mut table = TableState::seated(6, &[seat(1), seat(2), seat(3)]).expect("valid table");
    let mut effects = vec![TableEffect::SceneEntered { scene: scene(10) }];
    for (number, x) in [(1, 20), (2, 40), (3, 60)] {
        effects.push(TableEffect::CastEntered {
            entry: hero(
                &format!("hero{number}"),
                &format!("Hero{number}"),
                number,
                10,
                x,
            ),
        });
    }
    effects.push(TableEffect::CastEntered {
        entry: creature("goblin", "Goblin", 10, 80),
    });
    apply(&mut table, 1, &effects).expect("the table is set");
    apply(&mut table, 2, ogre_turn_effects).expect("the turn applies");
    table
}

/// Attachments with a public line, and the given secrets filed under each seat.
fn attachments_with(secrets: &[SeatSecrets; 3], with_story: bool) -> Attachments {
    let mut attachments = Attachments::new();
    attachments
        .set_place(scene(10), text("The cellar stairs"))
        .unwrap();
    attachments.set_activity(ProjectionStatus::Telling).unwrap();
    attachments
        .set_shared_readout(text("The party waits."))
        .unwrap();
    if with_story {
        attachments
            .push_public(public_line(&format!(
                "{SENTINEL_PUBLIC_FACT} is known to all."
            )))
            .unwrap();
    }
    for (number, secrets) in SEATS.into_iter().zip(secrets) {
        let who = seat(number);
        if let Some(fact) = &secrets.fact {
            attachments
                .set_private_fact(who, fact_id(1), Secret::new(fact.clone()))
                .unwrap();
        }
        if let Some(line) = &secrets.line {
            let mut story =
                StoryLine::new(text("A private line.")).with_narration(Narration::new(line));
            if let Some(byte) = secrets.line_evidence {
                story = story.with_evidence(evidence(byte));
            }
            attachments.push_private(who, story).unwrap();
        }
        if let Some(notice) = &secrets.notice {
            let decline = Decline::new(DeclineCode::NotViable, notice).unwrap();
            attachments
                .set_notice(who, decline, correlation(2).revision)
                .unwrap();
        }
        if let Some(readout) = &secrets.readout {
            attachments.set_seat_readout(who, text(readout)).unwrap();
        }
    }
    attachments
}

fn full_secrets() -> [SeatSecrets; 3] {
    [1, 2, 3].map(SeatSecrets::full)
}

fn audiences() -> Vec<Audience> {
    let mut all = vec![Audience::Public];
    all.extend(SEATS.into_iter().map(|number| Audience::Seat(seat(number))));
    all
}

/// Everything an audience could be shown of a projection: its JSON, its Debug text and the line of
/// the event that records it.
fn everything_shown(table: &TableState, attachments: &Attachments, audience: Audience) -> String {
    let projection = project(&table.snapshot(), attachments, audience).expect("projects");
    let record_line = projection
        .record()
        .event(&scope(), correlation(9))
        .render_audit_line();
    format!(
        "{}\n{projection:?}\n{record_line}",
        serde_json::to_string(&projection).expect("serializes")
    )
}

/// The view and the line that logs it, without the build's internal work count: building a view
/// for a table with one more creature examines one more entry, which is no part of what is shown.
fn what_the_audience_is_shown(
    table: &TableState,
    attachments: &Attachments,
    audience: Audience,
) -> String {
    let projection = project(&table.snapshot(), attachments, audience).expect("projects");
    let record_line = projection
        .record()
        .event(&scope(), correlation(9))
        .render_audit_line();
    format!(
        "{}
{record_line}",
        serde_json::to_string(&projection).expect("serializes")
    )
}

fn hidden_ogre_effects() -> Vec<TableEffect> {
    vec![
        TableEffect::CastEntered {
            entry: crate::table_state_tests::creature("ogre", HIDDEN_NAME, 10, 50),
        },
        TableEffect::CastChanged {
            cast: element("ogre"),
            change: CastChange::Visibility {
                visibility: Visibility::Hidden,
            },
        },
    ]
}

// Sentinel secrets.

#[test]
fn a_seats_private_data_reaches_only_that_seat_in_any_projection() {
    let table = table_of_three(&hidden_ogre_effects());
    let attachments = attachments_with(&full_secrets(), true);
    for audience in audiences() {
        let shown = everything_shown(&table, &attachments, audience);
        for number in SEATS {
            let own = audience == Audience::Seat(seat(number));
            for kind in ["fact", "line", "notice", "readout"] {
                let secret = secret_of(kind, number);
                let reaches_seat = matches!(kind, "fact" | "line" | "notice" | "readout");
                assert_eq!(
                    shown.contains(&secret),
                    own && reaches_seat,
                    "{kind} of seat {number} shown to {audience:?}"
                );
            }
        }
    }
}

#[test]
fn a_seats_machine_readout_shows_only_to_that_seat_with_or_without_a_story() {
    let table = table_of_three(&hidden_ogre_effects());
    let secrets = [1, 2, 3].map(|number| SeatSecrets {
        readout: Some(secret_of("readout", number)),
        ..SeatSecrets::default()
    });
    let attachments = attachments_with(&secrets, false);
    for audience in audiences() {
        let shown = everything_shown(&table, &attachments, audience);
        for number in SEATS {
            let own = audience == Audience::Seat(seat(number));
            assert_eq!(
                shown.contains(&secret_of("readout", number)),
                own,
                "readout of seat {number} shown to {audience:?}"
            );
        }
    }
}

#[test]
fn what_everyone_may_know_reaches_every_audience_that_shows_the_story() {
    let table = table_of_three(&hidden_ogre_effects());
    let attachments = attachments_with(&full_secrets(), true);
    for audience in audiences() {
        let shown = everything_shown(&table, &attachments, audience);
        assert!(
            shown.contains(SENTINEL_PUBLIC_FACT),
            "{audience:?} lost the public line"
        );
    }
}

#[test]
fn a_creature_hidden_from_the_party_is_named_in_no_projection_and_every_hero_is() {
    let table = table_of_three(&hidden_ogre_effects());
    let attachments = attachments_with(&full_secrets(), true);
    for audience in audiences() {
        let shown = everything_shown(&table, &attachments, audience);
        assert!(
            !shown.contains("Ogre"),
            "{audience:?} was shown the hidden creature"
        );
        assert!(
            !shown.contains("ogre"),
            "{audience:?} was shown the hidden creature"
        );
    }
    let tv = serde_json::to_string(&project_shared(&table.snapshot(), &attachments)).unwrap();
    for number in SEATS {
        assert!(
            tv.contains(&format!("Hero{number}")),
            "hero {number} is always on the TV"
        );
    }
}

#[test]
fn the_debug_text_of_the_published_data_holds_no_secret_and_no_name() {
    let table = table_of_three(&hidden_ogre_effects());
    let attachments = attachments_with(&full_secrets(), true);
    for debug in [
        format!("{attachments:?}"),
        format!("{table:?}"),
        format!("{:?}", table.snapshot()),
    ] {
        assert!(!debug.contains(SENTINEL_SECRET), "{debug}");
        assert!(!debug.contains(SENTINEL_PUBLIC_FACT), "{debug}");
        assert!(!debug.contains("Hero1"), "{debug}");
        assert!(!debug.contains(HIDDEN_NAME), "{debug}");
    }
}

// Paired states.

type Pair = (TableState, Attachments);

/// The projections of a pair that every audience other than `owner` is shown, as text.
fn shown_to_everyone_but(owner: u64, pair: &Pair) -> Vec<(Audience, String)> {
    audiences()
        .into_iter()
        .filter(|audience| *audience != Audience::Seat(seat(owner)))
        .map(|audience| (audience, everything_shown(&pair.0, &pair.1, audience)))
        .collect()
}

fn records_for_everyone_but(owner: u64, pair: &Pair) -> Vec<crate::projection::ProjectionRecord> {
    audiences()
        .into_iter()
        .filter(|audience| *audience != Audience::Seat(seat(owner)))
        .map(|audience| {
            *project(&pair.0.snapshot(), &pair.1, audience)
                .unwrap()
                .record()
        })
        .collect()
}

fn assert_indistinguishable_to_everyone_but(owner: u64, one: &Pair, other: &Pair, what: &str) {
    assert_eq!(
        shown_to_everyone_but(owner, one),
        shown_to_everyone_but(owner, other),
        "{what}: another audience can tell the states apart"
    );
    assert_eq!(
        records_for_everyone_but(owner, one),
        records_for_everyone_but(owner, other),
        "{what}: another audience's record differs"
    );
}

fn assert_owner_can_tell_apart(owner: u64, one: &Pair, other: &Pair, what: &str) {
    let audience = Audience::Seat(seat(owner));
    assert_ne!(
        everything_shown(&one.0, &one.1, audience),
        everything_shown(&other.0, &other.1, audience),
        "{what}: the owner should see the difference"
    );
}

type Edit = fn(&mut SeatSecrets);

fn edits() -> Vec<(&'static str, Edit)> {
    vec![
        ("a different private fact", |secrets| {
            secrets.fact = Some("a different secret".to_owned());
        }),
        ("no private fact", |secrets| secrets.fact = None),
        ("a different private line", |secrets| {
            secrets.line = Some("a different whisper".to_owned());
        }),
        ("no private line", |secrets| {
            secrets.line = None;
            secrets.line_evidence = None;
        }),
        ("a different specimen of the private line", |secrets| {
            secrets.line_evidence = Some(222);
        }),
        ("a different decline", |secrets| {
            secrets.notice = Some("a different refusal".to_owned());
        }),
        ("no decline", |secrets| secrets.notice = None),
    ]
}

#[test]
fn two_states_that_differ_only_in_one_seats_private_data_look_the_same_to_everyone_else() {
    for owner in SEATS {
        for (what, edit) in edits() {
            let base = full_secrets();
            let mut changed = base.clone();
            let index = usize::try_from(owner - 1).unwrap();
            edit(&mut changed[index]);
            let table = table_of_three(&hidden_ogre_effects());
            let one = (table.clone(), attachments_with(&base, true));
            let other = (table, attachments_with(&changed, true));
            assert_indistinguishable_to_everyone_but(owner, &one, &other, what);
            assert_owner_can_tell_apart(owner, &one, &other, what);
        }
    }
}

#[test]
fn a_seats_readout_is_private_too_when_the_seat_has_no_story() {
    let blank = || [1, 2, 3].map(|_| SeatSecrets::default());
    let mut with_readout = blank();
    with_readout[1].readout = Some(secret_of("readout", 2));
    let mut other_readout = blank();
    other_readout[1].readout = Some("another readout".to_owned());
    let table = table_of_three(&hidden_ogre_effects());
    let one = (table.clone(), attachments_with(&with_readout, false));
    let other = (table, attachments_with(&other_readout, false));
    assert_indistinguishable_to_everyone_but(2, &one, &other, "a seat readout");
    assert_owner_can_tell_apart(2, &one, &other, "a seat readout");
}

#[test]
fn two_states_that_differ_only_in_a_hidden_creature_look_the_same_to_everyone() {
    let attachments = attachments_with(&full_secrets(), true);
    let nothing_hidden = table_of_three(&[TableEffect::TimeAdvanced { advance_s: 0 }]);
    let ogre = table_of_three(&hidden_ogre_effects());
    let moved_ogre = {
        let mut effects = hidden_ogre_effects();
        effects.push(TableEffect::CastChanged {
            cast: element("ogre"),
            change: CastChange::HitPoints { hit_points: 1 },
        });
        effects.push(TableEffect::CastChanged {
            cast: element("ogre"),
            change: CastChange::Visibility {
                visibility: Visibility::Invisible,
            },
        });
        table_of_three(&effects)
    };
    let states = [nothing_hidden, ogre, moved_ogre];
    for audience in audiences() {
        let shown: Vec<_> = states
            .iter()
            .map(|table| what_the_audience_is_shown(table, &attachments, audience))
            .collect();
        assert_eq!(shown[0], shown[1], "{audience:?}: a hidden ogre shows");
        assert_eq!(
            shown[1], shown[2],
            "{audience:?}: a hidden ogre's state shows"
        );
    }
}

#[test]
fn the_same_difference_in_a_creature_the_party_sees_is_shown_which_proves_the_test_has_teeth() {
    let attachments = attachments_with(&full_secrets(), true);
    let nothing = table_of_three(&[TableEffect::TimeAdvanced { advance_s: 0 }]);
    let seen_ogre = table_of_three(&[TableEffect::CastEntered {
        entry: creature("ogre", HIDDEN_NAME, 10, 50),
    }]);
    let tv = |table: &TableState| {
        serde_json::to_string(&project_shared(&table.snapshot(), &attachments)).unwrap()
    };
    assert_ne!(tv(&nothing), tv(&seen_ogre));
    assert!(tv(&seen_ogre).contains(HIDDEN_NAME));
}

#[test]
fn a_seats_view_never_changes_when_only_another_seats_view_data_does() {
    let table = table_of_three(&hidden_ogre_effects());
    let base = attachments_with(&full_secrets(), true);
    let mut changed_secrets = full_secrets();
    changed_secrets[2] = SeatSecrets::default();
    let changed = attachments_with(&changed_secrets, true);
    for number in [1, 2] {
        let before = json_of(&project_seat(&table.snapshot(), &base, seat(number)).unwrap());
        let after = json_of(&project_seat(&table.snapshot(), &changed, seat(number)).unwrap());
        assert_eq!(before, after, "seat {number}");
    }
}
