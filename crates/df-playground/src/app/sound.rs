//! Makes the sound match what the scene wants. It runs only when the controls, the scene or the
//! scenario changed, never for a frame that only moved figures.

use super::state::with_app;
use crate::audio;

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
