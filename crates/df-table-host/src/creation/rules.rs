//! The creation machine's rules port over `df-rules`: a draft derived into the hero's sheet, and
//! "Surprise me". Also the one conversion of a derived sheet to the `HeroProfile` the production
//! rules adapter rolls a seat's checks from.
//!
//! It owns no rule: every number and every problem is `df_rules::hero`'s. It maps the rules'
//! problems and marks to the machine's plain values and writes the sheet as the JSON the check
//! judge reads. It fails only where `df-rules` refuses (a problem list), never by guessing.
use std::collections::BTreeMap;

use df_rules::hero::{HeroSheet, derive_hero, fill_build, surprise_build};
use df_table::creation::{
    BuildDraft, BuildIssue, BuildRules, Derivation, DesignMark, HeroStart, SheetDocument,
};

use crate::adapters::{AdapterError, HeroProfile};

/// The id a hero's sheet carries: the character id the check judge is asked about.
pub(crate) fn sheet_id(hero: u64) -> String {
    format!("hero-{hero}")
}

/// The rules of creation, read from `df_rules::hero`.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeroRules;

impl HeroRules {
    /// The typed sheet of `hero` built from `draft`, or the draft's problems as the machine
    /// words them.
    pub fn sheet(hero: &HeroStart, draft: &BuildDraft) -> Result<HeroSheet, Vec<BuildIssue>> {
        derive_hero(&sheet_id(hero.hero), &hero.name, draft.picks()).map_err(|problems| {
            problems
                .into_iter()
                .map(|problem| BuildIssue {
                    slot: problem.group,
                    sentence: problem.sentence,
                })
                .collect()
        })
    }
}

impl BuildRules for HeroRules {
    fn derive(&self, hero: &HeroStart, draft: &BuildDraft) -> Derivation {
        let sheet = match Self::sheet(hero, draft) {
            Ok(sheet) => sheet,
            Err(issues) => return Derivation::Incomplete { issues },
        };
        let marks = sheet
            .not_in_play
            .iter()
            .map(|mark| DesignMark {
                subject: mark.subject.clone(),
                reason: mark.reason.key().to_owned(),
                note: mark.note.clone(),
            })
            .collect();
        match serde_json::to_value(&sheet) {
            Ok(written) => Derivation::Complete {
                sheet: SheetDocument::new(written),
                marks,
            },
            Err(error) => Derivation::Incomplete {
                issues: vec![BuildIssue {
                    slot: None,
                    sentence: format!("The sheet could not be written: {error}."),
                }],
            },
        }
    }

    fn surprise(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String> {
        surprise_build(draft.picks(), pick)
    }

    fn fill(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String> {
        fill_build(draft.picks(), draft.cleared(), pick)
    }
}

/// The check profile of a derived sheet: its scores, Proficiency Bonus and proficient skills, for
/// the production rules adapter (`DfRulesOwner` over a `SeatRoster`).
pub fn hero_profile(sheet: &HeroSheet) -> Result<HeroProfile, AdapterError> {
    HeroProfile::new(
        sheet.abilities,
        sheet.proficiency_bonus,
        sheet.proficient_skills.iter().copied(),
    )
}
