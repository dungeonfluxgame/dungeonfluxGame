//! The TV's screen-space effects (owner, 2026-10-17): the stage blur behind the cast, the slow
//! push-in and rotating drift, the heavy scrim, the film grain, the breathing vignette, the
//! firelight glows, the dust and the colour grade. The pictures are photographs, so the effects
//! that paint over them (firelight, dust, colour grade) start off (owner, 2026-10-17: "make sure
//! they are photograph like"); `dev fx <effect> on|off` switches any of them for every display,
//! and the display view carries the names switched off.
//! It is presentation only and never touches game state.

use std::{
    collections::BTreeSet,
    sync::{LazyLock, Mutex, PoisonError},
};

use df_protocol::common as rpc;

/// Every effect the TV can switch off, by the name the display and the CLI use. The client's list
/// (`presentation::SCREEN_EFFECTS`) must name the same effects.
pub(super) const EFFECTS: [&str; 8] = [
    "blur",
    "drift",
    "scrim",
    "grain",
    "vignette",
    "firelight",
    "dust",
    "grade",
];

/// Effects that paint over the photograph and start switched off.
const OFF_BY_DEFAULT: [&str; 3] = ["firelight", "dust", "grade"];

static OFF: LazyLock<Mutex<BTreeSet<&'static str>>> =
    LazyLock::new(|| Mutex::new(OFF_BY_DEFAULT.into_iter().collect()));

fn off() -> std::sync::MutexGuard<'static, BTreeSet<&'static str>> {
    OFF.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Names the effects switched off on a display view; any other audience is left as it is.
pub(super) fn apply(message: &mut rpc::ViewMessage) {
    if let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() {
        display.effects_off = off().iter().map(|name| (*name).to_owned()).collect();
    }
}

/// Switches `name` on or off. An unknown name is refused and nothing changes.
pub(super) fn set(name: &str, on: bool) -> Result<(), UnknownEffect> {
    let effect = EFFECTS
        .iter()
        .copied()
        .find(|effect| *effect == name)
        .ok_or(UnknownEffect)?;
    if on {
        off().remove(effect);
    } else {
        off().insert(effect);
    }
    super::actor::record_audit_line(
        "gameplay.display",
        format!("DISPLAY_FX effect={effect} on={on}"),
    );
    Ok(())
}

pub(super) fn states() -> Vec<(&'static str, bool)> {
    let off = off();
    EFFECTS
        .iter()
        .map(|effect| (*effect, !off.contains(effect)))
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct UnknownEffect;

#[cfg(test)]
mod tests {
    use super::*;

    fn display_view() -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView::default(),
            )),
            ..Default::default()
        }
    }

    fn effects_off(message: &rpc::ViewMessage) -> Vec<String> {
        match &message.audience {
            Some(rpc::view_message::Audience::Display(display)) => display.effects_off.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn painting_effects_start_off_and_a_switched_off_one_reaches_the_display_view() {
        // One test owns the process-wide state, so the steps run in order.
        let mut view = display_view();
        apply(&mut view);
        assert_eq!(effects_off(&view), vec!["dust", "firelight", "grade"]);
        for effect in EFFECTS {
            set(effect, true).unwrap();
        }
        assert!(states().iter().all(|(_, on)| *on));
        let mut view = display_view();
        apply(&mut view);
        assert!(effects_off(&view).is_empty());

        set("grain", false).unwrap();
        set("blur", false).unwrap();
        let mut view = display_view();
        apply(&mut view);
        assert_eq!(effects_off(&view), vec!["blur", "grain"]);

        assert_eq!(set("lens-flare", false), Err(UnknownEffect));
        set("grain", true).unwrap();
        set("blur", true).unwrap();
        let mut view = display_view();
        apply(&mut view);
        assert!(effects_off(&view).is_empty());
    }
}
