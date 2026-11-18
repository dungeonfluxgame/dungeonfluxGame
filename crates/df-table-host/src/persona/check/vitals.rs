//! What a seated hero's check does to their vitals, through the one owner of them
//! (`creation::apply_vitals`): a healing roll heals by its total, and a Second Wind first spends
//! one of its uses (a check run is refused before any dice when none is left). The dice are the
//! check's own; nothing is rolled here.
//!
//! Audit lines: `CHECK vitals hero= change= hp=` for each change taken, `CHECK vitals_refused
//! hero= change= error=` for one the rules refuse.

use df_rules::hero::{SECOND_WIND, VitalsChange};
use df_rules::roll::RollResult;

use super::request::{CheckCall, CheckError, CheckKind};
use crate::state::AppState;

/// Whether `call` spends a use of a limited feature: a Second Wind.
fn spends_second_wind(call: &CheckCall) -> bool {
    call.kind == CheckKind::Healing && call.uses.as_deref() == Some(SECOND_WIND)
}

/// Why `call` cannot be rolled for hero `hero` now: a Second Wind with no use left. Asked before
/// any dice, so a refused use is never rolled.
pub(super) async fn refusal_before(
    app: &AppState,
    hero: u64,
    call: &CheckCall,
) -> Result<(), CheckError> {
    if !spends_second_wind(call) {
        return Ok(());
    }
    let vitals = crate::creation::hero_vitals(app, hero)
        .await
        .map_err(|error| CheckError::Vitals(Box::new(error)))?;
    let left = vitals
        .as_ref()
        .and_then(|vitals| {
            vitals
                .features
                .iter()
                .find(|feature| feature.feature == SECOND_WIND)
        })
        .map_or(0, |feature| feature.left);
    if left == 0 {
        return Err(CheckError::NoUsesLeft {
            feature: SECOND_WIND.to_owned(),
        });
    }
    Ok(())
}

/// The vitals changes the rolled `result` of `call` makes, in order.
fn changes_of(call: &CheckCall, result: &RollResult) -> Vec<VitalsChange> {
    if call.kind != CheckKind::Healing {
        return Vec::new();
    }
    let mut changes = Vec::new();
    if spends_second_wind(call) {
        changes.push(VitalsChange::UseFeature {
            feature: SECOND_WIND.to_owned(),
        });
    }
    if let Ok(amount) = u32::try_from(result.total)
        && amount > 0
    {
        changes.push(VitalsChange::Heal { amount });
    }
    changes
}

/// Takes what `call`'s rolled `result` does to hero `hero`'s vitals; a change the rules refuse is
/// recorded and the ones after it are not taken.
pub(super) async fn apply_result(app: &AppState, hero: u64, call: &CheckCall, result: &RollResult) {
    for change in changes_of(call, result) {
        let key = change.key();
        match crate::creation::apply_vitals(app, hero, change).await {
            Ok(after) => app.record_line(format!(
                "CHECK vitals hero={hero} change={key} hp={}/{}",
                after.current_hp, after.max_hp
            )),
            Err(error) => {
                app.record_line(format!(
                    "CHECK vitals_refused hero={hero} change={key} error={}",
                    error.key()
                ));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use df_rules::hero::{SECOND_WIND, VitalsChange};
    use df_rules::roll::RollMode;

    use super::super::fixtures::sheet;
    use super::super::request::{CheckCall, CheckKind, ModeGround};
    use super::super::rolling::roll_request;
    use super::changes_of;

    fn call(kind: CheckKind, uses: Option<&str>) -> CheckCall {
        CheckCall {
            kind,
            ability: None,
            skill: None,
            dc_band: None,
            mode: RollMode::Normal,
            grounds: ModeGround::None,
            uses: uses.map(str::to_owned),
            attack_mode: None,
            target_ac: None,
        }
    }

    #[test]
    fn a_second_wind_spends_its_use_and_heals_by_its_total_and_other_rolls_change_nothing() {
        let wind = call(CheckKind::Healing, Some(SECOND_WIND));
        let rolled = roll_request(&wind, &sheet(), None)
            .unwrap()
            .resolve(&[5])
            .unwrap();
        assert_eq!(
            changes_of(&wind, &rolled),
            [
                VitalsChange::UseFeature {
                    feature: SECOND_WIND.to_owned()
                },
                VitalsChange::Heal { amount: 8 }
            ],
            "the d10 shows 5, plus the Fighter level's three"
        );
        let swing = call(CheckKind::Damage, Some("longsword"));
        let dealt = roll_request(&swing, &sheet(), None)
            .unwrap()
            .resolve(&[5])
            .unwrap();
        assert!(changes_of(&swing, &dealt).is_empty());
    }
}
