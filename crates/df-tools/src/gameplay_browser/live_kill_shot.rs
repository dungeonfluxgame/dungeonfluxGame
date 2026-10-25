//! K1: the kill-shot clip on the shared display.
//!
//! Once a ready kill shot arrives for the fight just finished, it plays
//! full-frame on the TV once ("Kill shot", the killer's and the victim's
//! names, muted unless sound is on), then hands back to whatever the TV was
//! showing. A pending clip shows nothing. It never delays a line, the fight's
//! close or the next scene, and it never replays after it was shown: the
//! (id, epoch) pair is claimed before playback starts, so a replayed view, a
//! reconnect or a repeated snapshot with the same pair shows nothing again.
use super::*;
use df_observe::presentation::{ComponentPhase, PresentationComponent};
use std::{cell::RefCell, collections::HashSet, rc::Rc};
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;

/// Which (id, epoch) pairs this page has already shown.
#[derive(Default)]
struct Shown {
    pairs: HashSet<(String, u64)>,
}

/// The TV's kill-shot layer: one video, its caption, and what it claimed.
pub(super) struct KillShotPlayer {
    layer: Element,
    video: HtmlVideoElement,
    caption: Element,
    shown: Rc<RefCell<Shown>>,
    playing: Rc<RefCell<Option<(String, u64)>>>,
    _listeners: Vec<Closure<dyn FnMut()>>,
}

impl KillShotPlayer {
    pub(super) fn new(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let layer = document.create_element("section")?;
        layer.set_attribute("class", "tv-kill-shot")?;
        layer.set_attribute("hidden", "")?;
        layer.set_attribute("aria-hidden", "true")?;
        let video = document
            .create_element("video")?
            .dyn_into::<HtmlVideoElement>()?;
        video.set_attribute("class", "tv-kill-shot-clip")?;
        video.set_attribute("playsinline", "")?;
        video.set_attribute("preload", "auto")?;
        video.set_muted(true);
        layer.append_child(&video)?;
        let caption = document.create_element("div")?;
        caption.set_attribute("class", "tv-kill-shot-caption")?;
        layer.append_child(&caption)?;
        parent.append_child(&layer)?;

        let shown = Rc::new(RefCell::new(Shown::default()));
        let playing: Rc<RefCell<Option<(String, u64)>>> = Rc::new(RefCell::new(None));
        let mut listeners = Vec::new();
        {
            let playing = Rc::clone(&playing);
            let layer = layer.clone();
            let ended = Closure::wrap(Box::new(move || {
                finish(&layer, &playing, "ended");
            }) as Box<dyn FnMut()>);
            video.add_event_listener_with_callback("ended", ended.as_ref().unchecked_ref())?;
            listeners.push(ended);
        }
        {
            let playing = Rc::clone(&playing);
            let layer = layer.clone();
            let failed = Closure::wrap(Box::new(move || {
                finish(&layer, &playing, "failed");
            }) as Box<dyn FnMut()>);
            video.add_event_listener_with_callback("error", failed.as_ref().unchecked_ref())?;
            listeners.push(failed);
        }
        Ok(Self {
            layer,
            video,
            caption,
            shown,
            playing,
            _listeners: listeners,
        })
    }

    /// Shows a ready kill shot once, or does nothing. Pending, failed and
    /// absent shots show nothing; an already-shown (id, epoch) never replays.
    pub(super) fn apply(&self, shot: Option<&rpc::KillShot>) -> Result<(), JsValue> {
        let Some(shot) = shot else {
            return Ok(());
        };
        if shot.state != rpc::HomeVideoState::Ready as i32 || shot.clip_path.is_empty() {
            return Ok(());
        }
        if shot.id.is_empty() {
            return Ok(());
        }
        // Claim before playback: even a refused play counts as shown, so a
        // replayed view cannot retry it into a second showing.
        if !self
            .shown
            .borrow_mut()
            .pairs
            .insert((shot.id.clone(), shot.epoch))
        {
            return Ok(());
        }
        if self.playing.borrow().is_some() {
            return Ok(());
        }
        let muted = super::live_voice::effects_muted().unwrap_or(true);
        self.video.set_muted(muted);
        self.caption.set_text_content(Some(&format!(
            "Kill shot — {} fells {}",
            shot.killer, shot.victim
        )));
        self.video.set_src(&shot.clip_path);
        self.video.load();
        self.layer.remove_attribute("hidden")?;
        self.layer.set_attribute("aria-hidden", "false")?;
        *self.playing.borrow_mut() = Some((shot.id.clone(), shot.epoch));
        record_display(
            &shot.id,
            shot.sequence,
            ComponentPhase::Opened,
            "kill_shot_started",
        );
        match self.video.play() {
            Ok(promise) => {
                let layer = self.layer.clone();
                let playing = Rc::clone(&self.playing);
                wasm_bindgen_futures::spawn_local(async move {
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                        finish(&layer, &playing, "play_refused");
                    }
                });
            }
            Err(_) => finish(&self.layer, &self.playing, "play_refused"),
        }
        Ok(())
    }

    /// Stops any showing at once; the TV goes back to what it was showing.
    pub(super) fn release(&self) {
        finish(&self.layer, &self.playing, "released");
        let _paused = self.video.pause();
        let _removed = self.video.remove_attribute("src");
        self.video.load();
    }
}

impl Drop for KillShotPlayer {
    fn drop(&mut self) {
        self.release();
    }
}

fn finish(layer: &Element, playing: &Rc<RefCell<Option<(String, u64)>>>, reason: &'static str) {
    let taken = playing.borrow_mut().take();
    let _hidden = layer.set_attribute("hidden", "");
    let _aria = layer.set_attribute("aria-hidden", "true");
    if let Some((id, _epoch)) = taken {
        let sequence_number = id
            .strip_prefix("kill-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0);
        record_display(&id, sequence_number, ComponentPhase::Hidden, reason);
    }
}

/// A DISPLAY record for the clip starting and ending on the TV.
fn record_display(id: &str, sequence_number: u64, phase: ComponentPhase, reason: &'static str) {
    super::presentation_timing::component(
        None,
        PresentationComponent::TitleCard,
        super::presentation_timing::identity(id),
        "kill_shot",
        phase,
        reason,
        u32::try_from(sequence_number.min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
    );
}

#[cfg(test)]
mod tests {
    use super::Shown;
    use std::collections::HashSet;

    #[test]
    fn a_pair_is_claimed_once() {
        let mut shown = Shown {
            pairs: HashSet::new(),
        };
        assert!(shown.pairs.insert(("kill-1-a".to_owned(), 3)));
        assert!(!shown.pairs.insert(("kill-1-a".to_owned(), 3)));
        assert!(shown.pairs.insert(("kill-1-a".to_owned(), 4)));
        assert!(shown.pairs.insert(("kill-2-a".to_owned(), 3)));
    }
}
