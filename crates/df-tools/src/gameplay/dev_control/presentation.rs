//! Display-only development controls, delivered by the existing layout watch wakeup.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PresentationState {
    schema: u32,
    revision: u64,
    paused: bool,
    steps: u64,
    replays: u64,
    speed: f64,
    timing: BTreeMap<String, u32>,
}
impl Default for PresentationState {
    fn default() -> Self {
        Self {
            schema: 1,
            revision: 0,
            paused: false,
            steps: 0,
            replays: 0,
            speed: 1.0,
            timing: BTreeMap::new(),
        }
    }
}
impl PresentationState {
    pub(super) fn apply(&mut self, body: &Value) -> Result<(), String> {
        let mut next = self.clone();
        match body
            .get("op")
            .and_then(Value::as_str)
            .ok_or("presentation needs op")?
        {
            "pause" => next.paused = true,
            "resume" => next.paused = false,
            "step" => {
                next.paused = true;
                next.steps = next.steps.checked_add(1).ok_or("step counter exhausted")?;
            }
            "replay" => {
                next.replays = next
                    .replays
                    .checked_add(1)
                    .ok_or("replay counter exhausted")?
            }
            "speed" => {
                let speed = body
                    .get("value")
                    .or_else(|| body.get("speed"))
                    .and_then(Value::as_f64)
                    .ok_or("speed needs value")?;
                if !speed.is_finite() || !(0.25..=4.0).contains(&speed) {
                    return Err("speed must be 0.25..=4".to_owned());
                }
                next.speed = speed;
            }
            "timing" => {
                let given = body
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("timing needs name")?
                    .to_ascii_lowercase()
                    .replace('_', "-");
                let name=match given.as_str(){"dice-tumble"|"attack-d20-tumble"=>"dice-tumble","dice-read"|"attack-face-dwell"=>"dice-read","damage-dice-tumble"=>"damage-dice-tumble","roll-card-miss-hold"|"miss-card-hold"=>"roll-card-miss-hold","roll-card-hit-hold"|"hit-card-hold"=>"roll-card-hit-hold",_=>return Err("timing names: DICE_TUMBLE DICE_READ DAMAGE_DICE_TUMBLE ROLL_CARD_MISS_HOLD ROLL_CARD_HIT_HOLD".to_owned())};
                let ms = body
                    .get("ms")
                    .and_then(Value::as_u64)
                    .filter(|ms| (1..=60000).contains(ms))
                    .ok_or("timing must be 1..=60000 ms")?;
                next.timing.insert(name.to_owned(), ms as u32);
            }
            "timing-reset" => next.timing.clear(),
            _ => return Err("unknown presentation operation".to_owned()),
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("presentation revision exhausted")?;
        *self = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn presentation_controls_validate_and_keep_rules_out_of_the_document() {
        let mut state = PresentationState::default();
        for command in [
            json!({"op":"pause"}),
            json!({"op":"step"}),
            json!({"op":"replay"}),
            json!({"op":"speed","value":0.25}),
            json!({"op":"timing","name":"dice-read","ms":50}),
            json!({"op":"timing-reset"}),
            json!({"op":"resume"}),
        ] {
            state.apply(&command).unwrap();
        }
        assert_eq!(state.revision, 7);
        assert_eq!(state.steps, 1);
        assert_eq!(state.replays, 1);
        assert!(!state.paused);
        assert!(state.timing.is_empty());
        let before = serde_json::to_value(&state).unwrap();
        assert!(state.apply(&json!({"op":"speed","value":5})).is_err());
        assert!(
            state
                .apply(&json!({"op":"timing","name":"rules-turn","ms":10}))
                .is_err()
        );
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        assert!(before.get("heroes").is_none());
        assert!(before.get("outcomes").is_none());
    }
}
