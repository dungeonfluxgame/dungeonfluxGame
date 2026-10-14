//! Shared bounds for the server-provided party notebook view.

use std::collections::BTreeSet;

pub const MAX_NOTEBOOK_ENTRIES: usize = 256;
pub const MAX_NOTEBOOK_TITLE_BYTES: usize = 96;
pub const MAX_NOTEBOOK_TEXT_BYTES: usize = 4096;
pub const MAX_NOTEBOOK_SCENE_BYTES: usize = 64;
pub const MAX_NOTEBOOK_HERO_BYTES: usize = 96;
pub const MAX_NOTEBOOK_ATTITUDE_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotebookKind {
    Person,
    Place,
    Task,
    Clue,
    Note,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotebookEntryView {
    pub kind: NotebookKind,
    pub title: String,
    pub text: String,
    pub scene: String,
    pub order: u64,
    pub attitude: Option<String>,
    pub owner: Option<String>,
    pub author: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotebookView {
    pub entries: Vec<NotebookEntryView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotebookValidationError {
    TooManyEntries,
    EmptyField,
    FieldTooLong,
    InvalidAudience,
    InvalidOrder,
}

impl NotebookView {
    pub fn validate(&self) -> Result<(), NotebookValidationError> {
        if self.entries.len() > MAX_NOTEBOOK_ENTRIES {
            return Err(NotebookValidationError::TooManyEntries);
        }
        let mut orders = BTreeSet::new();
        for entry in &self.entries {
            if entry.title.trim().is_empty()
                || entry.text.trim().is_empty()
                || entry.scene.trim().is_empty()
                || entry
                    .attitude
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                || entry
                    .owner
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                || entry
                    .author
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
            {
                return Err(NotebookValidationError::EmptyField);
            }
            if entry.title.len() > MAX_NOTEBOOK_TITLE_BYTES
                || entry.text.len() > MAX_NOTEBOOK_TEXT_BYTES
                || entry.scene.len() > MAX_NOTEBOOK_SCENE_BYTES
                || entry
                    .attitude
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_NOTEBOOK_ATTITUDE_BYTES)
                || entry
                    .owner
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_NOTEBOOK_HERO_BYTES)
                || entry
                    .author
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_NOTEBOOK_HERO_BYTES)
            {
                return Err(NotebookValidationError::FieldTooLong);
            }
            if entry.order == 0 || !orders.insert(entry.order) {
                return Err(NotebookValidationError::InvalidOrder);
            }
            if entry.owner.is_some() && entry.kind != NotebookKind::Clue
                || entry.attitude.is_some() && entry.kind != NotebookKind::Person
                || entry.author.is_some()
                    && !matches!(entry.kind, NotebookKind::Clue | NotebookKind::Note)
                || matches!(entry.kind, NotebookKind::Clue | NotebookKind::Note)
                    && entry.author.is_none()
            {
                return Err(NotebookValidationError::InvalidAudience);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_NOTEBOOK_ATTITUDE_BYTES, MAX_NOTEBOOK_ENTRIES, MAX_NOTEBOOK_HERO_BYTES,
        MAX_NOTEBOOK_SCENE_BYTES, MAX_NOTEBOOK_TEXT_BYTES, MAX_NOTEBOOK_TITLE_BYTES,
        NotebookEntryView, NotebookKind, NotebookValidationError, NotebookView,
    };

    // SRD 5.2.1: no page applies; these tests validate narrative view fields, not game rules.
    fn entry() -> NotebookEntryView {
        NotebookEntryView {
            kind: NotebookKind::Person,
            title: "Tomas Greaves".to_owned(),
            text: "Mapmaker at the tea house.".to_owned(),
            scene: "opening".to_owned(),
            order: 1,
            attitude: Some("friendly".to_owned()),
            owner: None,
            author: None,
        }
    }

    #[test]
    fn notebook_view_accepts_bounded_entries_and_rejects_empty_text() {
        assert!(
            NotebookView {
                entries: vec![entry()]
            }
            .validate()
            .is_ok()
        );
        let mut invalid = entry();
        invalid.text.clear();
        assert_eq!(
            NotebookView {
                entries: vec![invalid]
            }
            .validate(),
            Err(NotebookValidationError::EmptyField)
        );
    }

    #[test]
    fn notebook_view_rejects_oversized_lists_and_strings() {
        let too_many = NotebookView {
            entries: (0..=MAX_NOTEBOOK_ENTRIES)
                .map(|order| NotebookEntryView {
                    order: order as u64 + 1,
                    ..entry()
                })
                .collect(),
        };
        assert_eq!(
            too_many.validate(),
            Err(NotebookValidationError::TooManyEntries)
        );
        let mut oversized = entry();
        oversized.text = "x".repeat(MAX_NOTEBOOK_TEXT_BYTES + 1);
        assert_eq!(
            NotebookView {
                entries: vec![oversized]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        let mut oversized_title = entry();
        oversized_title.title = "x".repeat(MAX_NOTEBOOK_TITLE_BYTES + 1);
        assert_eq!(
            NotebookView {
                entries: vec![oversized_title]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        let mut oversized_scene = entry();
        oversized_scene.scene = "x".repeat(MAX_NOTEBOOK_SCENE_BYTES + 1);
        assert_eq!(
            NotebookView {
                entries: vec![oversized_scene]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        let mut oversized_attitude = entry();
        oversized_attitude.attitude = Some("x".repeat(MAX_NOTEBOOK_ATTITUDE_BYTES + 1));
        assert_eq!(
            NotebookView {
                entries: vec![oversized_attitude]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        let mut oversized_owner = NotebookEntryView {
            kind: NotebookKind::Clue,
            title: "Private clue".to_owned(),
            text: "Clue text".to_owned(),
            scene: "opening".to_owned(),
            order: 1,
            attitude: None,
            owner: Some("x".repeat(MAX_NOTEBOOK_HERO_BYTES + 1)),
            author: Some("Brakka".to_owned()),
        };
        assert_eq!(
            NotebookView {
                entries: vec![oversized_owner.clone()]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        oversized_owner.owner = None;
        oversized_owner.author = Some("x".repeat(MAX_NOTEBOOK_HERO_BYTES + 1));
        assert_eq!(
            NotebookView {
                entries: vec![oversized_owner]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
    }

    // SRD 5.2.1 p. N/A: duplicate notebook ordering is view integrity, not a game rule.
    #[test]
    fn notebook_view_rejects_duplicate_orders() {
        let mut duplicate = entry();
        duplicate.title = "Jorun Greaves".to_owned();
        assert_eq!(
            NotebookView {
                entries: vec![entry(), duplicate]
            }
            .validate(),
            Err(NotebookValidationError::InvalidOrder)
        );
    }

    #[test]
    fn notebook_view_bounds_hero_specific_attitude_to_people() {
        let mut invalid = entry();
        invalid.attitude = Some("f".repeat(MAX_NOTEBOOK_ATTITUDE_BYTES + 1));
        assert_eq!(
            NotebookView {
                entries: vec![invalid]
            }
            .validate(),
            Err(NotebookValidationError::FieldTooLong)
        );
        let mut invalid = entry();
        invalid.kind = NotebookKind::Place;
        invalid.attitude = Some("friendly".to_owned());
        assert_eq!(
            NotebookView {
                entries: vec![invalid]
            }
            .validate(),
            Err(NotebookValidationError::InvalidAudience)
        );
    }
}
