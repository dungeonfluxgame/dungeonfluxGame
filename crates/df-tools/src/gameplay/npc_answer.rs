//! Bounded answers for personal questions that previously invented facts in live play.
//!
//! This is an authoritative answer route, not a semantic validator for arbitrary model prose.
//! Only the listed question subjects use it. Fact references resolve to exact authored speech;
//! neither shared words nor an unrelated selected candidate grant authority to add a claim.

use super::{NpcReplyContext, npc_dossier};
use crate::gameplay::{
    adventure::dossier::{AnswerSubject, QuestionAuthority, QuestionSubject},
    npc_facts::{self, AskReading},
    social::{self, Approach},
    speech_text::shared_words,
};

pub(super) struct Answer {
    pub(super) text: String,
    pub(super) keys: Vec<String>,
    pub(super) withheld: bool,
}

/// Keyword fallback for the question subject of one part. It decides only when Jev did not read
/// the line; the recognised questions are bounded so it never guesses whether a claim is true.
fn keyword_subject(part: &str) -> Option<QuestionSubject> {
    let part = part.to_lowercase().replace('’', "'");
    let has = |phrases: &[&str]| phrases.iter().any(|phrase| part.contains(phrase));
    let following_question = part.trim_start().starts_with("why he's following ")
        || part.trim_start().starts_with("why is he following ");
    if has(&[
        "how do you know",
        "how did you know",
        "did you see",
        "did you witness",
        "what did you see",
        "what's your evidence",
        "what is your evidence",
        "how can you be sure",
    ]) {
        Some(QuestionSubject::Evidence)
    } else if has(&[
        "who sent you",
        "who hired you",
        "who do you work for",
        "who are you working for",
        "your employer",
        "who pays you",
        "who's paying you",
        "who is paying you",
        "whose orders",
    ]) {
        Some(QuestionSubject::Employer)
    } else if has(&[
        "payment",
        "paid",
        "pay you",
        "pay were",
        "pay was",
        "reward",
        "promised",
        "compensation",
    ]) {
        Some(QuestionSubject::Payment)
    } else if has(&[
        "delivery before",
        "delivered before",
        "deliveries before",
        "made this delivery",
        "done this before",
        "first delivery",
        "first time delivering",
        "ever delivered",
        "delivered this before",
        "delivered it before",
        "delivered packets before",
        "previous deliveries",
    ]) {
        Some(QuestionSubject::DeliveryHistory)
    } else if has(&[
        "are you hurt",
        "were you hurt",
        "you injured",
        "you wounded",
        "you bleeding",
        "you winded",
        "are you all right",
        "are you alright",
        "are you okay",
        "how badly are you hurt",
        "your injuries",
    ]) {
        Some(QuestionSubject::Condition)
    } else if following_question
        || has(&[
            "why do you want",
            "why do you need",
            "why want",
            "why are you after",
            "why are you chasing",
            "why are you following",
            "why do you follow",
            "why are you pursuing",
            "what do you want",
            "what's in it for you",
            "what is in it for you",
            "your motive",
            "why does this matter to you",
        ])
    {
        Some(QuestionSubject::Motive)
    } else if has(&[
        "who are you",
        "your name",
        "what should i call you",
        "what do i call you",
    ]) {
        Some(QuestionSubject::Identity)
    } else {
        None
    }
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

/// Answers a part that asks no question subject from the dossier's own subject answers. The
/// subject is Jev's when Jev read the line, otherwise the keyword subject of this part.
fn ordinary_part(
    context: &NpcReplyContext,
    part: &str,
    subject: Option<AnswerSubject>,
) -> Vec<(String, String)> {
    let Some(turn) = context.turn.as_ref() else {
        let part = part.to_lowercase();
        if (part.contains("what happened") || part.contains("what's happened"))
            && let Some(line) = context.facts["npc_current_scripted_line"].as_str()
            && !line.trim().is_empty()
            && !context.facts["forbidden"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .any(|term| crate::gameplay::adventure::dossier::mentions_term(line, term))
        {
            return vec![(line.to_owned(), "current_scripted_line".to_owned())];
        }
        return Vec::new();
    };
    let subject = subject.unwrap_or_else(|| {
        social::classify_keywords_reading_for_dossier(part, &turn.dossier).subject
    });
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
    // In a compound identity question, preserve the other offered, authored answer rather
    // than reducing "who are you and what happened?" to the name alone.
    let mut answers: Vec<_> = context.facts["answer_from"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|line| !turn.withheld.leaks(line) && shared_words(part, line) > 0)
        .map(|line| (line.to_owned(), String::new()))
        .collect();
    let part = part.to_lowercase();
    if answers.is_empty()
        && (part.contains("what happened") || part.contains("what's happened"))
        && let Some(opening) = context.facts["your_opening_words"].as_str()
        && !turn.withheld.leaks(opening)
    {
        answers.push((opening.to_owned(), String::new()));
    }
    answers
}

/// The authored answer for the hero's question parts. Jev's per-part focus names each part's
/// question subject when it read the line; the keyword subject decides only without Jev. The
/// reply's `subject` is the answer subject for parts that ask no question subject. `None` means
/// the model reply answers: a question part has no authored answer, a price question sits in a
/// merchant's trade scope, or nothing authored answered at all. Withheld parts stay withheld.
pub(super) fn prepare(
    context: &NpcReplyContext,
    approach: Approach,
    reading: &AskReading,
    subject: AnswerSubject,
) -> Option<Answer> {
    let parts = npc_facts::question_parts(&context.typed_words);
    let questions: Vec<Option<QuestionSubject>> = match &reading.focus {
        Some(foci) => (0..parts.len())
            .map(|index| foci.get(index).and_then(|focus| focus.question_subject()))
            .collect(),
        None => {
            if approach != Approach::Question && !context.typed_words.contains('?') {
                return None;
            }
            parts.iter().map(|part| keyword_subject(part)).collect()
        }
    };
    if questions.iter().all(Option::is_none) {
        return None;
    }
    let mut lines = Vec::new();
    let mut keys = Vec::new();
    let mut withheld = false;
    for (part, question) in parts.iter().zip(questions) {
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
            None => ordinary_part(context, part, reading.focus.is_some().then_some(subject)),
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
    fn following_motive_requires_a_question_instead_of_an_ordinary_mention() {
        for question in [
            "Bandit, why are you following the Courier?",
            "Why he's following the courier?",
            "Why he’s following the courier?",
            "Why is he following the courier?",
        ] {
            assert_eq!(keyword_subject(question), Some(QuestionSubject::Motive));
        }
        for statement in [
            "He's following the Courier.",
            "I know why he's following the Courier.",
        ] {
            assert_eq!(keyword_subject(statement), None);
        }
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
        assert!(prepare(&plain, Approach::Bargain, &reading, AnswerSubject::Other).is_none());
        let mut trade = context_without_authored_answers(PRICE_LINE);
        trade.merchant_trade_scope = true;
        assert!(prepare(&trade, Approach::Bargain, &reading, AnswerSubject::Other).is_none());
    }

    #[test]
    fn a_withheld_price_question_in_a_merchant_trade_keeps_its_private_refusal() {
        let reading =
            AskReading::from_focus(vec![npc_facts::Focus::SmallTalk, npc_facts::Focus::Payment]);
        let mut trade = context_without_authored_answers(PRICE_LINE);
        trade.merchant_trade_scope = true;
        trade.withheld_subjects.push(QuestionSubject::Payment);
        let answer = prepare(&trade, Approach::Bargain, &reading, AnswerSubject::Other);
        assert!(answer.is_some_and(|answer| answer.withheld));
    }
}
