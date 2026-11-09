//! The scene progress line (component 3): a play ring, "Scene 1 of 5 • Introduction" and a track
//! of one segment per scene, the current one glowing with a diamond (render 02). It is drawn on
//! the bottom vignette, not on glass.

use serde::{Deserialize, Serialize};

use crate::kit::{Step, current_index, shown, steps};
use crate::overlay::Progress;

/// The progress line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressLine {
    pub progress: Progress,
    /// Whether the round play mark leads the line.
    #[serde(default)]
    pub show_play: bool,
    /// The word of the pointed continue capsule at the line's right end (render 02); `None`
    /// draws none, as when the narration box carries its own hint.
    #[serde(default)]
    pub continue_label: Option<String>,
}

impl ProgressLine {
    /// A run of one scene has no line.
    pub fn is_drawn(&self) -> bool {
        self.progress.total >= 2
    }

    /// "Scene 2 of 5", with the index held inside the run.
    pub fn count_text(&self) -> String {
        format!(
            "Scene {} of {}",
            current_index(&self.progress),
            self.progress.total
        )
    }

    /// The scene's name after the bullet, when it has one.
    pub fn title(&self) -> Option<&str> {
        shown(self.progress.title.as_deref())
    }

    pub fn continue_label(&self) -> Option<&str> {
        shown(self.continue_label.as_deref())
    }

    pub fn segments(&self) -> Vec<Step> {
        steps(&self.progress)
    }
}
