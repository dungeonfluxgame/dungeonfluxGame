//! Seeded dice and draw bindings for the species and reaction fixtures. Each draw is recorded from
//! the caller's `Rng`, so one seed reproduces one fight exactly.
use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_rules::tactical::{
    Action, Battle, DrawBinding, Resolution, RuleError, SpeciesAction, resolve, resolve_species,
};
use df_types::{OperationId, RevisionLabel};

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}

pub fn source() -> RuleReference {
    RuleReference {
        catalog: label("fixture-srd-5.2.1"),
        source: label("pdf-8974902d"),
        entry: label("tactical"),
        clause: label("species-reaction"),
    }
}

fn ids() -> (OperationId, ResolutionId, WindowId) {
    (
        OperationId::from_bytes(&[4; 16]).unwrap(),
        ResolutionId::from_bytes(&[5; 16]).unwrap(),
        WindowId::from_bytes(&[6; 16]).unwrap(),
    )
}

pub fn record(rng: &mut Rng, sides: u32, ordinal: u32, source: &RuleReference) -> ActualDraw {
    let (operation, resolution, window) = ids();
    ActualDraw {
        operation,
        resolution,
        window,
        ordinal,
        sides,
        value: (rng.next() % u64::from(sides)) as u32 + 1,
        source: source.clone(),
    }
}

pub fn binding(source: &RuleReference) -> DrawBinding<'_> {
    let (operation, resolution, window) = ids();
    DrawBinding {
        operation,
        resolution,
        window,
        source,
        next_ordinal: 0,
    }
}

/// Resolve an ordinary action, recording dice on demand as the journey does.
pub fn resolve_dice(
    battle: &Battle,
    actor: u32,
    action: &Action,
    rng: &mut Rng,
) -> Result<Resolution, RuleError> {
    let source = source();
    let mut supplied = Vec::new();
    loop {
        match resolve(battle, actor, action, binding(&source), &supplied) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = supplied.len() as u32;
                supplied.push(record(rng, sides, ordinal, &source));
            }
            other => return other,
        }
    }
}

/// Resolve a species decision the same way, so the reaction's own draws are recorded too.
pub fn resolve_species_dice(
    battle: &Battle,
    actor: u32,
    action: &SpeciesAction,
    rng: &mut Rng,
) -> Result<Resolution, RuleError> {
    let source = source();
    let mut supplied = Vec::new();
    loop {
        match resolve_species(battle, actor, action, binding(&source), &supplied) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = supplied.len() as u32;
                supplied.push(record(rng, sides, ordinal, &source));
            }
            other => return other,
        }
    }
}
