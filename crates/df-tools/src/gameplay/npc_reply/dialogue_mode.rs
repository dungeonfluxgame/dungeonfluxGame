//! The kind of conversation a hero's line opens with an NPC, read from Jev's own judgements.
//!
//! The mode is not a second reading of the words. It is composed from the judgements the reply
//! path already makes under closed choices: the social approach (`approach_model`), the focus of
//! each question part (`npc_facts`) and the trade state the merchant judgement selected for this
//! turn. No word of the line is read here. The mode decides what the reply is allowed to steer
//! toward: a merchant's trade allowances reach the reply only when the line is about trade, so a
//! flirt, a threat or a plea is answered as itself and never with a sales pitch.
//!
//! A plea shares the persuasion mode: both ask the NPC to act on a reason or an appeal, both are
//! the 2024 Persuasion skill, and the NPC's own warmth decides how either lands.

use serde_json::{Value, json};

use super::super::{npc_facts::AskReading, social::Approach};

/// One hero line's conversation mode, from the strongest social signal down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogueMode {
    SmallTalk,
    Question,
    Shopping,
    Negotiation,
    Persuasion,
    Threat,
    Deception,
    Flattery,
    Romance,
    Insult,
    /// Jev read neither the approach nor the line's parts, so nothing narrows the reply.
    Unjudged,
}

/// The trade state the merchant judgement selected for this turn, before the reply is written.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TradeTurn {
    /// The caller matched canonical trade terms to this hero and this merchant.
    pub(super) listed_trade: bool,
    /// The line names a figure against a listed price, a buyback figure, or answers a live quote.
    pub(super) bargaining: bool,
}

impl DialogueMode {
    /// Composes the mode from Jev's approach, Jev's focus reading and the judged trade state. A
    /// threat, an insult and a flirt decide the mode whatever else the line does; a live bargain
    /// comes next, then the other persuasive approaches, then plain trade and questions.
    pub(super) fn read(approach: Approach, ask: &AskReading, trade: TradeTurn) -> Self {
        match approach {
            Approach::Intimidation => return Self::Threat,
            Approach::Insult => return Self::Insult,
            Approach::Flirting => return Self::Romance,
            _ => {}
        }
        if trade.bargaining
            || approach == Approach::Bribe
            || (approach == Approach::Bargain && !trade.listed_trade)
        {
            return Self::Negotiation;
        }
        match approach {
            Approach::Deception => return Self::Deception,
            Approach::Persuasion => return Self::Persuasion,
            Approach::Flattery => return Self::Flattery,
            _ => {}
        }
        if trade.listed_trade || approach == Approach::Bargain {
            Self::Shopping
        } else if approach == Approach::Question || ask.asks_something() {
            Self::Question
        } else if approach == Approach::Other && ask.focus.is_none() {
            Self::Unjudged
        } else {
            Self::SmallTalk
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::SmallTalk => "small_talk",
            Self::Question => "question",
            Self::Shopping => "shopping",
            Self::Negotiation => "negotiation",
            Self::Persuasion => "persuasion",
            Self::Threat => "threat",
            Self::Deception => "deception",
            Self::Flattery => "flattery",
            Self::Romance => "romance",
            Self::Insult => "insult",
            Self::Unjudged => "unjudged",
        }
    }

    /// Whether the reply may draw on the speaker's trade allowances (quoting listed prices, fees
    /// for unlisted work, deciding a purchase). A question may be about a service; an unjudged
    /// line keeps every allowance it had before modes existed.
    pub(super) fn allows_trade(self) -> bool {
        matches!(
            self,
            Self::Shopping | Self::Negotiation | Self::Question | Self::Unjudged
        )
    }

    /// Whether the line may urge a person to do, give, allow or reveal something, so a person
    /// without a dossier has their willingness judged before the reply (SRD 5.2.1 Influence).
    pub(super) fn may_urge(self) -> bool {
        matches!(
            self,
            Self::Negotiation
                | Self::Persuasion
                | Self::Threat
                | Self::Deception
                | Self::Flattery
                | Self::Romance
        )
    }

    /// How the reply takes up this mode. The NPC's own persona, feelings and the server's check
    /// facts decide the reaction; this only says what kind of moment it is and what it may not do.
    fn meaning(self) -> &'static str {
        match self {
            Self::SmallTalk => {
                "Casual talk. Answer in your own voice and opinions. Do not steer toward trade, prices or an errand unless the hero brings one up."
            }
            Self::Question => {
                "The hero asks for something to be explained or told. Answer what you know concretely, within your role."
            }
            Self::Shopping => {
                "The hero is browsing, asking a price or buying. Talk about the goods and the listed terms only."
            }
            Self::Negotiation => {
                "The hero is bargaining or offering a deal. Hold to the server's terms and checks; counter, accept or refuse in character for a reason of your own."
            }
            Self::Persuasion => {
                "The hero makes a case, an appeal or a plea. Weigh it by your wants, your feelings toward this hero and your warmth. When influence facts are present they decide whether you agree; otherwise agree to nothing new."
            }
            Self::Threat => {
                "The hero threatens you. React as your nerve and your feelings say: you may give way, look for help or a way out, or stand your ground. Never answer a threat warmly. When influence facts are present they decide whether you give way."
            }
            Self::Deception => {
                "The hero makes a claim to sway you. When deception or influence facts are present they decide whether you believe or act on it; otherwise you are not convinced."
            }
            Self::Flattery => {
                "The hero praises you to win your goodwill. Let your warmth and pride decide how it lands. Praise alone grants nothing."
            }
            Self::Romance => {
                "The hero flirts or courts you. Answer tastefully, within the content rating and your own feelings toward this hero: interest grows slowly with trust and is never automatic. You may welcome it, deflect it kindly or decline it, and you never agree to more than your present feelings allow. A character whose sexual_content_allowed is false keeps it to friendly warmth or a polite refusal."
            }
            Self::Insult => {
                "The hero mocks or belittles you. React as your pride, nerve and feelings say. Do not reward it."
            }
            Self::Unjudged => {
                "The kind of conversation was not judged. Answer the hero's words as they are."
            }
        }
    }

    /// The facts the reply reads for this mode: the mode, the approach it came from and how to
    /// take it up.
    pub(super) fn facts(self, approach: Approach) -> Value {
        json!({
            "mode": self.label(),
            "approach": approach.label(),
            "meaning": self.meaning(),
        })
    }

    /// One audit line per judged line: who was spoken to, the mode and the approach it came from.
    /// The hero is hashed, as in the other social audit lines.
    pub(super) fn record(self, npc_id: &str, hero: &str, approach: Approach, ask: &AskReading) {
        super::super::actor::record_audit_line(
            "gameplay.social.dialogue_mode",
            format!(
                "DIALOGUE_MODE npc={npc_id} actor_hash={} mode={} approach={} focus={}",
                df_observe::seat_hash(hero.as_bytes()),
                self.label(),
                approach.label(),
                if ask.focus.is_some() { "jev" } else { "none" },
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::npc_facts::{Focus, ReadingSource};
    use super::*;

    fn judged(focus: Vec<Focus>) -> AskReading {
        let mut ask = AskReading::neutral();
        ask.source = ReadingSource::Jev;
        ask.focus = Some(focus);
        ask
    }

    const PLAIN: TradeTurn = TradeTurn {
        listed_trade: false,
        bargaining: false,
    };
    const LISTED: TradeTurn = TradeTurn {
        listed_trade: true,
        bargaining: false,
    };
    const BARGAINING: TradeTurn = TradeTurn {
        listed_trade: true,
        bargaining: true,
    };

    #[test]
    fn each_brief_mode_is_reached_from_jevs_approach_focus_and_trade_state() {
        let small_talk = judged(vec![Focus::SmallTalk]);
        let asks = judged(vec![Focus::Place]);
        for (approach, ask, trade, expected) in [
            (Approach::Other, &small_talk, PLAIN, DialogueMode::SmallTalk),
            (
                Approach::Kindness,
                &small_talk,
                PLAIN,
                DialogueMode::SmallTalk,
            ),
            (Approach::Question, &asks, PLAIN, DialogueMode::Question),
            (Approach::Other, &asks, PLAIN, DialogueMode::Question),
            (Approach::Question, &asks, LISTED, DialogueMode::Shopping),
            (
                Approach::Bargain,
                &small_talk,
                LISTED,
                DialogueMode::Shopping,
            ),
            (
                Approach::Question,
                &asks,
                BARGAINING,
                DialogueMode::Negotiation,
            ),
            (
                Approach::Bargain,
                &small_talk,
                PLAIN,
                DialogueMode::Negotiation,
            ),
            (
                Approach::Bribe,
                &small_talk,
                LISTED,
                DialogueMode::Negotiation,
            ),
            (
                Approach::Persuasion,
                &small_talk,
                PLAIN,
                DialogueMode::Persuasion,
            ),
            (
                Approach::Intimidation,
                &asks,
                BARGAINING,
                DialogueMode::Threat,
            ),
            (
                Approach::Deception,
                &small_talk,
                PLAIN,
                DialogueMode::Deception,
            ),
            (
                Approach::Flattery,
                &small_talk,
                LISTED,
                DialogueMode::Flattery,
            ),
            (Approach::Flirting, &asks, LISTED, DialogueMode::Romance),
            (Approach::Insult, &small_talk, PLAIN, DialogueMode::Insult),
        ] {
            assert_eq!(
                DialogueMode::read(approach, ask, trade),
                expected,
                "{} {trade:?}",
                approach.label()
            );
        }
    }

    #[test]
    fn a_line_jev_did_not_read_is_unjudged_and_keeps_every_trade_allowance() {
        let mode = DialogueMode::read(Approach::Other, &AskReading::neutral(), PLAIN);
        assert_eq!(mode, DialogueMode::Unjudged);
        assert!(mode.allows_trade());
        assert!(!mode.may_urge());
        // A judged trade turn still reads as shopping when the approach and focus are silent.
        assert_eq!(
            DialogueMode::read(Approach::Other, &AskReading::neutral(), LISTED),
            DialogueMode::Shopping
        );
    }

    #[test]
    fn only_trade_and_questions_draw_on_trade_allowances_and_only_requests_are_urged() {
        for mode in [
            DialogueMode::SmallTalk,
            DialogueMode::Persuasion,
            DialogueMode::Threat,
            DialogueMode::Deception,
            DialogueMode::Flattery,
            DialogueMode::Romance,
            DialogueMode::Insult,
        ] {
            assert!(!mode.allows_trade(), "{}", mode.label());
        }
        for mode in [
            DialogueMode::Shopping,
            DialogueMode::Negotiation,
            DialogueMode::Question,
        ] {
            assert!(mode.allows_trade(), "{}", mode.label());
        }
        for mode in [
            DialogueMode::SmallTalk,
            DialogueMode::Question,
            DialogueMode::Shopping,
            DialogueMode::Insult,
        ] {
            assert!(!mode.may_urge(), "{}", mode.label());
        }
    }

    #[test]
    fn mode_facts_name_the_mode_and_its_source_approach_and_romance_respects_consent() {
        let facts = DialogueMode::Romance.facts(Approach::Flirting);
        assert_eq!(facts["mode"], "romance");
        assert_eq!(facts["approach"], "flirting");
        let meaning = facts["meaning"].as_str().unwrap();
        assert!(meaning.contains("never automatic"));
        assert!(meaning.contains("sexual_content_allowed is false"));
    }
}
