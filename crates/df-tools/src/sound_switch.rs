//! The display's one sound control as a pure state machine.
//!
//! Voice, music and effects all follow this single switch. A browser keeps audio locked until a
//! deliberate gesture, so the control starts `Locked`; the first deliberate press on the locked
//! control unlocks sound and turns it on, never leaving it muted. A mute choice lives only in
//! the page's memory: a reload starts locked and unmuted again.

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
    Muted,
    Unmuted,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SoundSwitch {
    unlocked: bool,
    muted: bool,
}

impl SoundSwitch {
    pub(crate) const fn new() -> Self {
        Self {
            unlocked: false,
            muted: false,
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

    /// A deliberate press on the control itself.
    pub(crate) fn press(&mut self) -> Press {
        match self.sound() {
            Sound::Locked => {
                self.unlocked = true;
                self.muted = false;
                Press::Unlocked
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

    /// A click or key press elsewhere on the page. It unlocks audio but changes no mute
    /// choice; returns whether it was the unlocking gesture.
    pub(crate) fn gesture_elsewhere(&mut self) -> bool {
        let first = !self.unlocked;
        self.unlocked = true;
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

    #[test]
    fn a_new_page_is_locked_and_not_muted() {
        let switch = SoundSwitch::new();
        assert_eq!(switch.sound(), Sound::Locked);
        assert!(!switch.is_muted());
    }

    #[test]
    fn the_first_press_on_the_locked_control_turns_sound_on() {
        let mut switch = SoundSwitch::new();
        assert_eq!(switch.press(), Press::Unlocked);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn the_control_toggles_between_on_and_muted() {
        let mut switch = SoundSwitch::new();
        switch.press();
        assert_eq!(switch.press(), Press::Muted);
        assert_eq!(switch.sound(), Sound::Muted);
        assert_eq!(switch.press(), Press::Unmuted);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_mute_choice_survives_only_inside_the_page_session() {
        let mut switch = SoundSwitch::new();
        switch.press();
        switch.press();
        assert_eq!(switch.sound(), Sound::Muted);
        // A reload builds a new switch: locked, and the first press enables sound.
        let mut reloaded = SoundSwitch::new();
        assert_eq!(reloaded.sound(), Sound::Locked);
        reloaded.press();
        assert_eq!(reloaded.sound(), Sound::On);
    }

    #[test]
    fn a_refused_play_relocks_and_the_next_press_enables_sound_not_mute() {
        let mut switch = SoundSwitch::new();
        switch.press();
        switch.press();
        switch.refused();
        assert_eq!(switch.sound(), Sound::Locked);
        assert_eq!(switch.press(), Press::Unlocked);
        assert_eq!(switch.sound(), Sound::On);
    }

    #[test]
    fn a_gesture_elsewhere_unlocks_without_changing_the_mute_choice() {
        let mut switch = SoundSwitch::new();
        assert!(switch.gesture_elsewhere());
        assert_eq!(switch.sound(), Sound::On);
        assert!(!switch.gesture_elsewhere());
        switch.press();
        assert_eq!(switch.sound(), Sound::Muted);
        assert!(!switch.gesture_elsewhere());
        assert_eq!(switch.sound(), Sound::Muted);
    }
}
