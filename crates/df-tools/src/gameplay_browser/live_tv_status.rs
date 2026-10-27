/// Loading covers server preparation and the current browser fetch/decode, but not crossfade.
pub(super) fn plate_loading_visible(
    state: Option<&str>,
    server_pending: bool,
    client_loading: bool,
) -> bool {
    (server_pending && state == Some("pending")) || (client_loading && state == Some("ready"))
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PlaceOutcome {
    Ignore,
    Show { url: String, primary: bool },
    Retry(String),
    Keep,
}

pub(super) struct PlaceAttempt {
    scene: String,
    generation: String,
    candidates: Vec<String>,
    index: usize,
    terminal: bool,
}

impl PlaceAttempt {
    pub(super) fn new(scene: String, generation: String, candidates: Vec<String>) -> Self {
        Self {
            scene,
            generation,
            candidates,
            index: 0,
            terminal: false,
        }
    }

    fn is_current(&self, scene: &str, generation: Option<&str>) -> bool {
        !self.terminal && self.scene == scene && generation == Some(self.generation.as_str())
    }

    pub(super) fn loaded(&mut self, scene: &str, generation: Option<&str>) -> PlaceOutcome {
        if !self.is_current(scene, generation) {
            return PlaceOutcome::Ignore;
        }
        self.terminal = true;
        match self.candidates.get(self.index) {
            Some(url) => PlaceOutcome::Show {
                url: url.clone(),
                primary: self.index == 0,
            },
            None => PlaceOutcome::Ignore,
        }
    }

    pub(super) fn failed(
        &mut self,
        scene: &str,
        generation: Option<&str>,
        has_retained: bool,
    ) -> PlaceOutcome {
        if !self.is_current(scene, generation) {
            return PlaceOutcome::Ignore;
        }
        if !has_retained && let Some(url) = self.candidates.get(self.index + 1) {
            self.index += 1;
            return PlaceOutcome::Retry(url.clone());
        }
        self.terminal = true;
        PlaceOutcome::Keep
    }
}

pub(super) fn place_selection_changed(previous: Option<&[String]>, next: &[String]) -> bool {
    previous != Some(next)
}

pub(super) fn show_retained_place(
    plate_state: Option<&str>,
    place_loaded: bool,
    place_from_prior_scene: bool,
    plate_loaded: bool,
    explicit_fallback: bool,
) -> bool {
    explicit_fallback
        || (place_loaded
            && !plate_loaded
            && (matches!(plate_state, None | Some("none" | "failed"))
                || (plate_state == Some("unavailable") && place_from_prior_scene)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_server_plate_state_clears_the_notice_even_with_stale_pending() {
        for state in ["ready", "failed"] {
            assert!(!plate_loading_visible(Some(state), true, false));
            assert!(!plate_loading_visible(Some(state), false, false));
        }
    }

    #[test]
    fn only_server_pending_keeps_the_plate_notice_visible() {
        assert!(plate_loading_visible(Some("pending"), true, false));
        assert!(!plate_loading_visible(Some("pending"), false, false));
        assert!(!plate_loading_visible(Some("none"), false, false));
    }

    #[test]
    fn ready_plate_stays_loading_only_during_its_browser_request() {
        assert!(plate_loading_visible(Some("ready"), false, true));
        assert!(!plate_loading_visible(Some("ready"), false, false));
        assert!(!plate_loading_visible(Some("failed"), false, true));
        assert!(!plate_loading_visible(Some("none"), false, true));
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
    fn narration_reserves_the_party_bar_and_wrapped_choice_row_in_every_scene_shot() {
        let styles = include_str!("live_stage.css");
        let with_choices = styles
            .split_once(".tv-stage[data-tv-mode=scene]:not(:has(.tv-figures[data-arrange=combat])) .tv-safe>.tv-stagebox.tv-stagebox {")
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0;
        assert!(with_choices.contains("margin-bottom: calc(var(--u)*7.3 + var(--fs-label)*4.8);"));
        let without_choices = styles
            .split_once(".tv-stage[data-tv-mode=scene]:not(:has(.tv-figures[data-arrange=combat])):is([data-chips=\"0\"],[data-chips=off]) .tv-safe>.tv-stagebox.tv-stagebox {")
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0;
        assert!(without_choices.contains("margin-bottom: calc(var(--u)*6.4);"));
    }

    #[test]
    fn long_narration_stays_scrollable_after_stage_overrides() {
        let stage = include_str!("live_stage.css");
        let rule = stage
            .split_once(
                ".tv-narration,\n.tv-stage:has(.tv-figures[data-arrange=stage]) .tv-narration {",
            )
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0;
        assert!(rule.contains("-webkit-line-clamp: unset;"));
        assert!(rule.contains("overflow-x: hidden;"));
        assert!(rule.contains("overflow-y: auto;"));
        assert!(rule.contains("overflow-wrap: anywhere;"));
    }

    #[test]
    fn the_dungeon_master_label_keeps_its_sentence_case() {
        let styles = include_str!("live_stage.css");
        let rule = styles
            .split_once(".tv-dm-label")
            .unwrap()
            .1
            .split_once('{')
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0;
        let declarations: String = rule
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert!(declarations.contains("letter-spacing:normal;"));
        assert!(declarations.contains("text-transform:none;"));
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
    fn a_loaded_place_picture_is_not_covered_by_the_plate_wait_gradient() {
        let styles = include_str!("live_plate.css");
        let gradient = styles
            .lines()
            .find(|line| line.contains("background-image:radial-gradient(ellipse at 50% 42%"))
            .unwrap();
        assert_eq!(gradient.matches(":not([data-place-picture])").count(), 3);
    }

    #[test]
    fn failed_tv_images_are_hidden_until_a_successful_load() {
        let styles = include_str!("live_tv.css");
        assert!(
            styles.contains(".tv-stage img[data-image-failed]{display:none;visibility:hidden}")
        );
    }

    #[test]
    fn failed_scene_picture_keeps_the_confirmed_previous_picture_without_retry() {
        let mut attempt = PlaceAttempt::new(
            "first-scene".into(),
            "2".into(),
            vec!["missing".into(), "alternate".into()],
        );
        assert_eq!(
            attempt.failed("first-scene", Some("2"), true),
            PlaceOutcome::Keep
        );
        assert_eq!(
            attempt.loaded("first-scene", Some("2")),
            PlaceOutcome::Ignore
        );
        assert!(!place_selection_changed(
            Some(&["missing".into(), "alternate".into()]),
            &["missing".into(), "alternate".into()]
        ));
    }

    #[test]
    fn without_a_confirmed_picture_a_failed_candidate_can_use_the_next_one() {
        let mut attempt = PlaceAttempt::new(
            "first-scene".into(),
            "2".into(),
            vec!["missing".into(), "fallback".into()],
        );
        assert_eq!(
            attempt.failed("first-scene", Some("2"), false),
            PlaceOutcome::Retry("fallback".into())
        );
        assert_eq!(
            attempt.loaded("first-scene", Some("2")),
            PlaceOutcome::Show {
                url: "fallback".into(),
                primary: false
            }
        );
    }

    #[test]
    fn an_old_scene_or_superseded_generation_cannot_replace_the_current_picture() {
        let mut attempt = PlaceAttempt::new("scene-a".into(), "1".into(), vec!["old".into()]);
        assert_eq!(attempt.loaded("scene-b", Some("1")), PlaceOutcome::Ignore);
        assert_eq!(
            attempt.failed("scene-a", Some("2"), false),
            PlaceOutcome::Ignore
        );
        assert_eq!(attempt.loaded("scene-a", None), PlaceOutcome::Ignore);
        assert_eq!(
            attempt.loaded("scene-a", Some("1")),
            PlaceOutcome::Show {
                url: "old".into(),
                primary: true
            }
        );
    }

    #[test]
    fn a_missing_or_failed_plate_keeps_a_loaded_place_visible() {
        for state in [None, Some("none"), Some("failed")] {
            assert!(show_retained_place(state, true, true, false, false));
            assert!(!show_retained_place(state, false, true, false, false));
            assert!(!show_retained_place(state, true, true, true, false));
        }
        assert!(!show_retained_place(
            Some("pending"),
            true,
            true,
            false,
            false
        ));
        assert!(show_retained_place(
            Some("unavailable"),
            true,
            true,
            false,
            false
        ));
        assert!(!show_retained_place(
            Some("unavailable"),
            true,
            false,
            false,
            false
        ));
        assert!(show_retained_place(
            Some("pending"),
            true,
            true,
            false,
            true
        ));
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
