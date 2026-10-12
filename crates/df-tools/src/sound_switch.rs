//! The display's one sound control as a pure state machine.
//!
//! Voice, music and effects all follow this single switch. A browser keeps audio locked until a
//! deliberate gesture, so the control starts `Locked`; the first deliberate press on the locked
//! control unlocks sound and turns it on, never leaving it muted. A mute choice lives only in
//! the page's memory: a reload starts locked and unmuted again.
//!
//! One physical click fires several events (pointerdown, mousedown, pointerup, mouseup, click),
//! and the page unlocks on the first of them. When that unlocking gesture was a press ON the
//! control, the click that completes it is the same click and changes nothing, however long the
//! hand held the button: the rule follows the events, not a clock. A press within
//! [`ONE_GESTURE_MS`] of the unlock is also ignored, for a harness that delivers a click twice.
//! So a single click on the locked control always ends in `On`; only a later, separate click
//! toggles mute.

/// A press this soon after the gesture that unlocked sound is the same physical click.
pub(crate) const ONE_GESTURE_MS: u64 = 500;

/// What the viewer sees and what the audio engines obey.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Sound {
    Locked,
    On,
    Muted,
}

/// What the browser layer must do after a press on the control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Press {
    /// Audio was locked: play the unlock clip inside this gesture.
    Unlocked,
    /// The click that already unlocked sound: nothing more to do.
    SameGesture,
    Muted,
    Unmuted,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SoundSwitch {
    unlocked: bool,
    muted: bool,
    /// When the gesture that unlocked sound happened.
    unlocked_at_ms: u64,
    /// The unlocking gesture was a press on the control: its click is not a separate press.
    absorb_click: bool,
}

impl SoundSwitch {
    pub(crate) const fn new() -> Self {
        Self {
            unlocked: false,
            muted: false,
            unlocked_at_ms: 0,
            absorb_click: false,
        }
    }

    pub(crate) const fn sound(self) -> Sound {
        match (self.unlocked, self.muted) {
            (false, _) => Sound::Locked,
            (true, false) => Sound::On,
            (true, true) => Sound::Muted,
        }
    }

    pub(crate) const fn is_unlocked(self) -> bool {
        self.unlocked
    }

    pub(crate) const fn is_muted(self) -> bool {
        self.muted
    }

    /// A deliberate press on the control itself, at `now_ms`.
    pub(crate) fn press(&mut self, now_ms: u64) -> Press {
        match self.sound() {
            Sound::Locked => {
                self.unlocked = true;
                self.muted = false;
                self.unlocked_at_ms = now_ms;
                Press::Unlocked
            }
            Sound::On if self.absorb_click => {
                self.absorb_click = false;
                Press::SameGesture
            }
            Sound::On if now_ms.saturating_sub(self.unlocked_at_ms) < ONE_GESTURE_MS => {
                Press::SameGesture
            }
            Sound::On => {
                self.muted = true;
                Press::Muted
            }
            Sound::Muted => {
                self.muted = false;
                Press::Unmuted
            }
        }
    }

    /// A pointer or key press anywhere on the page, the control included (`on_control`). It
    /// unlocks audio but changes no mute choice; returns whether it was the unlocking gesture.
    /// Any press ends the previous one's claim on its click.
    pub(crate) fn gesture(&mut self, now_ms: u64, on_control: bool) -> bool {
        let first = !self.unlocked;
        self.absorb_click = first && on_control;
        if first {
            self.unlocked = true;
            self.unlocked_at_ms = now_ms;
        }
        first
    }

    /// The browser refused to play: audio is locked again until the next gesture.
    pub(crate) fn refused(&mut self) {
        self.unlocked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long enough after the unlocking click that a press is a separate one.
    const LATER_MS: u64 = 5_000;

    #[test]
    fn a_new_page_is_locked_and_not_muted() {
        let switch = SoundSwitch::new();
        assert_eq!(switch.sound(), Sound::Locked);
        assert!(!switch.is_muted());
        assert!(!switch.is_unlocked());
    }

    #[test]
    fn the_first_press_on_the_locked_control_turns_sound_on() {
        let mut switch = SoundSwitch::new();
        assert_eq!(switch.press(0), Press::Unlocked);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn the_control_toggles_between_on_and_muted() {
        let mut switch = SoundSwitch::new();
        switch.press(0);
        assert_eq!(switch.press(LATER_MS), Press::Muted);
        assert_eq!(switch.sound(), Sound::Muted);
        assert_eq!(switch.press(2 * LATER_MS), Press::Unmuted);
        assert_eq!(switch.sound(), Sound::On);
    }

    /// The real event order of one physical click on the locked control: the page unlocks on the
    /// pointer press, and the click handler that follows must not mute what it just unlocked.
    #[test]
    fn one_real_click_on_the_locked_control_ends_in_on_and_a_later_click_mutes() {
        let mut switch = SoundSwitch::new();
        // pointerdown, mousedown: the page-level gesture listener (the control is no exception).
        assert!(switch.gesture(1_000, true));
        assert!(!switch.gesture(1_001, true));
        // pointerup, mouseup: no handler. click, about 90 ms after the press:
        assert_eq!(switch.press(1_090), Press::SameGesture);
        assert_eq!(switch.sound(), Sound::On);
        // A separate click much later toggles mute, and the next one unmutes.
        assert_eq!(switch.press(1_090 + LATER_MS), Press::Muted);
        assert_eq!(switch.press(1_090 + 2 * LATER_MS), Press::Unmuted);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_slow_click_on_the_locked_control_still_ends_in_on_whatever_the_hold_time() {
        for hold_ms in [0, 80, 700, 5_000] {
            let mut switch = SoundSwitch::new();
            assert!(switch.gesture(1_000, true));
            assert_eq!(
                switch.press(1_000 + hold_ms),
                Press::SameGesture,
                "{hold_ms}"
            );
            assert_eq!(switch.sound(), Sound::On);
            // The next, separate click mutes.
            assert_eq!(switch.press(1_000 + hold_ms + LATER_MS), Press::Muted);
        }
    }

    #[test]
    fn a_press_dragged_off_the_control_gives_the_next_real_click_its_own_effect() {
        let mut switch = SoundSwitch::new();
        assert!(switch.gesture(0, true));
        // No click ever arrives (the pointer left); the next press anywhere ends the claim.
        assert!(!switch.gesture(LATER_MS, true));
        assert_eq!(switch.press(LATER_MS + 80), Press::Muted);
    }

    #[test]
    fn a_click_delivered_twice_in_one_gesture_still_ends_in_on() {
        let mut switch = SoundSwitch::new();
        assert_eq!(switch.press(10), Press::Unlocked);
        assert_eq!(switch.press(14), Press::SameGesture);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_keyboard_press_on_the_focused_locked_control_ends_in_on() {
        let mut switch = SoundSwitch::new();
        // keydown unlocks, then the click the browser synthesises from Enter or Space.
        assert!(switch.gesture(300, true));
        assert_eq!(switch.press(305), Press::SameGesture);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_mute_choice_survives_only_inside_the_page_session() {
        let mut switch = SoundSwitch::new();
        switch.press(0);
        switch.press(LATER_MS);
        assert_eq!(switch.sound(), Sound::Muted);
        // A reload builds a new switch: locked, and the first press enables sound.
        let mut reloaded = SoundSwitch::new();
        assert_eq!(reloaded.sound(), Sound::Locked);
        reloaded.press(0);
        assert_eq!(reloaded.sound(), Sound::On);
    }

    #[test]
    fn a_refused_play_relocks_and_the_next_press_enables_sound_not_mute() {
        let mut switch = SoundSwitch::new();
        switch.press(0);
        switch.press(LATER_MS);
        switch.refused();
        assert_eq!(switch.sound(), Sound::Locked);
        assert_eq!(switch.press(2 * LATER_MS), Press::Unlocked);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_gesture_elsewhere_unlocks_without_changing_the_mute_choice() {
        let mut switch = SoundSwitch::new();
        assert!(switch.gesture(0, false));
        assert_eq!(switch.sound(), Sound::On);
        assert!(!switch.gesture(1, false));
        assert_eq!(switch.press(LATER_MS), Press::Muted);
        assert_eq!(switch.sound(), Sound::Muted);
        assert!(!switch.gesture(2 * LATER_MS, false));
        assert_eq!(switch.sound(), Sound::Muted);
    }
}
