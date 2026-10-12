//! Reading spoken text without a model: its sentences, the words that carry meaning, and the
//! checks a reply must pass to be worth a speech bubble. The storyteller applies the checks to
//! every spoken slot, the NPC fact selection ranks lines against the hero's words with the same
//! word reading, and the transcript rules (`npc_talk_rules`) use all of it on recorded
//! conversations, so the server and its tests agree on what a sentence, a repeat and a denial are.

use std::collections::BTreeSet;

/// A sentence is not counted as a repeat below this many words.
pub(super) const REPEAT_MIN_WORDS: usize = 4;

/// Words with typographic apostrophes unified, lower-cased.
pub(super) fn normalised(text: &str) -> String {
    text.replace(['\u{2019}', '\u{2018}'], "'").to_lowercase()
}

/// The sentences of `text`, each ending at `.`, `!` or `?` followed by a space or the end.
pub(super) fn sentences(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        let ends = matches!(character, '.' | '!' | '?')
            && characters
                .peek()
                .is_none_or(|(_, next)| next.is_whitespace() || matches!(next, '"' | '\u{201d}'));
        if !ends {
            continue;
        }
        let end = index + character.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() {
            found.push(sentence.to_owned());
        }
        start = end;
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        found.push(rest.to_owned());
    }
    found
}

/// Lower-case words made of letters, digits and inner apostrophes.
pub(super) fn words(text: &str) -> Vec<String> {
    normalised(text)
        .split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .map(|word| word.trim_matches('\'').to_owned())
        .filter(|word| !word.is_empty())
        .collect()
}

const FILLER_WORDS: [&str; 105] = [
    "a", "an", "the", "and", "or", "but", "of", "to", "in", "on", "at", "for", "with", "from",
    "by", "is", "are", "was", "were", "be", "been", "am", "i", "you", "he", "she", "it", "we",
    "they", "me", "my", "your", "his", "her", "our", "their", "this", "that", "these", "those",
    "do", "does", "did", "have", "has", "had", "not", "no", "so", "if", "as", "will", "would",
    "can", "could", "what", "who", "how", "when", "where", "why", "about", "there", "then", "tell",
    "say", "know", "need", "want", "any", "them", "him", "us", "one", "ones", "here", "very",
    "much", "more", "some", "just", "into", "than", "also", "like", "get", "got", "must", "may",
    "might", "should", "shall", "please", "now", "still", "too", "even", "only", "same", "such",
    "other", "over", "after", "before", "while",
];

/// The words of `text` that carry meaning: no filler, at least three letters, plural `s` cut.
pub(super) fn content_words(text: &str) -> BTreeSet<String> {
    words(text)
        .into_iter()
        .filter(|word| word.chars().count() >= 3 && !FILLER_WORDS.contains(&word.as_str()))
        .map(|word| word.trim_end_matches('s').to_owned())
        .collect()
}

/// How many meaning-carrying words the two texts share.
pub(super) fn shared_words(a: &str, b: &str) -> usize {
    let (a, b) = (content_words(a), content_words(b));
    a.intersection(&b).count()
}

/// The fraction of `of`'s meaning-carrying words that `within` also uses.
pub(super) fn coverage(of: &str, within: &str) -> f64 {
    let wanted = content_words(of);
    if wanted.is_empty() {
        return 0.0;
    }
    let present = content_words(within);
    wanted.iter().filter(|word| present.contains(*word)).count() as f64 / wanted.len() as f64
}

const DENIAL_PHRASES: [&str; 20] = [
    "i don't know",
    "i do not know",
    "i didn't know",
    "i can't say",
    "i cannot say",
    "i can't tell",
    "i cannot tell",
    "i couldn't say",
    "i could not say",
    "i have no idea",
    "no idea",
    "i don't have",
    "i do not have",
    "i wouldn't know",
    "i'm not sure",
    "i am not sure",
    "i can't speak",
    "i cannot speak",
    "i know nothing",
    "that name is not part",
];

/// Whether the sentence says the speaker does not know or will not tell.
pub(super) fn is_denial(sentence: &str) -> bool {
    let sentence = normalised(sentence);
    DENIAL_PHRASES
        .iter()
        .any(|phrase| sentence.contains(phrase))
}

/// A sentence reduced to its words, for deciding whether two sentences are the same.
pub(super) fn sentence_key(sentence: &str) -> String {
    words(sentence).join(" ")
}

/// A rule a spoken line must pass, checked after the model writes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SpokenRule {
    /// At most this many sentences.
    SentencesAtMost(usize),
    /// The first sentence may not say the speaker does not know or cannot tell.
    NoDenialFirst,
    /// No sentence of four or more words may repeat one of these earlier sentences.
    NoRepeat(Vec<String>),
}

impl SpokenRule {
    /// What the rule found wrong, or `None` when `text` passes. The words never contain the text.
    pub(super) fn violation(&self, text: &str) -> Option<&'static str> {
        let parts = sentences(text);
        match self {
            Self::SentencesAtMost(most) => {
                (parts.len() > *most).then_some("too many sentences; write at most three")
            }
            Self::NoDenialFirst => parts
                .first()
                .is_some_and(|first| is_denial(first))
                .then_some(
                    "the first sentence denied knowing; answer first from the facts you were given",
                ),
            Self::NoRepeat(earlier) => {
                let known: BTreeSet<String> = earlier
                    .iter()
                    .flat_map(|line| sentences(line))
                    .filter(|part| words(part).len() >= REPEAT_MIN_WORDS)
                    .map(|part| sentence_key(&part))
                    .collect();
                parts
                    .iter()
                    .any(|part| known.contains(&sentence_key(part)))
                    .then_some("a sentence repeated something already said; say something new")
            }
        }
    }
}
