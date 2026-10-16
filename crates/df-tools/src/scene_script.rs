//! Selects the server's current scene lines for the display's presentation queue.

use crate::{
    presentation::{TableKind, TableLine, TableRole},
    voice_line::CheckVoiceLine,
};

#[derive(Default)]
pub(crate) struct SceneScript<'a> {
    pub(crate) narration: &'a str,
    pub(crate) previous_narration: &'a str,
    pub(crate) lines: &'a [TableLine],
    pub(crate) fresh: &'a [&'a TableLine],
    pub(crate) check: Option<&'a CheckVoiceLine>,
    pub(crate) restart: bool,
    /// The server has accepted a player's speech and is still composing its answer. A reconnect
    /// during that turn must not revive the authored opening line or the previous narrator cue.
    pub(crate) awaiting_reply: bool,
    pub(crate) scene_changed: bool,
    pub(crate) in_fight: bool,
}

pub(crate) enum SceneScriptLine<'a> {
    Narration(&'a str),
    Table(&'a TableLine),
    CheckRoll,
    CheckResult,
}

impl<'a> SceneScript<'a> {
    pub(crate) fn lines(&self) -> Vec<SceneScriptLine<'a>> {
        let mut script = Vec::new();
        let scripted = self
            .fresh
            .iter()
            .rev()
            .find(|line| line.scripted && line.kind == TableKind::Speech)
            .copied();
        let fresh_check = self.check.is_some_and(|check| {
            self.fresh
                .iter()
                .any(|line| line.sequence == check.result_sequence)
        });
        if self.restart {
            // A reconnect leaves the latest answer or committed outcome visible, silently.
            let current = self.lines.last().filter(|line| {
                ((line.kind == TableKind::Speech && line.role == TableRole::Npc)
                    || public_outcome(line, self.check))
                    && self
                        .check
                        .is_none_or(|check| line.sequence > check.result_sequence)
            });
            if let Some(line) = current {
                script.push(SceneScriptLine::Table(line));
            } else if self.awaiting_reply {
                // The player's question is the current beat; keep the stage quiet until its reply
                // is committed instead of replaying a previous caption or scene intro.
            } else if self.check.is_some() {
                script.push(SceneScriptLine::CheckRoll);
                script.push(SceneScriptLine::CheckResult);
            } else if let Some(line) = scripted.filter(|line| line.role == TableRole::Npc) {
                script.push(SceneScriptLine::Table(line));
            } else if !self.narration.trim().is_empty() {
                script.push(SceneScriptLine::Narration(self.narration));
            }
        } else if self.scene_changed {
            if !fresh_check && !self.narration.trim().is_empty() {
                script.push(SceneScriptLine::Narration(self.narration));
            }
            if fresh_check {
                script.push(SceneScriptLine::CheckRoll);
                script.push(SceneScriptLine::CheckResult);
            }
            if let Some(line) = scripted {
                script.push(SceneScriptLine::Table(line));
            }
        } else {
            if !fresh_check
                && !self.in_fight
                && let Some(added) = added_narration(self.previous_narration, self.narration)
            {
                script.push(SceneScriptLine::Narration(added));
            }
            if fresh_check {
                script.push(SceneScriptLine::CheckRoll);
                script.push(SceneScriptLine::CheckResult);
            }
            script.extend(
                self.fresh
                    .iter()
                    .filter(|line| match line.kind {
                        TableKind::Speech => {
                            matches!(line.role, TableRole::Hero(_))
                                || (line.role == TableRole::Npc && !line.scripted)
                        }
                        TableKind::Action => {
                            !self.in_fight
                                && (line.role != TableRole::Narrator
                                    || public_outcome(line, self.check))
                        }
                    })
                    .map(|line| SceneScriptLine::Table(line)),
            );
        }
        script
    }
}

fn public_outcome(line: &TableLine, check: Option<&CheckVoiceLine>) -> bool {
    line.role == TableRole::Narrator
        && line.kind == TableKind::Action
        && line.speaker.trim() != "Roll"
        && check.is_none_or(|check| line.sequence != check.result_sequence)
}

/// Returning to the scene's shorter base text does not add a new line.
fn added_narration<'a>(told: &str, now: &'a str) -> Option<&'a str> {
    let (told, now) = (told.trim(), now.trim());
    if now.is_empty() || told.starts_with(now) {
        return None;
    }
    Some(now.strip_prefix(told).map(str::trim).unwrap_or(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_conductor::{ClipOp, Command, Conductor, LineTiming, ScriptLine, ViewPlan};
    use crate::stage_machines::reading_time_ms;
    use crate::voice_line::{NARRATOR_ROLE, VoiceLine, display_lines};
    use df_protocol::common as rpc;

    const OPENING: &str = "You meet Marek at the counter of his cloth shop.";
    const FINDING: &str = "The brass balance has a worn maker's mark beneath its pan.";
    const PICKUP: &str = "Sigrun picks up folded paper label.";

    fn outcome(sequence: u64, text: &str) -> TableLine {
        TableLine {
            sequence,
            actor_id: String::new(),
            speaker: "Narrator".into(),
            role: TableRole::Narrator,
            kind: TableKind::Action,
            text: text.into(),
            scripted: false,
        }
    }

    fn speech(sequence: u64, role: TableRole, speaker: &str, text: &str) -> TableLine {
        TableLine {
            sequence,
            actor_id: String::new(),
            speaker: speaker.to_owned(),
            role,
            kind: TableKind::Speech,
            text: text.to_owned(),
            scripted: false,
        }
    }

    fn text<'a>(script: &SceneScript<'a>) -> Vec<&'a str> {
        script
            .lines()
            .into_iter()
            .map(|line| match line {
                SceneScriptLine::Narration(text) => text,
                SceneScriptLine::Table(line) => line.text.as_str(),
                SceneScriptLine::CheckRoll => "[roll]",
                SceneScriptLine::CheckResult => script.check.unwrap().voice.text.as_str(),
            })
            .collect()
    }

    fn check(sequence: u64) -> CheckVoiceLine {
        CheckVoiceLine {
            sequence,
            result_sequence: sequence + 1,
            finding: FINDING.into(),
            voice: VoiceLine {
                role: NARRATOR_ROLE.into(),
                text: format!("Success. {FINDING}"),
            },
        }
    }

    #[test]
    fn investigation_then_pickup_queues_the_outcome_without_replaying_the_scene_intro() {
        let entering = SceneScript {
            narration: OPENING,
            scene_changed: true,
            ..Default::default()
        };
        assert_eq!(text(&entering), [OPENING]);

        let check = check(10);
        let mut roll = outcome(10, "Brakka studies the brass balance: 10 + 0 = 10.");
        roll.speaker = "Roll".into();
        let finding = outcome(11, FINDING);
        let checked_lines = [roll, finding];
        let checked_fresh = checked_lines.iter().collect::<Vec<_>>();
        let expanded = format!("{OPENING} {FINDING}");
        let investigated = SceneScript {
            narration: OPENING,
            previous_narration: OPENING,
            lines: &checked_lines,
            fresh: &checked_fresh,
            check: Some(&check),
            ..Default::default()
        };
        assert_eq!(text(&investigated), ["[roll]", &check.voice.text]);

        let pickup = outcome(12, PICKUP);
        let picked_up = SceneScript {
            narration: OPENING,
            previous_narration: OPENING,
            lines: std::slice::from_ref(&pickup),
            fresh: &[&pickup],
            ..Default::default()
        };
        assert_eq!(text(&picked_up), [PICKUP]);
        assert_eq!(
            text(&SceneScript {
                previous_narration: &expanded,
                ..picked_up
            }),
            [PICKUP]
        );
        let refreshed = SceneScript {
            previous_narration: OPENING,
            fresh: &[],
            ..picked_up
        };
        assert!(text(&refreshed).is_empty());

        let displayed = rpc::DisplayGameplayView {
            narration: OPENING.into(),
            journey: Some(rpc::JourneyView {
                table_log: vec![rpc::TableLine {
                    sequence: pickup.sequence,
                    role: rpc::TableRole::Narrator as i32,
                    kind: rpc::TableKind::Action as i32,
                    speaker: pickup.speaker.clone(),
                    text: pickup.text.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let voices = display_lines(&displayed);
        let mut conductor = Conductor::new();
        conductor.set_sound(true);
        let script = SceneScript {
            fresh: &[&pickup],
            ..refreshed
        };
        let commands = conductor.restart(
            0,
            ViewPlan {
                diff: Default::default(),
                script: text(&script)
                    .into_iter()
                    .map(|text| ScriptLine {
                        detail: text.to_owned(),
                        voiceable: voices.iter().any(|voice| voice.text == text),
                        read_ms: reading_time_ms(text),
                        slot: None,
                        settled: false,
                        timing: LineTiming::Ordinary,
                    })
                    .collect(),
                cast: vec![],
            },
        );
        assert!(commands.iter().any(|command| {
            matches!(command, Command::Clip { op: ClipOp::Request, detail, .. } if detail == PICKUP)
        }));
        assert!(!commands.iter().any(|command| {
            matches!(command, Command::Line { detail, .. } | Command::Clip { detail, .. } if detail == OPENING)
        }));
    }

    #[test]
    fn new_clues_and_repeated_check_identities_each_get_one_result() {
        let expanded = format!("{OPENING} {FINDING}");
        let clue = SceneScript {
            narration: &expanded,
            previous_narration: OPENING,
            ..Default::default()
        };
        assert_eq!(text(&clue), [FINDING]);
        assert!(
            text(&SceneScript {
                previous_narration: &expanded,
                ..clue
            })
            .is_empty()
        );
        for sequence in [20, 30] {
            let check = check(sequence);
            let finding = outcome(sequence + 1, FINDING);
            let script = SceneScript {
                narration: OPENING,
                previous_narration: OPENING,
                fresh: &[&finding],
                check: Some(&check),
                ..Default::default()
            };
            assert_eq!(text(&script), ["[roll]", &check.voice.text]);
            assert!(
                text(&SceneScript {
                    fresh: &[],
                    ..script
                })
                .is_empty()
            );
        }
    }

    #[test]
    fn reconnect_keeps_current_outcome_or_check_and_new_scene_still_enters() {
        let pickup = outcome(12, PICKUP);
        let script = SceneScript {
            narration: OPENING,
            lines: std::slice::from_ref(&pickup),
            restart: true,
            ..Default::default()
        };
        assert_eq!(text(&script), [PICKUP]);
        let check = check(20);
        assert_eq!(
            text(&SceneScript {
                check: Some(&check),
                ..script
            }),
            ["[roll]", &check.voice.text]
        );
        let mut reply = outcome(22, "Marek says the label belongs with those parcels.");
        reply.role = TableRole::Npc;
        reply.kind = TableKind::Speech;
        assert_eq!(
            text(&SceneScript {
                lines: std::slice::from_ref(&reply),
                check: Some(&check),
                ..script
            }),
            [reply.text.as_str()]
        );
        assert_eq!(
            text(&SceneScript {
                narration: "The next shop opens onto a sunny square.",
                previous_narration: OPENING,
                scene_changed: true,
                ..Default::default()
            }),
            ["The next shop opens onto a sunny square."]
        );
    }

    #[test]
    fn purchase_reply_then_new_question_waits_for_the_new_answer_without_replaying_old_audio() {
        let purchase = outcome(1, "Marek hands over the fine clothes for 200 copper.");
        let buy_reply = speech(
            2,
            TableRole::Npc,
            "Marek",
            "The tailor folds the purchased clothes into paper.",
        );
        let question = speech(
            3,
            TableRole::Hero(0),
            "Brakka",
            "Could I inspect the other fine clothes?",
        );
        let waiting_lines = [purchase.clone(), buy_reply.clone(), question.clone()];
        let waiting = SceneScript {
            narration: OPENING,
            lines: &waiting_lines,
            restart: true,
            awaiting_reply: true,
            ..Default::default()
        };
        assert!(
            text(&waiting).is_empty(),
            "a reconnect during a current question must not replay the old purchase line or intro"
        );

        let answer = speech(
            4,
            TableRole::Npc,
            "Marek",
            "Marek shows Brakka the remaining wool coats.",
        );
        let answered_lines = [purchase, buy_reply.clone(), question, answer.clone()];
        let answered = SceneScript {
            narration: OPENING,
            lines: &answered_lines,
            fresh: &[&answer],
            ..Default::default()
        };
        assert_eq!(text(&answered), [answer.text.as_str()]);
        let reply_read_ms = reading_time_ms(&answer.text);
        assert!(reply_read_ms >= crate::stage_machines::READ_MINIMUM_MS);

        let mut conductor = Conductor::new();
        conductor.set_sound(true);
        let commands = conductor.restart(
            0,
            ViewPlan {
                diff: Default::default(),
                script: text(&answered)
                    .into_iter()
                    .map(|text| ScriptLine {
                        detail: text.to_owned(),
                        voiceable: true,
                        read_ms: reading_time_ms(text),
                        slot: None,
                        settled: false,
                        timing: LineTiming::Ordinary,
                    })
                    .collect(),
                cast: vec![],
            },
        );
        assert!(commands.iter().any(|command| {
            matches!(command, Command::Clip { op: ClipOp::Request, detail, .. } if detail == &answer.text)
        }));
        assert!(!commands.iter().any(|command| {
            matches!(command, Command::Clip { op: ClipOp::Request, detail, .. } if detail == &buy_reply.text)
        }));
    }
}
