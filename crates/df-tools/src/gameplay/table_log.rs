//! The public table log: the latest lines of who said and did what, shown to every screen.
//!
//! Everything here is table talk: an accepted action's public phrase, the words a hero types to
//! the scene's speaker, and the speaker's in-character reply. The private clue and any other
//! member-private text are never written here by any caller. The log is a bounded in-memory ring
//! per run; it is presentation only and never an authority for game state.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use tokio::sync::watch;

/// How many of the latest lines a view carries.
pub(super) const RING_LINES: usize = 24;
/// Longest text of one line, in characters.
pub(super) const LINE_CHARACTERS: usize = 420;
/// A speech-intent mark (typing or holding to talk) that is not refreshed this long is gone, so a
/// closed tab never leaves a stuck bubble on the table. The phone refreshes about every 2 s.
pub(super) const PRESENCE_EXPIRY: Duration = Duration::from_secs(5);
/// Hero seats a presence mark can name.
const PRESENCE_HEROES: u32 = 2;

/// One hero's live speech intent. Ephemeral: never stored in the game, never a revision.
struct Composing {
    hero_index: u32,
    kind: rpc::ComposeKind,
    deadline: Instant,
}

struct Ring {
    next_sequence: u64,
    lines: VecDeque<rpc::TableLine>,
    /// Counts every change a view shows (a line appended, a reply starting or ending), so a
    /// view whose game revision did not move can still be told from the one before it.
    version: u64,
    /// Counts speech-intent changes, which only the shared display shows: a phone's view (and
    /// so its mounted controls) is never rebuilt because another player is typing.
    presence_version: u64,
    /// The speakers whose reply to a hero's typed line is being written right now.
    replying: Vec<String>,
    /// Heroes typing or holding to talk right now, until stopped or expired.
    composing: Vec<Composing>,
    /// A task is waiting to expire the earliest mark.
    reaper_armed: bool,
}

pub(super) struct TableLog {
    /// A disabled log (qualification mode) records nothing and attaches nothing.
    enabled: bool,
    /// How long a speech-intent mark lives without a refresh.
    expiry: Duration,
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
            expiry: PRESENCE_EXPIRY,
            ring: Mutex::new(Ring {
                next_sequence: 1,
                lines: VecDeque::new(),
                version: 0,
                presence_version: 0,
                replying: Vec::new(),
                composing: Vec::new(),
                reaper_armed: false,
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
            // The hero's line replaces the hero's loader in the same view.
            if role == rpc::TableRole::Hero && kind == rpc::TableKind::Speech {
                ring.composing.retain(|mark| mark.hero_index != hero_index);
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

    /// Records what hero `hero_index` is doing in the dialogue box. Every refresh extends the
    /// mark's life and republishes the view, so a display that stops hearing it expires it too.
    /// Presentation only: nothing is committed and no game revision moves.
    pub(super) fn set_composing(self: &Arc<Self>, hero_index: u32, kind: rpc::ComposeKind) {
        let now = Instant::now();
        if let Some(deadline) = self.set_composing_at(hero_index, kind, now) {
            self.arm_reaper(deadline);
        }
    }

    /// The deadline of the earliest mark, when one is waiting to expire.
    fn set_composing_at(
        &self,
        hero_index: u32,
        kind: rpc::ComposeKind,
        now: Instant,
    ) -> Option<Instant> {
        if !self.enabled || hero_index >= PRESENCE_HEROES {
            return None;
        }
        let (changed, version, earliest) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let before = ring
                .composing
                .iter()
                .find(|mark| mark.hero_index == hero_index)
                .map(|mark| mark.kind);
            ring.composing.retain(|mark| mark.hero_index != hero_index);
            let shown = matches!(
                kind,
                rpc::ComposeKind::Typing | rpc::ComposeKind::Speaking | rpc::ComposeKind::Submitted
            );
            if shown {
                ring.composing.push(Composing {
                    hero_index,
                    kind,
                    deadline: now + self.expiry,
                });
            }
            // A refresh republishes too: it is the heartbeat the display expires the mark by.
            let changed = shown || before.is_some();
            if changed {
                ring.presence_version += 1;
            }
            let earliest = ring.composing.iter().map(|mark| mark.deadline).min();
            (changed, ring.version + ring.presence_version, earliest)
        };
        if changed {
            self.wake.send_replace(version);
        }
        earliest
    }

    /// Drops the marks whose time is up; republishes if any went. Returns the next deadline.
    fn expire_at(&self, now: Instant) -> Option<Instant> {
        let (removed, version, next) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let before = ring.composing.len();
            ring.composing.retain(|mark| mark.deadline > now);
            let removed = ring.composing.len() != before;
            if removed {
                ring.presence_version += 1;
            }
            let next = ring.composing.iter().map(|mark| mark.deadline).min();
            if next.is_none() {
                ring.reaper_armed = false;
            }
            (removed, ring.version + ring.presence_version, next)
        };
        if removed {
            self.wake.send_replace(version);
        }
        next
    }

    /// One task at a time waits for the earliest mark to expire, then for the next, so a closed
    /// tab clears from every screen without any other event.
    fn arm_reaper(self: &Arc<Self>, deadline: Instant) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if ring.reaper_armed {
                return;
            }
            ring.reaper_armed = true;
        }
        let log = self.clone();
        runtime.spawn(async move {
            let mut deadline = deadline;
            loop {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                match log.expire_at(Instant::now()) {
                    Some(next) => deadline = next,
                    None => break,
                }
            }
        });
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
        let display = matches!(view.audience, Some(rpc::view_message::Audience::Display(_)));
        let journey = match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Player(player)) => player.journey.as_mut(),
            Some(rpc::view_message::Audience::Display(display)) => display.journey.as_mut(),
            None => None,
        };
        if let Some(journey) = journey {
            let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            journey.table_log = ring.lines.iter().cloned().collect();
            journey.presentation_sequence = ring.version;
            journey.reply_speaker = ring.replying.first().cloned().unwrap_or_default();
            if !display {
                return;
            }
            journey.presentation_sequence += ring.presence_version;
            let now = Instant::now();
            journey.composing = ring
                .composing
                .iter()
                .filter(|mark| mark.deadline > now)
                .map(|mark| rpc::ComposingHero {
                    hero_index: mark.hero_index,
                    kind: mark.kind as i32,
                })
                .collect();
        }
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
    // The resolved attack names the weapon it used (a longbow for an archer); the hero's melee
    // weapon is only the fallback when the outcome carries none.
    let used = own
        .filter(|outcome| !outcome.weapon.is_empty())
        .map_or_else(
            || weapon.to_owned(),
            |outcome| outcome.weapon.replace('-', " "),
        );
    let phrase = match kind {
        Kind::CreateCharacter | Kind::Unspecified => return Vec::new(),
        Kind::GreatswordAttack => match own {
            Some(outcome) if outcome.hit => format!(
                "strikes with the {used} \u{2014} {}hit, {} damage",
                if outcome.critical { "critical " } else { "" },
                damage_words(outcome)
            ),
            Some(_) => format!("swings the {used} \u{2014} miss"),
            None => format!("strikes with the {used}"),
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
                damage_words(outcome),
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

/// The damage dealt with its type when the rules named one: "9 slashing".
fn damage_words(outcome: &rpc::CombatOutcome) -> String {
    if outcome.damage_type.is_empty() {
        outcome.damage.to_string()
    } else {
        format!("{} {}", outcome.damage, outcome.damage_type)
    }
}

/// The third-person phrase for an offer label: its imperative verb takes an `s`
/// ("Ask Ilsa what is wrong" becomes "asks Ilsa what is wrong").
pub(super) fn third_person(label: &str) -> String {
    let label = label.trim();
    let (verb, rest) = label.split_once(' ').unwrap_or((label, ""));
    let verb = verb.to_lowercase();
    let consonant_y = verb.len() > 1
        && verb.ends_with('y')
        && !verb[..verb.len() - 1].ends_with(['a', 'e', 'i', 'o', 'u']);
    let conjugated = if consonant_y {
        format!("{}ies", &verb[..verb.len() - 1])
    } else if verb.ends_with(['s', 'x', 'z', 'o']) || verb.ends_with("ch") || verb.ends_with("sh") {
        format!("{verb}es")
    } else {
        format!("{verb}s")
    };
    if rest.is_empty() {
        conjugated
    } else {
        format!("{conjugated} {}", third_person_words(rest))
    }
}

/// The second-person words of an imperative label, turned to the third person the log speaks in.
fn third_person_words(rest: &str) -> String {
    rest.split(' ')
        .map(|word| match word {
            "yourself" => "themselves",
            "your" => "their",
            "you" => "them",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" ")
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

    fn composing(log: &TableLog) -> Vec<(u32, i32)> {
        attached(log)
            .composing
            .iter()
            .map(|mark| (mark.hero_index, mark.kind))
            .collect()
    }

    #[test]
    fn a_hero_typing_shows_for_everyone_and_stopping_clears_it() {
        let log = TableLog::new();
        let wake = log.subscribe();
        let before = attached(&log).presentation_sequence;
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        assert_eq!(composing(&log), [(0, rpc::ComposeKind::Typing as i32)]);
        assert!(attached(&log).presentation_sequence > before);
        assert!(wake.has_changed().unwrap());
        log.set_composing_at(0, rpc::ComposeKind::Stopped, now);
        assert!(composing(&log).is_empty());
    }

    #[test]
    fn two_heroes_composing_at_once_each_have_their_own_mark() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        log.set_composing_at(1, rpc::ComposeKind::Speaking, now);
        let marks = composing(&log);
        assert!(marks.contains(&(0, rpc::ComposeKind::Typing as i32)));
        assert!(marks.contains(&(1, rpc::ComposeKind::Speaking as i32)));
        log.set_composing_at(0, rpc::ComposeKind::Stopped, now);
        assert_eq!(composing(&log), [(1, rpc::ComposeKind::Speaking as i32)]);
    }

    #[test]
    fn a_refresh_republishes_as_the_displays_heartbeat_and_a_stop_of_nothing_does_not() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        let first = attached(&log).presentation_sequence;
        let mut wake = log.subscribe();
        wake.borrow_and_update();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2));
        assert!(attached(&log).presentation_sequence > first);
        assert!(wake.has_changed().unwrap());
        log.set_composing_at(1, rpc::ComposeKind::Stopped, now);
        let settled = attached(&log).presentation_sequence;
        log.set_composing_at(1, rpc::ComposeKind::Stopped, now);
        assert_eq!(attached(&log).presentation_sequence, settled);
    }

    #[test]
    fn a_refresh_keeps_the_mark_alive_and_a_missing_one_expires_it() {
        let log = TableLog::new();
        let now = Instant::now();
        let deadline = log
            .set_composing_at(0, rpc::ComposeKind::Typing, now)
            .expect("a deadline");
        assert_eq!(deadline, now + PRESENCE_EXPIRY);
        // Refreshed at 2 s: alive past the first deadline.
        let refreshed = log
            .set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2))
            .expect("a deadline");
        assert_eq!(log.expire_at(deadline), Some(refreshed));
        assert_eq!(composing(&log).len(), 1);
        // Nothing more arrives: the next expiry clears it and republishes.
        let before = attached(&log).presentation_sequence;
        assert_eq!(log.expire_at(refreshed), None);
        assert!(composing(&log).is_empty());
        assert!(attached(&log).presentation_sequence > before);
    }

    #[test]
    fn unknown_seats_and_a_disabled_log_record_no_presence() {
        let log = TableLog::new();
        let now = Instant::now();
        assert_eq!(log.set_composing_at(2, rpc::ComposeKind::Typing, now), None);
        assert!(composing(&log).is_empty());
        let disabled = TableLog::disabled();
        disabled.set_composing_at(0, rpc::ComposeKind::Typing, now);
        assert!(composing(&disabled).is_empty());
        assert_eq!(attached(&disabled).presentation_sequence, 0);
    }

    #[test]
    fn a_hero_line_clears_that_heros_mark_in_the_same_view() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Submitted, now);
        log.set_composing_at(1, rpc::ComposeKind::Typing, now);
        speech(&log, "Who sent you?");
        let view = attached(&log);
        assert_eq!(view.table_log.len(), 1);
        assert_eq!(composing(&log), [(1, rpc::ComposeKind::Typing as i32)]);
    }

    #[test]
    fn a_phones_view_never_carries_or_moves_with_another_players_presence() {
        let log = TableLog::new();
        let now = Instant::now();
        let player = |log: &TableLog| {
            let mut view = rpc::ViewMessage {
                audience: Some(rpc::view_message::Audience::Player(
                    rpc::PlayerGameplayView {
                        journey: Some(rpc::JourneyView::default()),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            };
            log.attach(&mut view);
            match view.audience {
                Some(rpc::view_message::Audience::Player(player)) => player.journey.unwrap(),
                _ => unreachable!("the test builds a player view"),
            }
        };
        let before = player(&log).presentation_sequence;
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        log.set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2));
        let after = player(&log);
        assert_eq!(after.presentation_sequence, before);
        assert!(after.composing.is_empty());
    }

    #[test]
    fn presence_never_adds_a_table_line_or_a_game_revision() {
        let log = TableLog::new();
        log.set_composing_at(0, rpc::ComposeKind::Typing, Instant::now());
        assert!(log.lines().is_empty());
        let journey = attached(&log);
        assert!(journey.table_log.is_empty());
    }

    #[tokio::test]
    async fn the_reaper_clears_a_mark_nobody_refreshes_and_wakes_the_watchers() {
        let mut log = TableLog::with(true);
        log.expiry = Duration::from_millis(60);
        let log = Arc::new(log);
        let mut wake = log.subscribe();
        log.set_composing(0, rpc::ComposeKind::Typing);
        wake.changed().await.unwrap();
        assert_eq!(composing(&log).len(), 1);
        tokio::time::timeout(Duration::from_secs(2), wake.changed())
            .await
            .expect("woken by the expiry")
            .unwrap();
        assert!(composing(&log).is_empty());
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
    fn a_hit_names_the_damage_type_the_rules_gave_it() {
        let own = rpc::CombatOutcome {
            damage_type: "slashing".to_owned(),
            critical: true,
            ..outcome("Brakka", "Snag", true, 9)
        };
        let answer = rpc::CombatOutcome {
            damage_type: "piercing".to_owned(),
            ..outcome("Snag", "Brakka", true, 4)
        };

        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[own, answer],
            "greatsword",
            "",
            "",
        );

        assert_eq!(
            lines[0].text,
            "strikes with the greatsword \u{2014} critical hit, 9 slashing damage"
        );
        assert_eq!(
            lines[1].text,
            "strikes Brakka \u{2014} hit, 4 piercing damage"
        );
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

    /// Every label the tables can produce, read from the same files the server loads.
    fn table_labels() -> Vec<String> {
        let scripts: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../content/adventure-tables/beat-scripts.json"
        ))
        .unwrap();
        let types: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../content/adventure-tables/chapter-types.json"
        ))
        .unwrap();
        let mut labels = Vec::new();
        for template in scripts["chapter_types"].as_object().unwrap().values() {
            // `labels` is optional in a template; a template without them has none to check.
            for label in template["script"]["labels"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
            {
                labels.push(label.as_str().unwrap().to_owned());
            }
        }
        fn choices(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, item) in map {
                        if matches!(key.as_str(), "asks" | "helps" | "stands") {
                            if let Some(list) = item.as_array() {
                                out.extend(
                                    list.iter().filter_map(|l| l.as_str().map(str::to_owned)),
                                );
                            }
                        }
                        choices(item, out);
                    }
                }
                serde_json::Value::Array(list) => list.iter().for_each(|item| choices(item, out)),
                _ => {}
            }
        }
        choices(&types, &mut labels);
        labels
    }

    #[test]
    fn every_table_label_conjugates_to_its_correct_third_person_form() {
        let labels = table_labels();
        assert!(labels.len() > 50, "the tables supply the labels under test");
        for label in &labels {
            let verb = label.split(' ').next().unwrap().to_lowercase();
            let third = third_person(label);
            let form = third.split(' ').next().unwrap();
            let expected = if verb.ends_with("y")
                && !verb[..verb.len() - 1].ends_with(['a', 'e', 'i', 'o', 'u'])
            {
                format!("{}ies", &verb[..verb.len() - 1])
            } else if verb.ends_with(['s', 'x', 'z', 'o'])
                || verb.ends_with("ch")
                || verb.ends_with("sh")
            {
                format!("{verb}es")
            } else {
                format!("{verb}s")
            };
            assert_eq!(form, expected, "label {label:?}");
            let words: Vec<&str> = third.split(' ').collect();
            assert!(
                !words
                    .iter()
                    .any(|word| matches!(*word, "your" | "you" | "yourself")),
                "second person left in {third:?}"
            );
        }
        assert_eq!(
            third_person("Carry the clue onward"),
            "carries the clue onward"
        );
        assert_eq!(third_person("Study the markings"), "studies the markings");
        assert_eq!(third_person("Buy the ticket"), "buys the ticket");
        assert_eq!(third_person("Rally the crowd"), "rallies the crowd");
        assert_eq!(
            third_person("Pledge your help to Kestrel"),
            "pledges their help to Kestrel"
        );
        assert_eq!(third_person("Trust you"), "trusts them");
    }
}
