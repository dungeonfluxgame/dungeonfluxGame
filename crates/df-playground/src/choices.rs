//! What a phone's choice list shows, and the contract warnings an author should see.
//!
//! It is the choice-list rule of the plan (section K.14) as code: at most four suggested choices,
//! then fixed groups, short labels, and unavailable choices hidden unless asked for. It never
//! draws and never changes a fixture: it returns a view of it plus every way the fixture breaks
//! the rule, so the playground can show the breaks instead of silently fixing them.

use std::collections::BTreeSet;
use std::fmt;

use crate::fixture::{Choice, ChoiceGroup, PhoneScene, UnavailableChoice};

/// Most suggested choices a phone shows above the groups.
pub const MAXIMUM_SUGGESTED: usize = 4;
/// Most words in a choice label.
pub const MAXIMUM_LABEL_WORDS: usize = 4;
/// The fixed groups, in the order they are shown.
pub const GROUP_NAMES: [&str; 6] = ["Attack", "Cast", "Move", "Use", "Talk", "Look"];

/// One way a fixture breaks the choice-list rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    TooManySuggested { given: usize },
    LabelTooLong { label: String, words: usize },
    UnknownGroup { name: String },
    DuplicateGroup { name: String },
    EmptyGroup { name: String },
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySuggested { given } => write!(
                formatter,
                "{given} suggested choices; only the first {MAXIMUM_SUGGESTED} are shown"
            ),
            Self::LabelTooLong { label, words } => write!(
                formatter,
                "'{label}' has {words} words; a label is at most {MAXIMUM_LABEL_WORDS}"
            ),
            Self::UnknownGroup { name } => write!(
                formatter,
                "'{name}' is not a group; the groups are {}",
                GROUP_NAMES.join(", ")
            ),
            Self::DuplicateGroup { name } => write!(formatter, "group '{name}' is listed twice"),
            Self::EmptyGroup { name } => write!(formatter, "group '{name}' has no choices"),
        }
    }
}

/// The choice list as the phone draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceListView<'a> {
    pub suggested: &'a [Choice],
    /// How many suggested choices were left out because of the maximum.
    pub left_out: usize,
    /// Non-empty groups, the fixed ones first in their fixed order, then any unknown ones.
    pub groups: Vec<&'a ChoiceGroup>,
    /// Empty unless the player asked to see unavailable choices.
    pub unavailable: &'a [UnavailableChoice],
    pub warnings: Vec<Warning>,
}

fn label_words(label: &str) -> usize {
    label.split_whitespace().count()
}

fn group_rank(name: &str) -> usize {
    GROUP_NAMES
        .iter()
        .position(|group| *group == name)
        .unwrap_or(GROUP_NAMES.len())
}

/// Builds the view of a phone's choices and collects the warnings.
pub fn build(scene: &PhoneScene, show_unavailable: bool) -> ChoiceListView<'_> {
    let mut warnings = Vec::new();
    if scene.suggested.len() > MAXIMUM_SUGGESTED {
        warnings.push(Warning::TooManySuggested {
            given: scene.suggested.len(),
        });
    }
    let shown = scene.suggested.len().min(MAXIMUM_SUGGESTED);

    let all_labels = scene
        .suggested
        .iter()
        .map(|choice| &choice.label)
        .chain(
            scene
                .groups
                .iter()
                .flat_map(|group| group.choices.iter().map(|choice| &choice.label)),
        )
        .chain(scene.unavailable.iter().map(|choice| &choice.label));
    for label in all_labels {
        let words = label_words(label);
        if words > MAXIMUM_LABEL_WORDS {
            warnings.push(Warning::LabelTooLong {
                label: label.clone(),
                words,
            });
        }
    }

    let mut seen = BTreeSet::new();
    let mut groups = Vec::new();
    for group in &scene.groups {
        if !seen.insert(group.name.as_str()) {
            warnings.push(Warning::DuplicateGroup {
                name: group.name.clone(),
            });
            continue;
        }
        if group_rank(&group.name) == GROUP_NAMES.len() {
            warnings.push(Warning::UnknownGroup {
                name: group.name.clone(),
            });
        }
        if group.choices.is_empty() {
            warnings.push(Warning::EmptyGroup {
                name: group.name.clone(),
            });
            continue;
        }
        groups.push(group);
    }
    groups.sort_by_key(|group| group_rank(&group.name));

    ChoiceListView {
        suggested: &scene.suggested[..shown],
        left_out: scene.suggested.len() - shown,
        groups,
        unavailable: if show_unavailable {
            &scene.unavailable
        } else {
            &[]
        },
        warnings,
    }
}
