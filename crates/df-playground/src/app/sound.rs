//! Makes the sound match what the scene wants. It runs only when the controls, the scene or the
//! scenario changed, never for a frame that only moved figures. Also the master volume slider of
//! the toolbar; the sound and music switches are the toolbar's `Switch`es, sent to the host like
//! the others.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlInputElement};

use super::frames::send_change;
use super::report::schedule_report;
use super::state::with_app;
use crate::audio;
use crate::controls::ControlsChange;
use crate::sound_plan::AudioPrefs;

/// The `data-action` of the volume slider.
pub const VOLUME_ACTION: &str = "audio:volume";

/// The volume the slider of the event holds, when the event is the slider's.
fn slider_of(event: &Event) -> Option<u8> {
    let slider = event
        .target()
        .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
        .filter(|input| input.get_attribute("data-action").as_deref() == Some(VOLUME_ACTION))?;
    let volume_percent = slider.value().parse::<u8>().ok()?;
    Some(volume_percent.min(100))
}

/// The volume slider moved: the master volume takes its value at once (each step is small while
/// it is dragged, so the sound never jumps). The toolbar is not redrawn, so the drag goes on, and
/// the host hears the value only when the slider is let go (`on_release`), so a drag is one
/// change, not a hundred. Whether the event was the slider's.
pub fn on_input(event: &Event) -> bool {
    let Some(volume_percent) = slider_of(event) else {
        return false;
    };
    audio::set_prefs(AudioPrefs {
        volume_percent,
        ..audio::prefs()
    });
    schedule_report();
    true
}

/// The volume slider was let go: the host keeps the volume it ended on, for the next run. Whether
/// the event was the slider's.
pub fn on_release(event: &Event) -> bool {
    let Some(volume_percent) = slider_of(event) else {
        return false;
    };
    spawn_local(send_change(ControlsChange::Volume(volume_percent)));
    true
}

/// Takes `prefs`, which the host now holds, as the page's. A volume dragged and not yet let go is
/// left alone until the host's own volume changes, because the host only hears it on release.
pub fn take_host_prefs(prefs: AudioPrefs) {
    audio::set_prefs(prefs);
    schedule_report();
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
