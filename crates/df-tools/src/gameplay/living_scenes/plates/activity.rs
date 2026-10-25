//! Public table activity supplies poses; canonical custody still owns every prop.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct HeroActivity {
    pub(super) seat: u8,
    pub(super) sequence: u64,
    pub(super) text: String,
}

#[derive(Default)]
pub(super) struct Book {
    scene: Option<String>,
    floor: u64,
    seen: u64,
    latest: BTreeMap<u8, HeroActivity>,
    outcomes: BTreeMap<u64, String>,
}

impl Book {
    pub(super) fn scope(&mut self, view: &mut View) {
        if self.scene.as_deref() != Some(&view.scene_key) {
            // On first attachment the log has no scene IDs. Ignore unscoped history;
            // thereafter use the preceding visit's exact public high-water mark.
            self.floor = if self.scene.is_some() {
                self.seen
            } else {
                view.latest_sequence
            };
            self.scene = Some(view.scene_key.clone());
            self.latest.clear();
            self.outcomes.clear();
        }
        if view.latest_sequence < self.seen {
            // Undo can rewind the public log without moving to another scene.
            self.latest
                .retain(|_, activity| activity.sequence <= view.latest_sequence);
            self.outcomes
                .retain(|sequence, _| *sequence <= view.latest_sequence);
            self.floor = self.floor.min(view.latest_sequence);
        }
        self.seen = view.latest_sequence;
        for activity in view
            .hero_activity
            .iter()
            .filter(|activity| activity.sequence > self.floor)
        {
            if self
                .latest
                .get(&activity.seat)
                .is_none_or(|known| known.sequence < activity.sequence)
            {
                self.latest.insert(activity.seat, activity.clone());
            }
        }
        for (sequence, text) in &view.scene_outcomes {
            if *sequence > self.floor {
                self.outcomes.insert(*sequence, text.clone());
            }
        }
        while self.outcomes.len() > 6 {
            self.outcomes.pop_first();
        }
        view.scene_outcomes = self
            .outcomes
            .iter()
            .map(|(sequence, text)| (*sequence, text.clone()))
            .collect();
        // At most six owned rows survive the presentation ring scrolling past them.
        view.hero_activity = self.latest.values().cloned().collect();
    }
}

pub(super) fn latest(journey: &rpc::JourneyView) -> Vec<HeroActivity> {
    let mut seats = std::collections::BTreeSet::new();
    journey
        .table_log
        .iter()
        .rev()
        .filter_map(|line| {
            if rpc::TableRole::try_from(line.role).ok()? != rpc::TableRole::Hero
                || !matches!(
                    rpc::TableKind::try_from(line.kind).ok()?,
                    rpc::TableKind::Action | rpc::TableKind::Speech
                )
                || line.text.trim().is_empty()
            {
                return None;
            }
            let seat = u8::try_from(line.hero_index.checked_add(1)?).ok()?;
            if !(1..=6).contains(&seat) || !seats.insert(seat) {
                return None;
            }
            Some(HeroActivity {
                seat,
                sequence: line.sequence,
                text: line.text.trim().chars().take(240).collect(),
            })
        })
        .collect()
}

/// Scene-interaction buttons can emit a narrator outcome without a hero speech row.
/// Give the image model these public facts to associate with named heroes; Rust never
/// classifies player prose or guesses an actor from the narrator's default hero_index.
pub(super) fn outcomes(journey: &rpc::JourneyView) -> Vec<(u64, String)> {
    journey
        .table_log
        .iter()
        .rev()
        .filter(|line| {
            line.role == rpc::TableRole::Narrator as i32
                && line.kind == rpc::TableKind::Action as i32
                && line.roll.is_none()
                && !line.text.trim().is_empty()
        })
        .take(6)
        .map(|line| (line.sequence, line.text.trim().chars().take(240).collect()))
        .collect()
}

impl PlateScene {
    pub(super) fn with_activity(
        &self,
        prompt: crate::gameplay::imagery::AssembledPrompt,
    ) -> Result<crate::gameplay::imagery::AssembledPrompt, ImageFailure> {
        prompt
            .with_hero_activity(&self.hero_names, &self.hero_activity, &self.scene_outcomes)
            .map_err(|_| ImageFailure::InvalidRequest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::living_scenes::plates::tests::{scene, view};

    fn line(sequence: u64, hero_index: u32, text: &str) -> rpc::TableLine {
        rpc::TableLine {
            audio_url: String::new(),
            audio_part_urls: Vec::new(),
            sequence,
            hero_index,
            text: text.into(),
            role: rpc::TableRole::Hero as i32,
            kind: rpc::TableKind::Speech as i32,
            ..Default::default()
        }
    }

    #[test]
    fn each_hero_keeps_only_their_latest_public_activity_without_interpreting_player_words() {
        let journey = rpc::JourneyView {
            table_log: vec![
                line(1, 0, "old action"),
                line(2, 1, "studying the scale"),
                line(3, 0, "flirting with Tobin"),
                rpc::TableLine {
                    audio_url: String::new(),
                    audio_part_urls: Vec::new(),
                    role: rpc::TableRole::Npc as i32,
                    ..line(4, 1, "NPC reply")
                },
            ],
            ..Default::default()
        };
        let activity = latest(&journey);
        assert_eq!(activity.len(), 2);
        assert_eq!(
            (activity[0].seat, activity[0].text.as_str()),
            (1, "flirting with Tobin")
        );
        assert_eq!(
            (activity[1].seat, activity[1].text.as_str()),
            (2, "studying the scale")
        );
    }

    #[test]
    fn public_scene_outcomes_are_bounded_pose_evidence_without_guessing_a_hero_from_prose() {
        let mut journey = rpc::JourneyView {
            table_log: (1..=8)
                .map(|sequence| rpc::TableLine {
                    audio_url: String::new(),
                    audio_part_urls: Vec::new(),
                    sequence,
                    role: rpc::TableRole::Narrator as i32,
                    kind: rpc::TableKind::Action as i32,
                    text: format!("Orla counts coins at Mara's counter, public outcome {sequence}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        journey.table_log.push(rpc::TableLine {
            audio_url: String::new(),
            audio_part_urls: Vec::new(),
            sequence: 9,
            roll: Some(rpc::TableRoll::default()),
            role: rpc::TableRole::Narrator as i32,
            kind: rpc::TableKind::Action as i32,
            text: "roll details".into(),
            ..Default::default()
        });
        assert!(
            latest(&journey).is_empty(),
            "narrator index zero is not hero one"
        );
        let rows = outcomes(&journey);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].0, 8);
        assert!(rows[0].1.contains("Orla counts coins at Mara's counter"));
        assert!(!rows.iter().any(|(_, text)| text.contains("roll details")));
    }

    #[test]
    fn scene_visit_scope_drops_old_actions_on_arrival_and_revisit_but_survives_repeated_views() {
        let scene = scene(SceneKey::Opening, "Market", None);
        let mut view = view(&scene, &[1], false);
        let mut book = Book::default();
        // No scene IDs exist on RPC table rows; an initial attachment cannot claim old history.
        view.latest_sequence = 5;
        view.hero_activity = vec![HeroActivity {
            seat: 1,
            sequence: 4,
            text: "old place".into(),
        }];
        book.scope(&mut view);
        assert!(view.hero_activity.is_empty());
        view.latest_sequence = 6;
        view.hero_activity = vec![HeroActivity {
            seat: 1,
            sequence: 6,
            text: "at Mara's counter".into(),
        }];
        book.scope(&mut view);
        book.scope(&mut view);
        assert_eq!(view.hero_activity.len(), 1);
        view.hero_activity.clear();
        view.latest_sequence = 100;
        book.scope(&mut view);
        assert_eq!(view.hero_activity[0].text, "at Mara's counter");
        view.scene_key = "other place".into();
        book.scope(&mut view);
        assert!(view.hero_activity.is_empty());
        view.scene_key = "Market".into();
        view.hero_activity = vec![HeroActivity {
            seat: 1,
            sequence: 6,
            text: "stale market visit".into(),
        }];
        book.scope(&mut view);
        assert!(view.hero_activity.is_empty());
    }

    #[test]
    fn rewinding_public_history_discards_later_poses() {
        let scene = scene(SceneKey::Opening, "Market", None);
        let mut view = view(&scene, &[1], false);
        let mut book = Book::default();
        book.scope(&mut view);
        view.latest_sequence = 6;
        view.hero_activity = vec![HeroActivity {
            seat: 1,
            sequence: 6,
            text: "future gesture".into(),
        }];
        book.scope(&mut view);
        assert_eq!(view.hero_activity.len(), 1);
        view.latest_sequence = 5;
        view.hero_activity.clear();
        book.scope(&mut view);
        assert!(view.hero_activity.is_empty());
    }
}
