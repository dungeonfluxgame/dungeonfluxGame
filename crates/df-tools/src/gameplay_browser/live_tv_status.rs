/// Fetch/decode and crossfade lifetimes cannot extend the server's pending notice.
pub(super) fn plate_loading_visible(state: Option<&str>, server_pending: bool) -> bool {
    server_pending && state == Some("pending")
}

pub(super) const PLATE_TRANSITION_RECOVERY_MS: f64 = 1_100.0;

pub(super) fn plate_transition_expired(started_ms: Option<f64>, now_ms: f64) -> bool {
    started_ms.is_some_and(|started_ms| now_ms - started_ms >= PLATE_TRANSITION_RECOVERY_MS)
}

/// The title card's existing five-second establishing animation finishes before dialogue.
pub(super) const ESTABLISHING_BEAT_MS: f64 = 5_000.0;

pub(super) fn speaker_entry_ready(
    scene_started_ms: Option<f64>,
    picture_shown_ms: Option<f64>,
    now_ms: f64,
) -> bool {
    scene_started_ms.is_none_or(|scene_started_ms| {
        now_ms - scene_started_ms >= ESTABLISHING_BEAT_MS
            && picture_shown_ms.is_some_and(|picture_shown_ms| {
                picture_shown_ms >= scene_started_ms
                    && now_ms - picture_shown_ms >= PLATE_TRANSITION_RECOVERY_MS
            })
    })
}

pub(super) fn lobby_readiness(ready_seats: usize, seats: usize) -> String {
    if seats == 0 {
        "Waiting for heroes. Join with the room code.".to_owned()
    } else {
        format!("{ready_seats} of {seats} heroes ready")
    }
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

    #[test]
    fn lobby_invites_heroes_before_the_first_join_and_counts_them_afterward() {
        assert_eq!(
            lobby_readiness(0, 0),
            "Waiting for heroes. Join with the room code."
        );
        assert_eq!(lobby_readiness(2, 4), "2 of 4 heroes ready");
    }

    #[test]
    fn caption_scrolls_inside_its_row_above_the_party_bar() {
        let styles = include_str!("live_tv.css");
        assert!(styles.contains("grid-template-rows:auto auto minmax(0,1fr) auto auto"));
        assert!(styles.contains(".tv-stagebox{grid-row:4;"));
        assert!(styles.contains(".tv-partybar{grid-row:5;"));
        assert!(styles.contains("max-height:min(28vh,calc(var(--u)*28));overflow-y:auto"));
        assert!(!styles.contains("-webkit-line-clamp:6"));
    }

    #[test]
    fn lobby_readiness_and_hero_cards_outweigh_the_wordmark() {
        let styles = include_str!("live_tv.css");
        assert!(styles.contains(".tv-roster .tv-gathered{font-size:"));
        assert!(styles.contains(".tv-roster .tv-member-name{font-size:"));
        assert!(styles.contains(".tv-title-block .tv-wordmark{height:calc(var(--u)*4.8)}"));
    }

    #[test]
    fn retained_place_plate_sits_under_the_current_scene_graph() {
        let plate_styles = include_str!("live_plate.css");
        let stage_styles = include_str!("live_stage.css");
        assert!(plate_styles.contains(
            ".tv-stage[data-place-plate-retained] .tv-plate{z-index:1;opacity:1;visibility:visible"
        ));
        assert!(stage_styles.contains(".tv-scene-graph{position:absolute;z-index:2;"));
    }

    #[test]
    fn sound_opt_out_keeps_only_the_small_corner_control_visible() {
        let styles = include_str!("live_tv.css");
        assert!(styles.contains(
            "body:has(.tv-stage[data-sound-opted-out]) .tv-voice-control{opacity:.84!important;visibility:visible!important;animation:none!important}"
        ));
        assert!(
            styles
                .contains(".tv-stage .tv-voice-prompt[data-visible=true]{display:block!important}")
        );
    }
}
