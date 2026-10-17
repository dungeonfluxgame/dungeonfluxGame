//! The narrow port from place music to the project's single generation hub.
//!
//! The hub (`gameplay/resources.rs`) owns every generated resource's registry, single flight
//! per key, timeout, attempt limit, per-run cap and `RESOURCE` audit lines. Place music neither
//! duplicates nor bypasses any of that: it asks for a [`MusicSetSpec`] and shows what is
//! reported. `HubMusic` in `resource.rs` implements this port over the real hub; tests use a
//! stand-in that records the requests.

use super::generator::MusicSetSpec;

/// What the hub reports about one set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum MusicStatus {
    /// Generated, mastered and served: the paths the display loads.
    Ready(ReadyMusicSet),
    /// Requested or being generated.
    Pending,
    /// Generation failed; the reason is the hub's closed label. Never an implicit retry.
    Failed(String),
    /// The hub is serving a stand-in and says why (off, over budget, no credential).
    Fallback(String),
}

impl MusicStatus {
    pub(in crate::gameplay) fn label(&self) -> &'static str {
        match self {
            Self::Ready(_) => "ready",
            Self::Pending => "pending",
            Self::Failed(_) => "failed",
            Self::Fallback(_) => "fallback",
        }
    }
}

/// The served paths of a ready set: layers in track order bed, motif, tension, combat, and the
/// place's own ambience when its call succeeded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ReadyMusicSet {
    pub(in crate::gameplay) layer_paths: [String; 4],
    pub(in crate::gameplay) ambience_path: Option<String>,
}

/// `get_or_request`, `status`, `invalidate` and `regenerate`, as the hub offers them.
pub(in crate::gameplay) trait MusicHub: Send + Sync {
    /// Idempotent: the first call for a key starts generation, later calls only report.
    fn get_or_request(&self, set: &MusicSetSpec) -> MusicStatus;
    /// The state of a set without requesting it.
    fn status(&self, set: &MusicSetSpec) -> MusicStatus;
    /// Forgets a set's result so the next request generates again.
    fn invalidate(&self, set: &MusicSetSpec);
    /// Invalidates and requests in one step.
    fn regenerate(&self, set: &MusicSetSpec);
}
