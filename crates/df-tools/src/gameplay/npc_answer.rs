//! Bounded answers for personal questions that previously invented facts in live play.
//!
//! This is an authoritative answer route, not a semantic validator for arbitrary model prose.
//! Only the listed question subjects use it. Fact references resolve to exact authored speech;
//! neither shared words nor an unrelated selected candidate grant authority to add a claim.

use super::{NpcReplyContext, npc_dossier};
use crate::gameplay::{
    adventure::dossier::{AnswerSubject, QuestionAuthority, QuestionSubject},
    npc_facts::{self, AskReading},
};

pub(super) struct Answer {
    pub(super) text: String,
    pub(super) keys: Vec<String>,
    pub(super) withheld: bool,
}

fn authored(context: &NpcReplyContext, subject: QuestionSubject) -> Option<(String, String)> {
    let turn = context.turn.as_ref()?;
    let authority = turn.dossier.question_answers.get(&subject);
    // Reward already has an authored, typed answer in generated adventures. It never grants
    // authority to invent a payment or invert the promised reward into "nothing was promised".
    let reward = QuestionAuthority::Subject(AnswerSubject::Reward);
    let authority =
        authority.or_else(|| (subject == QuestionSubject::Payment).then_some(&reward))?;
    match authority {
        QuestionAuthority::Knowledge(id) if !id.trim().is_empty() => {
            let public = turn.dossier.knowledge.says_freely.iter();
            let trusted = turn.dossier.knowledge.says_when_trusted.iter().filter(|_| {
                turn.standing.trusted(&turn.dossier) && turn.conversation.exchange >= 2
            });
            let mut matches = public.chain(trusted).filter(|fact| fact.id == *id);
            let fact = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            (!fact.text.trim().is_empty()
                && !(fact.source == "authored_line" && turn.later_lines.contains(&fact.text))
                && !turn.withheld.leaks(&fact.text))
            .then(|| (fact.text.clone(), format!("question_fact:{id}")))
        }
        QuestionAuthority::Subject(subject) => {
            let answer = turn.dossier.subject_answers.get(subject)?;
            (!answer.text.trim().is_empty() && !turn.withheld.leaks(&answer.text))
                .then(|| (answer.text.clone(), format!("subject:{}", subject.label())))
        }
        QuestionAuthority::Knowledge(_) => None,
    }
}

/// A routing label is not a personal-name fact. Authored knowledge keeps its existing trust
/// gate; a profile may separately supply an exact public alias, name or deferral.
pub(super) fn identity(context: &NpcReplyContext) -> Option<(String, String)> {
    if context
        .withheld_subjects
        .contains(&QuestionSubject::Identity)
        || context.turn.as_ref().is_some_and(|turn| {
            turn.dossier
                .withheld_subjects
                .contains(&QuestionSubject::Identity)
        })
    {
        return None;
    }
    authored(context, QuestionSubject::Identity).or_else(|| {
        let text = context.identity_reply.as_ref()?;
        let forbidden = context.facts["forbidden"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str);
        let leaks = forbidden
            .chain(context.private_terms.iter().map(String::as_str))
            .any(|term| crate::gameplay::adventure::dossier::mentions_term(text, term))
            || context
                .turn
                .as_ref()
                .is_some_and(|turn| turn.withheld.leaks(text));
        (!text.trim().is_empty() && !leaks)
            .then(|| (text.clone(), "identity:public_reply".to_owned()))
    })
}

/// Answers a part that asks no question subject from the dossier's own subject answers, under the
/// reply's subject (Jev's). Without a dossier there is no authored answer.
fn ordinary_part(context: &NpcReplyContext, subject: AnswerSubject) -> Vec<(String, String)> {
    let Some(turn) = context.turn.as_ref() else {
        return Vec::new();
    };
    // "Other" is no subject at all: it never speaks for an authored answer.
    if subject == AnswerSubject::Other {
        return Vec::new();
    }
    if let Some(answer) = npc_dossier::subject_fallback(
        &turn.dossier,
        subject,
        &turn.withheld,
        &npc_dossier::Memory {
            conversation: &turn.conversation,
            said: &turn.said,
            offered: &turn.offered,
        },
    ) {
        return vec![answer];
    }
    // Only the subject's own authored answer answers an ordinary part. Other offered lines stay in
    // the facts for the model, so a part Jev read as a room or small-talk question is not answered
    // with an unrelated line.
    Vec::new()
}

/// The authored answer for the hero's question parts, from Jev's per-part focus. The reply's
/// `subject` is Jev's answer subject for parts that ask no question subject. `None` means the model
/// reply answers: Jev did not read the line (a neutral reading), a question part has no authored
/// answer, a price question sits in a merchant's trade scope, or nothing authored answered at all.
/// Withheld parts stay withheld.
pub(super) fn prepare(
    context: &NpcReplyContext,
    reading: &AskReading,
    subject: AnswerSubject,
) -> Option<Answer> {
    let parts = npc_facts::question_parts(&context.typed_words);
    let foci = reading.focus.as_ref()?;
    let questions: Vec<Option<QuestionSubject>> = (0..parts.len())
        .map(|index| foci.get(index).and_then(|focus| focus.question_subject()))
        .collect();
    if questions.iter().all(Option::is_none) {
        return None;
    }
    let mut lines = Vec::new();
    let mut keys = Vec::new();
    let mut withheld = false;
    for question in questions {
        let answer = match question {
            Some(question)
                if context.withheld_subjects.contains(&question)
                    || context
                        .turn
                        .as_ref()
                        .is_some_and(|turn| turn.dossier.withheld_subjects.contains(&question)) =>
            {
                withheld = true;
                continue;
            }
            // A price or trade question in a merchant's trade scope is answered by the model with
            // the trade facts; no authored answer may speak for what a good costs.
            Some(QuestionSubject::Payment) if context.merchant_trade_scope => return None,
            Some(question) => {
                let found = if question == QuestionSubject::Identity {
                    identity(context)
                } else {
                    authored(context, question)
                };
                // A missing authored answer is not an answer: the model reply runs instead.
                vec![found?]
            }
            None => ordinary_part(context, subject),
        };
        for (text, key) in answer {
            let length = lines
                .iter()
                .map(|line: &String| line.chars().count() + 1)
                .sum::<usize>();
            let accepted = lines.contains(&text)
                || length + text.chars().count() <= npc_dossier::REPLY_CHARACTERS;
            if accepted && !lines.contains(&text) {
                lines.push(text);
            }
            if accepted && !key.is_empty() {
                keys.push(key);
            }
        }
    }
    // Nothing authored answered and nothing was withheld: the model reply answers, not a canned line.
    if lines.is_empty() && !withheld {
        return None;
    }
    Some(Answer {
        text: lines.join(" "),
        keys,
        withheld,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_question_focus_names_a_question_subject() {
        assert_eq!(
            npc_facts::Focus::Motive.question_subject(),
            Some(QuestionSubject::Motive)
        );
        assert_eq!(npc_facts::Focus::SmallTalk.question_subject(), None);
        assert_eq!(npc_facts::Focus::Other.question_subject(), None);
    }

    #[test]
    fn a_neutral_reading_leaves_every_part_to_the_model() {
        let context = context_without_authored_answers(PRICE_LINE);
        assert!(prepare(&context, &AskReading::neutral(), AnswerSubject::Other).is_none());
    }

    const PRICE_LINE: &str = "Good day, Marek. What are you asking for the traveler's clothes?";

    /// A context for the line whose dossier has no authored question or reward answers.
    fn context_without_authored_answers(words: &str) -> NpcReplyContext {
        use crate::gameplay::{
            npc_dossier_tests::{adventure, member, story, view},
            pack::SceneKey,
            social::SocialLedger,
        };
        let generated = adventure(1);
        let ledger = SocialLedger::open(generated.dossiers.clone(), "answer-price", None).unwrap();
        let mut context = NpcReplyContext::for_cast(
            &view(&generated, SceneKey::Opening, "Brannoch"),
            member(1),
            words,
            &story(&generated),
            &ledger,
            "Opening",
        )
        .unwrap();
        let turn = context.turn.as_mut().unwrap();
        turn.dossier.question_answers.clear();
        turn.dossier.subject_answers.remove(&AnswerSubject::Reward);
        context
    }

    #[test]
    fn a_price_question_without_an_authored_answer_goes_to_the_model_not_a_canned_line() {
        let reading =
            AskReading::from_focus(vec![npc_facts::Focus::SmallTalk, npc_facts::Focus::Payment]);
        let plain = context_without_authored_answers(PRICE_LINE);
        assert!(prepare(&plain, &reading, AnswerSubject::Other).is_none());
        let mut trade = context_without_authored_answers(PRICE_LINE);
        trade.merchant_trade_scope = true;
        assert!(prepare(&trade, &reading, AnswerSubject::Other).is_none());
    }

    #[test]
    fn a_withheld_price_question_in_a_merchant_trade_keeps_its_private_refusal() {
        let reading =
            AskReading::from_focus(vec![npc_facts::Focus::SmallTalk, npc_facts::Focus::Payment]);
        let mut trade = context_without_authored_answers(PRICE_LINE);
        trade.merchant_trade_scope = true;
        trade.withheld_subjects.push(QuestionSubject::Payment);
        let answer = prepare(&trade, &reading, AnswerSubject::Other);
        assert!(answer.is_some_and(|answer| answer.withheld));
    }
}
