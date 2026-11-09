//! Makes the sound match what the scene wants. It runs only when the controls, the scene or the
//! scenario changed, never for a frame that only moved figures. Also this viewer's own sound
//! controls in the toolbar: the sound and music switches and the master volume.

use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlInputElement};

use super::page::render;
use super::report::schedule_report;
use super::state::with_app;
use crate::audio;
use crate::sound_plan::AudioPrefs;

/// The toolbar action of the sound switch, the music switch and the volume slider.
pub const SOUND_ACTION: &str = "audio:sound";
pub const MUSIC_ACTION: &str = "audio:music";
pub const VOLUME_ACTION: &str = "audio:volume";

/// A click on the sound or music switch: flips it, redraws the toolbar and tells the host.
/// Whether the action was one of them.
pub fn on_action(action: &str) -> bool {
    let prefs = audio::prefs();
    let next = match action {
        SOUND_ACTION => AudioPrefs {
            sound_on: !prefs.sound_on,
            ..prefs
        },
        MUSIC_ACTION => AudioPrefs {
            music_on: !prefs.music_on,
            ..prefs
        },
        _ => return false,
    };
    audio::set_prefs(next);
    render();
    schedule_report();
    true
}

/// The volume slider moved: the master volume takes its value at once (each step is small while
/// it is dragged, so the sound never jumps), and the host is told. The toolbar is not redrawn, so
/// the drag goes on.
pub fn on_input(event: &Event) -> bool {
    let Some(slider) = event
        .target()
        .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
        .filter(|input| input.get_attribute("data-action").as_deref() == Some(VOLUME_ACTION))
    else {
        return false;
    };
    let Ok(volume_percent) = slider.value().parse::<u8>() else {
        return true;
    };
    audio::set_prefs(AudioPrefs {
        volume_percent: volume_percent.min(100),
        ..audio::prefs()
    });
    schedule_report();
    true
}

/// Makes the music and the sounds match the scene: music is started, replaced, changed in place
/// or stopped, and each new sound cue is played once. It copies nothing: the audio module reads
/// the shown scenario and the cue list where they are.
pub fn sync_audio() {
    with_app(|app| {
        let music = match app.shown.as_deref() {
            Some(Ok(fixture)) => fixture.music.as_ref(),
            _ => None,
        };
        audio::sync_music(music, app.controls.device);
        audio::play_cues(&app.scene.cues, app.scene.cue_count, app.controls.device);
    });
}
