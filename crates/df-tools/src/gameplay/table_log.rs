//! The public table log: the latest lines of who said and did what, shown to every screen.
//!
//! Everything here is table talk: an accepted action's public phrase, the words a hero types to
//! the scene's speaker, and the speaker's in-character reply. The private clue and any other
//! member-private text are never written here by any caller. The log is a bounded in-memory ring
//! per run; it is presentation only and never an authority for game state.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
};

use df_protocol::common as rpc;
use prost::Message;
use tokio::sync::watch;

/// How many of the latest lines a view carries.
pub(super) const RING_LINES: usize = 24;
/// The most a whole view may encode to once the log is attached: the transport carries 8192
/// bytes per message, and the rest of the view (character sheet, combat) comes first.
pub(super) const VIEW_BYTE_BUDGET: usize = 8000;
/// Longest text of one line, in characters.
pub(super) const LINE_CHARACTERS: usize = 420;

struct Ring {
    next_sequence: u64,
    lines: VecDeque<rpc::TableLine>,
    /// Counts every change a view shows (a line appended, a reply starting or ending), so a
    /// view whose game revision did not move can still be told from the one before it.
    version: u64,
    /// The speakers whose reply to a hero's typed line is being written right now.
    replying: Vec<String>,
}

pub(super) struct TableLog {
    /// A disabled log (qualification mode) records nothing and attaches nothing.
    enabled: bool,
    ring: Mutex<Ring>,
    /// Carries the latest sequence so a watcher rebuilds its view when a line is appended.
    wake: watch::Sender<u64>,
}

impl TableLog {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::with(true))
    }

    pub(super) fn disabled() -> Arc<Self> {
        Arc::new(Self::with(false))
    }

    fn with(enabled: bool) -> Self {
        Self {
            enabled,
            ring: Mutex::new(Ring {
                next_sequence: 1,
                lines: VecDeque::new(),
                version: 0,
                replying: Vec::new(),
            }),
            wake: watch::channel(0).0,
        }
    }

    /// Subscribes to appended lines; a change means "publish a fresh view".
    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.wake.subscribe()
    }

    /// Appends one line. Control characters become spaces, the text is cut to
    /// [`LINE_CHARACTERS`] characters and an empty line is dropped.
    pub(super) fn push(
        &self,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) {
        if !self.enabled {
            return;
        }
        let text: String = text
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(LINE_CHARACTERS)
            .collect();
        if text.is_empty() {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let sequence = ring.next_sequence;
            ring.next_sequence += 1;
            ring.lines.push_back(rpc::TableLine {
                sequence,
                speaker: speaker.chars().take(48).collect(),
                role: role as i32,
                hero_index,
                kind: kind as i32,
                text,
            });
            while ring.lines.len() > RING_LINES {
                ring.lines.pop_front();
            }
            ring.version += 1;
            ring.version
        };
        self.wake.send_replace(version);
    }

    /// Marks `speaker` as writing a reply until the returned guard is dropped, so every screen
    /// can show that someone is thinking. Presentation only: nothing waits on it.
    pub(super) fn reply_pending(self: &Arc<Self>, speaker: &str) -> ReplyPending {
        if self.enabled {
            let version = {
                let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
                ring.replying.push(speaker.chars().take(48).collect());
                ring.version += 1;
                ring.version
            };
            self.wake.send_replace(version);
        }
        ReplyPending {
            log: self.clone(),
            speaker: speaker.chars().take(48).collect(),
        }
    }

    fn reply_done(&self, speaker: &str) {
        if !self.enabled {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(position) = ring.replying.iter().position(|known| known == speaker) {
                ring.replying.remove(position);
            }
            ring.version += 1;
            ring.version
        };
        self.wake.send_replace(version);
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> Vec<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .cloned()
            .collect()
    }

    /// Adds the public feed to the view of either audience.
    pub(super) fn attach(&self, view: &mut rpc::ViewMessage) {
        if !self.enabled {
            return;
        }
        {
            let Some(journey) = journey_of(view) else {
                return;
            };
            let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            journey.table_log = ring.lines.iter().cloned().collect();
            journey.presentation_sequence = ring.version;
            journey.reply_speaker = ring.replying.first().cloned().unwrap_or_default();
        }
        // A view over the transport's message size would end the watch stream, so the oldest
        // lines are left out until the whole view fits.
        while view.encoded_len() > VIEW_BYTE_BUDGET {
            match journey_of(view) {
                Some(journey) if !journey.table_log.is_empty() => {
                    journey.table_log.remove(0);
                }
                _ => return,
            }
        }
    }
}

fn journey_of(view: &mut rpc::ViewMessage) -> Option<&mut rpc::JourneyView> {
    match view.audience.as_mut() {
        Some(rpc::view_message::Audience::Player(player)) => player.journey.as_mut(),
        Some(rpc::view_message::Audience::Display(display)) => display.journey.as_mut(),
        None => None,
    }
}

/// Keeps a speaker marked as replying; dropping it (reply written, failed or abandoned) clears
/// the mark.
pub(super) struct ReplyPending {
    log: Arc<TableLog>,
    speaker: String,
}

impl Drop for ReplyPending {
    fn drop(&mut self) {
        self.log.reply_done(&self.speaker);
    }
}

/// One public line an accepted action produces.
pub(super) struct ActionLine {
    pub(super) speaker: String,
    pub(super) text: String,
}

/// The public lines of an accepted action: the acting hero's short phrase from the offer's
/// label and visible result, then what the enemy did in answer. Creating a hero says nothing.
pub(super) fn action_lines(
    kind: rpc::GameplayActionKind,
    hero: &str,
    combat: &[rpc::CombatOutcome],
    weapon: &str,
    destination: &str,
    label: &str,
) -> Vec<ActionLine> {
    use rpc::GameplayActionKind as Kind;
    let own = combat.iter().find(|outcome| outcome.actor_name == hero);
    let phrase = match kind {
        Kind::CreateCharacter | Kind::Unspecified => return Vec::new(),
        Kind::GreatswordAttack => match own {
            Some(outcome) if outcome.hit => format!(
                "strikes with the {weapon} \u{2014} {}hit, {} damage",
                if outcome.critical { "critical " } else { "" },
                outcome.damage
            ),
            Some(_) => format!("swings the {weapon} \u{2014} miss"),
            None => format!("strikes with the {weapon}"),
        },
        Kind::SecondWind => "catches a second wind".to_owned(),
        Kind::EndTurn => "ends the turn".to_owned(),
        Kind::ShortRest => "takes a short rest".to_owned(),
        Kind::ChooseHarborScene => format!("leads on to {destination}"),
        _ => third_person(label),
    };
    let mut lines = vec![ActionLine {
        speaker: hero.to_owned(),
        text: phrase,
    }];
    for outcome in combat.iter().filter(|outcome| outcome.actor_name != hero) {
        let text = if outcome.hit {
            format!(
                "strikes {} \u{2014} hit, {} damage{}",
                outcome.target_name,
                outcome.damage,
                if outcome.knocked_out {
                    format!(", {} falls unconscious", outcome.target_name)
                } else {
                    String::new()
                }
            )
        } else {
            format!("misses {}", outcome.target_name)
        };
        lines.push(ActionLine {
            speaker: outcome.actor_name.clone(),
            text,
        });
    }
    lines
}

/// The third-person phrase for an offer label: its imperative verb takes an `s`
/// ("Ask Ilsa what is wrong" becomes "asks Ilsa what is wrong").
pub(super) fn third_person(label: &str) -> String {
    let label = label.trim();
    let (verb, rest) = label.split_once(' ').unwrap_or((label, ""));
    let verb = verb.to_lowercase();
    let conjugated =
        if verb.ends_with(['s', 'x', 'z', 'o']) || verb.ends_with("ch") || verb.ends_with("sh") {
            format!("{verb}es")
        } else {
            format!("{verb}s")
        };
    if rest.is_empty() {
        conjugated
    } else {
        format!("{conjugated} {rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech(log: &TableLog, text: &str) {
        log.push(
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            text,
        );
    }

    #[test]
    fn lines_keep_their_order_with_increasing_sequence_and_the_ring_is_bounded() {
        let log = TableLog::new();
        for number in 0..30 {
            speech(&log, &format!("line {number}"));
        }

        let lines = log.lines();

        assert_eq!(lines.len(), RING_LINES);
        assert_eq!(lines[0].text, "line 6");
        assert_eq!(lines[RING_LINES - 1].text, "line 29");
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert_eq!(lines[RING_LINES - 1].sequence, 30);
    }

    #[test]
    fn a_long_log_is_trimmed_from_the_oldest_so_the_view_fits_the_transport_message() {
        let log = TableLog::new();
        for number in 0..RING_LINES {
            speech(&log, &format!("{number} {}", "word ".repeat(80)));
        }
        let mut view = display_view();
        log.attach(&mut view);
        assert!(
            view.encoded_len() <= VIEW_BYTE_BUDGET,
            "{}",
            view.encoded_len()
        );
        let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
            panic!("a display view");
        };
        let kept = display.journey.unwrap().table_log;
        assert!(!kept.is_empty() && kept.len() < RING_LINES);
        assert!(
            kept.last().unwrap().text.starts_with("23 "),
            "the newest line stays"
        );
    }

    fn display_view() -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn attached(log: &TableLog) -> rpc::JourneyView {
        let mut view = display_view();
        log.attach(&mut view);
        match view.audience {
            Some(rpc::view_message::Audience::Display(display)) => display.journey.unwrap(),
            _ => unreachable!("the test builds a display view"),
        }
    }

    #[test]
    fn every_change_moves_the_presentation_sequence_and_wakes_watchers() {
        let log = TableLog::new();
        let wake = log.subscribe();
        let first = attached(&log).presentation_sequence;
        speech(&log, "Who sent you?");
        let typed = attached(&log).presentation_sequence;
        let pending = log.reply_pending("Courier");
        let thinking = attached(&log);
        drop(pending);
        let done = attached(&log);

        assert!(first < typed);
        assert!(typed < thinking.presentation_sequence);
        assert!(thinking.presentation_sequence < done.presentation_sequence);
        assert_eq!(thinking.reply_speaker, "Courier");
        assert_eq!(done.reply_speaker, "");
        assert!(wake.has_changed().unwrap());
    }

    #[test]
    fn a_reply_mark_clears_even_when_the_reply_never_comes() {
        let log = TableLog::new();
        {
            let _abandoned = log.reply_pending("Courier");
            assert_eq!(attached(&log).reply_speaker, "Courier");
        }
        assert_eq!(attached(&log).reply_speaker, "");
        let disabled = TableLog::disabled();
        drop(disabled.reply_pending("Courier"));
        assert_eq!(attached(&disabled).presentation_sequence, 0);
    }

    #[test]
    fn text_is_cleaned_cut_and_empty_lines_are_dropped() {
        let log = TableLog::new();

        speech(&log, "  hello\u{0}\nthere\t friend ");
        speech(&log, &"x".repeat(LINE_CHARACTERS + 40));
        speech(&log, " \n ");

        let lines = log.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello there friend");
        assert_eq!(lines[1].text.chars().count(), LINE_CHARACTERS);
    }

    #[test]
    fn a_new_line_wakes_a_watcher_and_a_disabled_log_stays_silent() {
        let log = TableLog::new();
        let mut watcher = log.subscribe();
        assert!(!watcher.has_changed().unwrap());

        speech(&log, "hello");

        assert!(watcher.has_changed().unwrap());
        assert_eq!(*watcher.borrow_and_update(), 1);
        let off = TableLog::disabled();
        speech(&off, "hello");
        assert!(off.lines().is_empty());
    }

    fn outcome(actor: &str, target: &str, hit: bool, damage: u32) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            hit,
            damage,
            ..Default::default()
        }
    }

    #[test]
    fn an_attack_is_a_phrase_with_its_visible_result_and_the_enemys_answer_follows() {
        let combat = [
            outcome("Brakka", "Snag", true, 9),
            outcome("Snag", "Brakka", false, 0),
        ];

        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &combat,
            "greatsword",
            "the shop",
            "Greatsword strike",
        );

        assert_eq!(lines[0].speaker, "Brakka");
        assert_eq!(
            lines[0].text,
            "strikes with the greatsword \u{2014} hit, 9 damage"
        );
        assert_eq!(lines[1].speaker, "Snag");
        assert_eq!(lines[1].text, "misses Brakka");
        let miss = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[outcome("Brakka", "Snag", false, 0)],
            "greatsword",
            "",
            "",
        );
        assert_eq!(miss[0].text, "swings the greatsword \u{2014} miss");
    }

    #[test]
    fn other_actions_use_fixed_or_label_phrases_and_creating_a_hero_is_silent() {
        let phrase = |kind, label: &str| {
            action_lines(
                kind,
                "Brakka",
                &[],
                "greatsword",
                "the Archivist's Study",
                label,
            )
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
        };
        use rpc::GameplayActionKind as Kind;
        assert_eq!(phrase(Kind::ShortRest, "x"), ["takes a short rest"]);
        assert_eq!(
            phrase(Kind::ChooseHarborScene, "x"),
            ["leads on to the Archivist's Study"]
        );
        assert_eq!(
            phrase(Kind::AskCourier, "Question Ilsa closely"),
            ["questions Ilsa closely"]
        );
        assert!(phrase(Kind::CreateCharacter, "Bring your hero to life").is_empty());
    }

    #[test]
    fn both_audiences_receive_the_feed_and_a_disabled_log_attaches_nothing() {
        let log = TableLog::new();
        speech(&log, "Hello there");
        let player = || rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        };
        let display = || rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        };
        let feed = |view: &rpc::ViewMessage| match &view.audience {
            Some(rpc::view_message::Audience::Player(p)) => {
                p.journey.as_ref().unwrap().table_log.len()
            }
            Some(rpc::view_message::Audience::Display(d)) => {
                d.journey.as_ref().unwrap().table_log.len()
            }
            None => 0,
        };

        let (mut for_player, mut for_display, mut untouched) = (player(), display(), player());
        log.attach(&mut for_player);
        log.attach(&mut for_display);
        TableLog::disabled().attach(&mut untouched);

        assert_eq!((feed(&for_player), feed(&for_display)), (1, 1));
        assert_eq!(feed(&untouched), 0);
    }

    #[test]
    fn labels_become_third_person_phrases() {
        assert_eq!(
            third_person("Ask Ilsa what is wrong"),
            "asks Ilsa what is wrong"
        );
        assert_eq!(
            third_person("Press on to the boathouse"),
            "presses on to the boathouse"
        );
        assert_eq!(
            third_person("Vow to see Ilsa through"),
            "vows to see Ilsa through"
        );
        assert_eq!(third_person("Go"), "goes");
    }
}
