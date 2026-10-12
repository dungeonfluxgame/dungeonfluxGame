use std::cell::Cell;

use df_combat::movement::{
    MovementContext, MovementError, MovementFacts, MovementLimits, MovementOwner,
    MovementSelection, SuppliedPath, select_movement,
};
use df_model::checkpoint::{
    Basis, CheckpointPins, ContentDigest, ContentPins, ContentReference, EncounterState, EntityId,
    Position, RecordId, RuleReference, RulesMode, RulesPins, WorldEntity,
};
use df_types::{BuildIdentity, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision};

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn entity(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}

fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

fn content(value: &str) -> ContentReference {
    ContentReference {
        package: label("fixture-package"),
        entry: label(value),
    }
}

fn source() -> RuleReference {
    RuleReference {
        catalog: label("fixture-catalog"),
        source: label("fixture-source"),
        entry: label("fixture-movement"),
        clause: label("supplied-path-admission"),
    }
}

fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("fixture-ruleset"),
            catalog: label("fixture-catalog"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: label("fixture-sources"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: label("fixture-handler"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: label("fixture-content"),
            content_digest: ContentDigest([4; 32]),
            package: label("fixture-package"),
            package_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("fixture-source"),
            Some("fixture-native"),
            Some("fixture-wasm"),
            Some("fixture-config"),
            Some("fixture-content"),
        )
        .unwrap(),
    }
}

fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 7),
    }
}

fn limits() -> MovementLimits {
    MovementLimits {
        candidates: 4,
        objectives: 4,
        points_per_path: 8,
        total_points: 32,
        identity_comparisons: 6,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FixturePath {
    offer: RecordId,
    actor: EntityId,
    origin_location: EntityId,
    destination: EntityId,
    points: Vec<Position>,
    objective: ContentReference,
    source: RuleReference,
    geometry: u64,
    turn: u64,
    permitted: bool,
    legal: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum OwnerRefusal {
    StaleContext,
    StaleGeometry,
    StaleTurn,
    Unoffered,
    Undisclosed,
    Illegal,
}

// This fake supplies already-classified answers, never D&D geometry/source behavior.
struct FixtureOwner {
    basis: Basis,
    pins: CheckpointPins,
    geometry: u64,
    turn: u64,
    offered: Vec<RecordId>,
    admission_calls: Cell<usize>,
}

impl FixtureOwner {
    fn new() -> Self {
        Self {
            basis: basis(),
            pins: pins(),
            geometry: 3,
            turn: 9,
            offered: vec![record(1), record(2), record(3)],
            admission_calls: Cell::new(0),
        }
    }
}

impl MovementOwner for FixtureOwner {
    type Path = FixturePath;
    type Error = OwnerRefusal;

    fn point_count(&self, path: &FixturePath) -> usize {
        path.points.len()
    }

    fn validate_context(&self, context: &MovementContext<'_>) -> Result<(), OwnerRefusal> {
        self.admission_calls.set(self.admission_calls.get() + 1);
        if context.basis.session != self.basis.session
            || context.basis.run != self.basis.run
            || context.basis.revision.epoch() != self.basis.revision.epoch()
            || context.pins != &self.pins
        {
            return Err(OwnerRefusal::StaleContext);
        }
        Ok(())
    }

    fn admit_path<'a>(
        &self,
        _context: &MovementContext<'_>,
        candidate: &SuppliedPath<'a, FixturePath>,
    ) -> Result<MovementFacts<'a>, OwnerRefusal> {
        self.admission_calls.set(self.admission_calls.get() + 1);
        let path = candidate.path;
        if candidate.offer != path.offer || !self.offered.contains(&candidate.offer) {
            return Err(OwnerRefusal::Unoffered);
        }
        if path.geometry != self.geometry {
            return Err(OwnerRefusal::StaleGeometry);
        }
        if path.turn != self.turn {
            return Err(OwnerRefusal::StaleTurn);
        }
        if !path.permitted {
            return Err(OwnerRefusal::Undisclosed);
        }
        if !path.legal {
            return Err(OwnerRefusal::Illegal);
        }
        let origin_position = *path.points.first().ok_or(OwnerRefusal::Illegal)?;
        let destination_position = *path.points.last().ok_or(OwnerRefusal::Illegal)?;
        Ok(MovementFacts {
            actor: path.actor,
            origin_location: path.origin_location,
            origin_position,
            destination: path.destination,
            destination_position,
            objective: &path.objective,
            source: &path.source,
        })
    }
}

struct Fixture {
    basis: Basis,
    pins: CheckpointPins,
    actor: WorldEntity,
    encounter: EncounterState,
    preferences: Vec<ContentReference>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            basis: basis(),
            pins: pins(),
            actor: WorldEntity {
                id: entity(1),
                definition: content("actor"),
                location: Some(entity(2)),
                position: Some(Position { x: 0, y: 0, z: 0 }),
                identity_revision: label("fixture-identity"),
            },
            encounter: EncounterState {
                id: record(9),
                definition: content("encounter"),
                participants: vec![entity(1)],
                turn_order: vec![entity(1)],
                active_turn: Some(entity(1)),
                objectives: vec![content("escape"), content("hold")],
                combat_policy: content("tactics"),
                tactical: None,
            },
            preferences: vec![content("escape"), content("hold")],
        }
    }

    fn context(&self) -> MovementContext<'_> {
        MovementContext {
            basis: self.basis,
            pins: &self.pins,
            actor: &self.actor,
            encounter: &self.encounter,
            policy: &self.encounter.combat_policy,
            preferred_objectives: &self.preferences,
        }
    }
}

fn path(offer: u8, objective: &str) -> FixturePath {
    FixturePath {
        offer: record(offer),
        actor: entity(1),
        origin_location: entity(2),
        destination: entity(3),
        points: vec![Position { x: 0, y: 0, z: 0 }, Position { x: 4, y: 2, z: 7 }],
        objective: content(objective),
        source: source(),
        geometry: 3,
        turn: 9,
        permitted: true,
        legal: true,
    }
}

fn supplied(path: &FixturePath) -> SuppliedPath<'_, FixturePath> {
    SuppliedPath {
        offer: path.offer,
        path,
    }
}

#[test]
fn objective_preference_selects_the_exact_admitted_path_without_mutation() {
    let fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let hold = path(1, "hold");
    let escape = path(2, "escape");
    let before_actor = fixture.actor.clone();
    let before_encounter = fixture.encounter.clone();
    let before_path = escape.clone();
    let candidates = [supplied(&hold), supplied(&escape)];
    let context = fixture.context();

    let selected = select_movement(&owner, &context, &candidates, limits()).unwrap();
    let MovementSelection::Selected(selected) = selected else {
        panic!("expected a path")
    };

    assert!(std::ptr::eq(selected.path, &escape));
    assert_eq!(selected.offer, record(2));
    assert_eq!(selected.objective_preference, 0);
    assert_eq!(selected.facts.destination, entity(3));
    assert_eq!(
        selected.facts.destination_position,
        Position { x: 4, y: 2, z: 7 }
    );
    assert_eq!(selected.basis, fixture.basis);
    assert_eq!(selected.encounter, fixture.encounter.id);
    assert!(std::ptr::eq(selected.pins, &fixture.pins));
    assert_eq!(fixture.actor, before_actor);
    assert_eq!(fixture.encounter, before_encounter);
    assert_eq!(escape, before_path);
    assert_eq!(owner.admission_calls.get(), 3);
}

#[test]
fn ties_use_canonical_offer_identity_and_replay_is_order_independent() {
    let fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let first = path(1, "escape");
    let second = path(2, "escape");
    let context = fixture.context();
    let forward = [supplied(&first), supplied(&second)];
    let reverse = [supplied(&second), supplied(&first)];

    let expected = select_movement(&owner, &context, &forward, limits()).unwrap();

    assert_eq!(
        select_movement(&owner, &context, &reverse, limits()).unwrap(),
        expected
    );
    assert_eq!(
        select_movement(&owner, &context, &forward, limits()).unwrap(),
        expected
    );
    let MovementSelection::Selected(selected) = expected else {
        panic!("expected a path")
    };
    assert_eq!(selected.offer, record(1));
}

#[test]
fn changed_geometry_and_turn_refuse_even_a_preferred_path() {
    let fixture = Fixture::new();
    let candidate = path(1, "escape");
    let mut owner = FixtureOwner::new();
    owner.geometry += 1;
    let context = fixture.context();
    let candidates = [supplied(&candidate)];

    assert_eq!(
        select_movement(&owner, &context, &candidates, limits()).err(),
        Some(MovementError::Owner(OwnerRefusal::StaleGeometry))
    );
    owner.geometry = 3;
    owner.turn += 1;
    assert_eq!(
        select_movement(&owner, &context, &candidates, limits()).err(),
        Some(MovementError::Owner(OwnerRefusal::StaleTurn))
    );
}

#[test]
fn unrelated_session_revision_is_delegated_to_relevant_context_authority() {
    let mut fixture = Fixture::new();
    fixture.basis.revision = fixture.basis.revision.next_sequence().unwrap();
    let owner = FixtureOwner::new();
    let candidate = path(1, "escape");

    assert!(matches!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        ),
        Ok(MovementSelection::Selected(_))
    ));
    fixture.basis.run = RunId::from_bytes(&[3; 16]).unwrap();
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::Owner(OwnerRefusal::StaleContext))
    );
}

#[test]
fn changed_pins_and_unoffered_paths_cannot_establish_legality() {
    let mut fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let mut candidate = path(1, "escape");
    fixture.pins.rules.handler_digest = ContentDigest([99; 32]);
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::Owner(OwnerRefusal::StaleContext))
    );
    fixture.pins = pins();
    candidate.offer = record(99);
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::Owner(OwnerRefusal::Unoffered))
    );
}

#[test]
fn lower_ranked_illegal_or_undisclosed_paths_prevent_partial_success() {
    let fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let preferred = path(1, "escape");
    let mut other = path(2, "hold");
    other.legal = false;
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&preferred), supplied(&other)],
            limits()
        )
        .err(),
        Some(MovementError::Owner(OwnerRefusal::Illegal))
    );
    other.legal = true;
    other.permitted = false;
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&preferred), supplied(&other)],
            limits()
        )
        .err(),
        Some(MovementError::Owner(OwnerRefusal::Undisclosed))
    );
}

#[test]
fn unknown_objectives_foreign_sources_and_forged_origins_are_refused() {
    let fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let mut candidate = path(1, "unadmitted-objective");
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::UnknownObjective { index: 0 })
    );
    candidate.objective = content("escape");
    candidate.source.catalog = label("foreign-catalog");
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::SourceMismatch { index: 0 })
    );
    candidate.source = source();
    candidate.origin_location = entity(99);
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::OriginMismatch { index: 0 })
    );
}

#[test]
fn missing_actor_geometry_inactive_actor_and_unknown_preferences_are_refused() {
    let mut fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let candidate = path(1, "escape");
    fixture.actor.position = None;
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::MissingOrigin)
    );
    fixture.actor.position = Some(Position { x: 0, y: 0, z: 0 });
    fixture.encounter.active_turn = Some(entity(99));
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::ActorNotActive)
    );
    fixture.encounter.active_turn = Some(entity(1));
    fixture.preferences = vec![content("foreign-objective")];
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .err(),
        Some(MovementError::UnknownPreference { index: 0 })
    );
}

#[test]
fn bounded_work_is_refused_before_owner_admission() {
    let fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let first = path(1, "escape");
    let second = path(2, "hold");
    let candidates = [supplied(&first), supplied(&second)];
    let context = fixture.context();
    let mut capacity = limits();
    capacity.candidates = 1;
    assert_eq!(
        select_movement(&owner, &context, &candidates, capacity).err(),
        Some(MovementError::CandidateCapacity)
    );
    capacity = limits();
    capacity.identity_comparisons = 0;
    assert_eq!(
        select_movement(&owner, &context, &candidates, capacity).err(),
        Some(MovementError::ComparisonCapacity)
    );
    capacity = limits();
    capacity.points_per_path = 1;
    assert_eq!(
        select_movement(&owner, &context, &candidates, capacity).err(),
        Some(MovementError::PathCapacity { index: 0 })
    );
    capacity = limits();
    capacity.points_per_path = 2;
    capacity.total_points = 3;
    assert_eq!(
        select_movement(&owner, &context, &candidates, capacity).err(),
        Some(MovementError::TotalPointCapacity)
    );
    capacity = limits();
    capacity.objectives = 1;
    assert_eq!(
        select_movement(&owner, &context, &candidates, capacity).err(),
        Some(MovementError::ObjectiveCapacity)
    );
    assert_eq!(owner.admission_calls.get(), 0);
}

#[test]
fn duplicate_offers_and_preferences_have_exact_refusals() {
    let mut fixture = Fixture::new();
    let owner = FixtureOwner::new();
    let first = path(1, "escape");
    let second = path(1, "hold");
    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&first), supplied(&second)],
            limits()
        )
        .err(),
        Some(MovementError::DuplicateOffer {
            first: 0,
            duplicate: 1
        })
    );
    assert_eq!(owner.admission_calls.get(), 0);
    fixture.preferences = vec![content("escape"), content("escape")];
    assert_eq!(
        select_movement(&owner, &fixture.context(), &[supplied(&first)], limits()).err(),
        Some(MovementError::DuplicatePreference {
            first: 0,
            duplicate: 1
        })
    );
}

#[test]
fn no_matching_preference_is_a_successful_decline() {
    let mut fixture = Fixture::new();
    fixture.preferences = vec![content("escape")];
    let owner = FixtureOwner::new();
    let candidate = path(1, "hold");

    assert_eq!(
        select_movement(
            &owner,
            &fixture.context(),
            &[supplied(&candidate)],
            limits()
        )
        .unwrap(),
        MovementSelection::NoPreferredPath
    );
    assert_eq!(
        select_movement(&owner, &fixture.context(), &[], limits()).unwrap(),
        MovementSelection::NoPreferredPath
    );
}
