/// Fetch/decode and crossfade lifetimes cannot extend the server's pending notice.
pub(super) fn plate_loading_visible(state: Option<&str>, server_pending: bool) -> bool {
    server_pending && state == Some("pending")
}

pub(super) const PLATE_TRANSITION_RECOVERY_MS: f64 = 1_100.0;

pub(super) fn plate_transition_expired(started_ms: Option<f64>, now_ms: f64) -> bool {
    started_ms.is_some_and(|started| now_ms - started >= PLATE_TRANSITION_RECOVERY_MS)
}

/// The title card's existing five-second establishing animation finishes before dialogue.
pub(super) const ESTABLISHING_BEAT_MS: f64 = 5_000.0;

pub(super) fn speaker_entry_ready(
    scene_started_ms: Option<f64>,
    picture_shown_ms: Option<f64>,
    now_ms: f64,
) -> bool {
    scene_started_ms.is_none_or(|started| {
        now_ms - started >= ESTABLISHING_BEAT_MS
            && picture_shown_ms.is_some_and(|shown| {
                shown >= started && now_ms - shown >= PLATE_TRANSITION_RECOVERY_MS
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_server_plate_state_clears_the_notice_even_with_stale_pending() {
        for state in ["ready", "failed"] {
            assert!(!plate_loading_visible(Some(state), true));
            assert!(!plate_loading_visible(Some(state), false));
        }
    }

    #[test]
    fn only_server_pending_keeps_the_plate_notice_visible() {
        assert!(plate_loading_visible(Some("pending"), true));
        assert!(!plate_loading_visible(Some("pending"), false));
        assert!(!plate_loading_visible(Some("none"), false));
    }

    #[test]
    fn stale_dissolve_retires_within_twelve_controlled_clock_ticks() {
        let started = Some(10_000.0);
        let retired_at = (1..=12)
            .map(|tick| 10_000.0 + f64::from(tick * 100))
            .find(|now| plate_transition_expired(started, *now));
        assert_eq!(retired_at, Some(11_100.0));
        assert!(!plate_transition_expired(Some(11_000.0), 11_100.0));
        assert!(!plate_transition_expired(None, 11_100.0));
    }

    #[test]
    fn first_speaker_waits_for_both_establishing_beat_and_new_picture() {
        assert!(!speaker_entry_ready(Some(0.0), Some(-1.0), 10_000.0));
        assert!(!speaker_entry_ready(Some(0.0), None, 10_000.0));
        assert!(!speaker_entry_ready(Some(0.0), Some(1_000.0), 4_999.0));
        assert!(speaker_entry_ready(Some(0.0), Some(1_000.0), 5_000.0));
        assert!(!speaker_entry_ready(Some(0.0), Some(8_000.0), 9_099.0));
        assert!(speaker_entry_ready(Some(0.0), Some(8_000.0), 9_100.0));
        assert!(
            speaker_entry_ready(None, None, 0.0),
            "reload does not replay entry"
        );
    }
}
