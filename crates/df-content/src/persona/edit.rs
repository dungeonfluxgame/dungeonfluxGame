//! Changing a persona: one list of typed edits applied all together or not at all.
//!
//! An edit list is applied to a draft of the persona, and the draft is checked as a whole
//! ([`Persona::new`]), so a refused edit changes nothing and an edit that leaves the persona
//! inconsistent (removing a behavior a trigger still names) is refused. The id never changes.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::policy::{PersonaBehavior, PersonaSecret, PersonaTrigger};
use super::{Persona, PersonaDraft, PersonaError, PersonaField};

/// One change to a persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonaEdit {
    /// Replaces a text field: `display_name`, `job`, `speech_style`, `backstory` or
    /// `private_backstory`.
    SetText { field: PersonaField, text: String },
    /// Adds `text` after a text field's existing text, on a new line; sets it when it is empty.
    /// For `backstory` and `private_backstory`: the old text stays as it was.
    AppendText { field: PersonaField, text: String },
    /// Adds an entry to a list field: `personality`, `motivations`, `goals`, `needs`, `fears` or
    /// `boundaries`.
    AddEntry { field: PersonaField, text: String },
    /// Replaces entry `index` (from 0) of a list field.
    SetEntry {
        field: PersonaField,
        index: usize,
        text: String,
    },
    /// Removes entry `index` (from 0) of a list field.
    RemoveEntry { field: PersonaField, index: usize },
    /// Adds a secret, or replaces the one with its id.
    PutSecret { secret: PersonaSecret },
    /// Removes a secret by id.
    RemoveSecret { id: String },
    /// Adds a behavior, or replaces the one with its id.
    PutBehavior { behavior: PersonaBehavior },
    /// Removes a behavior by id. Refused while a trigger or the default still names it.
    RemoveBehavior { id: String },
    /// Sets the behavior the character starts in, or clears it.
    SetDefaultBehavior { id: Option<String> },
    /// Adds a trigger, or replaces the one with its id.
    PutTrigger { trigger: PersonaTrigger },
    /// Removes a trigger by id.
    RemoveTrigger { id: String },
}

/// Why an edit list was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum EditError {
    /// The persona that would result is not valid.
    Persona { cause: PersonaError },
    /// `field` is not a text field (or is the id, which never changes).
    NotAText { field: PersonaField },
    /// `field` is not a list of entries.
    NotAList { field: PersonaField },
    /// A list has no entry at `index`.
    NoSuchEntry { field: PersonaField, index: usize },
    /// There is no item with `id` in `field`'s list.
    NoSuchItem { field: PersonaField, id: String },
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persona { cause } => write!(formatter, "{cause}"),
            Self::NotAText { field } => write!(
                formatter,
                "the persona's {} is not a text you can set",
                field.key()
            ),
            Self::NotAList { field } => write!(
                formatter,
                "the persona's {} is not a list of entries",
                field.key()
            ),
            Self::NoSuchEntry { field, index } => write!(
                formatter,
                "the persona's {} has no entry {index}",
                field.key()
            ),
            Self::NoSuchItem { field, id } => write!(
                formatter,
                "the persona's {} has no item {id:?}",
                field.key()
            ),
        }
    }
}

impl std::error::Error for EditError {}

impl From<PersonaError> for EditError {
    fn from(cause: PersonaError) -> Self {
        Self::Persona { cause }
    }
}

fn text_mut(draft: &mut PersonaDraft, field: PersonaField) -> Result<&mut String, EditError> {
    match field {
        PersonaField::DisplayName => Ok(&mut draft.display_name),
        PersonaField::Job => Ok(&mut draft.job),
        PersonaField::SpeechStyle => Ok(&mut draft.speech_style),
        PersonaField::Backstory => Ok(&mut draft.backstory),
        PersonaField::PrivateBackstory => Ok(&mut draft.private_backstory),
        _ => Err(EditError::NotAText { field }),
    }
}

fn list_mut(draft: &mut PersonaDraft, field: PersonaField) -> Result<&mut Vec<String>, EditError> {
    match field {
        PersonaField::Personality => Ok(&mut draft.personality),
        PersonaField::Motivations => Ok(&mut draft.motivations),
        PersonaField::Goals => Ok(&mut draft.goals),
        PersonaField::Needs => Ok(&mut draft.needs),
        PersonaField::Fears => Ok(&mut draft.fears),
        PersonaField::Boundaries => Ok(&mut draft.boundaries),
        _ => Err(EditError::NotAList { field }),
    }
}

/// Puts `item` in `items`, replacing the one with the same id, or appends it.
fn put<T>(items: &mut Vec<T>, item: T, id_of: impl Fn(&T) -> &str) {
    match items.iter().position(|seen| id_of(seen) == id_of(&item)) {
        Some(index) => items[index] = item,
        None => items.push(item),
    }
}

/// Removes the item with `id`, or says there is none.
fn remove<T>(
    items: &mut Vec<T>,
    field: PersonaField,
    id: &str,
    id_of: impl Fn(&T) -> &str,
) -> Result<(), EditError> {
    let index = items
        .iter()
        .position(|seen| id_of(seen) == id)
        .ok_or_else(|| EditError::NoSuchItem {
            field,
            id: id.to_owned(),
        })?;
    items.remove(index);
    Ok(())
}

fn apply(draft: &mut PersonaDraft, edit: &PersonaEdit) -> Result<(), EditError> {
    match edit {
        PersonaEdit::SetText { field, text } => {
            *text_mut(draft, *field)? = text.clone();
        }
        PersonaEdit::AppendText { field, text } => {
            let target = text_mut(draft, *field)?;
            if !target.trim().is_empty() {
                target.push('\n');
            }
            // A new line never starts or ends with the blanks a model puts round it.
            target.push_str(text.trim());
        }
        PersonaEdit::AddEntry { field, text } => {
            list_mut(draft, *field)?.push(text.trim().to_owned());
        }
        PersonaEdit::SetEntry { field, index, text } => {
            let list = list_mut(draft, *field)?;
            let entry = list.get_mut(*index).ok_or(EditError::NoSuchEntry {
                field: *field,
                index: *index,
            })?;
            *entry = text.trim().to_owned();
        }
        PersonaEdit::RemoveEntry { field, index } => {
            let list = list_mut(draft, *field)?;
            if *index >= list.len() {
                return Err(EditError::NoSuchEntry {
                    field: *field,
                    index: *index,
                });
            }
            list.remove(*index);
        }
        PersonaEdit::PutSecret { secret } => {
            put(&mut draft.secrets, secret.clone(), |item| &item.id);
        }
        PersonaEdit::RemoveSecret { id } => {
            remove(&mut draft.secrets, PersonaField::Secrets, id, |item| {
                &item.id
            })?;
        }
        PersonaEdit::PutBehavior { behavior } => {
            put(&mut draft.behaviors, behavior.clone(), |item| &item.id);
        }
        PersonaEdit::RemoveBehavior { id } => {
            remove(&mut draft.behaviors, PersonaField::Behaviors, id, |item| {
                &item.id
            })?;
        }
        PersonaEdit::SetDefaultBehavior { id } => {
            draft.default_behavior = id.clone();
        }
        PersonaEdit::PutTrigger { trigger } => {
            put(&mut draft.triggers, trigger.clone(), |item| &item.id);
        }
        PersonaEdit::RemoveTrigger { id } => {
            remove(&mut draft.triggers, PersonaField::Triggers, id, |item| {
                &item.id
            })?;
        }
    }
    Ok(())
}

impl Persona {
    /// The persona as an unchecked draft, to change and check again ([`Persona::new`]).
    pub fn to_draft(&self) -> PersonaDraft {
        PersonaDraft {
            id: self.id.as_str().to_owned(),
            display_name: self.display_name.clone(),
            job: self.job.clone(),
            personality: self.personality.clone(),
            motivations: self.motivations.clone(),
            goals: self.goals.clone(),
            needs: self.needs.clone(),
            fears: self.fears.clone(),
            speech_style: self.speech_style.clone(),
            backstory: self.backstory.clone(),
            private_backstory: self.private_backstory.clone(),
            boundaries: self.boundaries.clone(),
            secrets: self.secrets.clone(),
            behaviors: self.behaviors.clone(),
            default_behavior: self.default_behavior.clone(),
            triggers: self.triggers.clone(),
        }
    }

    /// The persona with `edits` applied in order, all together or not at all: the result is
    /// checked as a whole, so an edit list that leaves it invalid changes nothing.
    pub fn edit(&self, edits: &[PersonaEdit]) -> Result<Persona, EditError> {
        let mut draft = self.to_draft();
        for edit in edits {
            apply(&mut draft, edit)?;
        }
        Ok(Persona::new(draft)?)
    }
}
