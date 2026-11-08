//! What a character calls the others in its prompts. A persona id may carry a name
//! (`mara_adventurer`), so a character who does not know someone's name never sees their id: it
//! sees a handle with no name in it (`person-1`, `person-2`, in the order the others are met),
//! wherever the id would be: who is present, who asked, the labels of recent lines, the briefs of
//! the others and its own standings and attention. What the model answers with a handle is read
//! back to the id in code. Run 59: Haldor, who knew no one, said "Mara" from the ids and the
//! scene's words alone. Names are learned, never given (owner rule; `known_names` in the state).

use df_content::persona::{Appraisal, PersonaBrief, PersonaState, PublicPersona};
use serde_json::Value;

/// The prefix of a handle; a handle is `person-<n>`.
const HANDLE_PREFIX: &str = "person-";
/// What the character calls someone it was not told of in this prompt.
const UNKNOWN: &str = "someone";

/// The ids a character may meet in one prompt and what it calls each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Handles {
    /// The character itself, called by its own id.
    own: String,
    /// The ids whose name it knows: their id is shown wherever it appears.
    known: Vec<String>,
    /// Each id with what the character calls it: the id itself when it knows the name, a handle
    /// otherwise.
    shown: Vec<(String, String)>,
}

impl Handles {
    /// What the character of `state` calls each of `ids` (duplicates once, in order) and everyone
    /// in its own standings and attention. Every prompt of one scene passes the scene's cast
    /// first, in its order, so a handle means the same person in a line and in its backfill.
    pub(super) fn new<'a>(state: &'a PersonaState, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let from_state = state
            .relationships
            .iter()
            .map(|relationship| relationship.toward.as_str())
            .chain(state.attention.focus.as_deref())
            .chain(
                state
                    .attention
                    .noticed
                    .iter()
                    .map(|noticed| noticed.speaker.as_str()),
            );
        let mut shown: Vec<(String, String)> = Vec::new();
        let mut strangers = 0;
        for id in ids.into_iter().chain(from_state) {
            if id == state.id || shown.iter().any(|(seen, _)| seen == id) {
                continue;
            }
            let called = if state.known_names.iter().any(|known| known == id) {
                id.to_owned()
            } else {
                strangers += 1;
                format!("{HANDLE_PREFIX}{strangers}")
            };
            shown.push((id.to_owned(), called));
        }
        Self {
            own: state.id.clone(),
            known: state.known_names.clone(),
            shown,
        }
    }

    /// What the storyteller calls each of `ids`, in order: everyone by a handle, since an id may
    /// carry a name or a hidden part (`bought_guard`) the prose it writes must never echo.
    pub(super) fn author<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let shown = ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id.to_owned(), format!("{HANDLE_PREFIX}{}", index + 1)))
            .collect();
        Self {
            own: String::new(),
            known: Vec::new(),
            shown,
        }
    }

    /// What the character calls `id`: itself and anyone whose name it knows by their id, a
    /// stranger it was told of by their handle, and anyone else `someone`.
    pub(super) fn shown(&self, id: &str) -> String {
        if id == self.own || self.known.iter().any(|known| known == id) {
            return id.to_owned();
        }
        self.shown
            .iter()
            .find(|(seen, _)| seen == id)
            .map_or_else(|| UNKNOWN.to_owned(), |(_, called)| called.clone())
    }

    /// The id the character means by `called`, a handle or an id it was shown.
    pub(super) fn id_of(&self, called: &str) -> Option<String> {
        self.shown
            .iter()
            .find(|(_, shown)| shown == called)
            .map(|(id, _)| id.clone())
    }

    /// Whether the character does not know the name of `id`.
    fn stranger(&self, id: &str) -> bool {
        self.shown(id) != id
    }

    /// A recent line as the character is shown it. Its `<id>: <line>` label (the dev callers'
    /// format, not speech) is shown as `shown` gives it, so a label that is no one the
    /// character knows (an id of someone gone, a display name) becomes `someone`.
    pub(super) fn recent_line(&self, line: &str) -> String {
        match line.split_once(": ") {
            Some((label, said)) => format!("{}: {said}", self.shown(label)),
            None => line.to_owned(),
        }
    }

    /// `brief` as the character is told it (see `public_view`).
    pub(super) fn listener_brief(&self, mut brief: PersonaBrief) -> PersonaBrief {
        brief.public = self.public_view(brief.public);
        brief
    }

    /// `public` as the character is told it: a stranger keeps only what anyone could see or hear
    /// (trade, manner, speech), under a handle, with no name, no backstory, no voice samples and
    /// no expertise, which may carry the name.
    pub(super) fn public_view(&self, mut public: PublicPersona) -> PublicPersona {
        if self.stranger(&public.id) {
            public.id = self.shown(&public.id);
            public.display_name = String::new();
            public.backstory = String::new();
            public.voice = Default::default();
            public.expertise = Vec::new();
        }
        public
    }

    /// The world facts as the character is told them: each stall's keeper as `shown` gives it.
    pub(super) fn world_facts(&self, mut facts: Value) -> Value {
        if let Some(stalls) = facts.get_mut("stalls").and_then(Value::as_array_mut) {
            for stall in stalls {
                if let Some(keeper) = stall.get_mut("keeper")
                    && let Some(id) = keeper.as_str().map(str::to_owned)
                {
                    *keeper = Value::from(self.shown(&id));
                }
            }
        }
        facts
    }

    /// The character's own state as its prompt carries it, with strangers under their handles.
    pub(super) fn state(&self, state: &PersonaState) -> PersonaState {
        let mut shown = state.clone();
        for relationship in &mut shown.relationships {
            relationship.toward = self.shown(&relationship.toward);
        }
        if let Some(focus) = shown.attention.focus.as_mut() {
            *focus = self.shown(focus);
        }
        for noticed in &mut shown.attention.noticed {
            noticed.speaker = self.shown(&noticed.speaker);
        }
        shown
    }

    /// `appraisal` with the handles it names read back to ids. It was checked against the
    /// handles before (`read_line`), so what it names is one of them.
    pub(super) fn appraisal_ids(&self, mut appraisal: Appraisal) -> Appraisal {
        let back = |called: &mut String| {
            if let Some(id) = self.id_of(called) {
                *called = id;
            }
        };
        if let Some(toward) = appraisal.toward.as_mut() {
            back(toward);
        }
        if let Some(focus) = appraisal.attention.focus.as_mut() {
            back(focus);
        }
        for noticed in &mut appraisal.attention.noticed {
            back(&mut noticed.speaker);
        }
        appraisal
    }
}
