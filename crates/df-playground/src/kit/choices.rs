//! The story choice panel (component 11): a prompt over glass buttons in rows (renders 07, 22,
//! 32) or cards with a description (render 04). A lit choice glows gold, the others are ice; once
//! one is chosen, the rest dim.
//!
//! Every look is the server's: the TV never ranks or picks a choice. This is the TV drawing of the
//! same projected list `crate::choices` builds for the phone (`ChoicePanel::from_list`).

use serde::{Deserialize, Serialize};

use crate::choices::ChoiceListView;
use crate::home::glyph::HomeIcon;
use crate::kit::{Fill, shown};

/// How the choices are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceLayout {
    /// Elongated hexagons stacked under the prompt.
    #[default]
    Rows,
    /// Octagon cards side by side, each with a description line.
    Cards,
}

impl ChoiceLayout {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Cards => "cards",
        }
    }
}

/// How one choice looks, as the server projected it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceLook {
    #[default]
    Idle,
    /// The leading or suggested choice.
    Lit,
    /// Taken by the table; the panel is locked.
    Chosen,
    /// Shown, but cannot be taken now.
    Unavailable,
}

impl ChoiceLook {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Lit => "lit",
            Self::Chosen => "chosen",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KitChoice {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<HomeIcon>,
    #[serde(default)]
    pub look: ChoiceLook,
    /// How many seats lean this way, drawn as pips.
    #[serde(default)]
    pub votes: Option<u8>,
}

/// The panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePanel {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub layout: ChoiceLayout,
    pub choices: Vec<KitChoice>,
}

/// A choice as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawnChoice<'a> {
    pub choice: &'a KitChoice,
    pub label: &'a str,
    pub description: Option<&'a str>,
    /// Another choice was chosen, so this one gives way.
    pub dimmed: bool,
    /// Its place in the stagger of the entrance, counted from 0.
    pub order: usize,
}

impl ChoicePanel {
    /// The panel of the list `crate::choices::build` projected, under `prompt`, in the list's
    /// order: the suggested choices lit, then each group's choices, then the unavailable ones the
    /// list carries, which cannot be taken. A choice's explanation, or the reason it cannot be
    /// taken, is its description. The ids name each choice's place in the list.
    pub fn from_list(list: &ChoiceListView<'_>, prompt: Option<&str>) -> Self {
        let suggested = list
            .suggested
            .iter()
            .enumerate()
            .map(|(index, choice)| KitChoice {
                id: format!("suggested-{index}"),
                label: choice.label.clone(),
                description: choice.explain.clone(),
                icon: None,
                look: ChoiceLook::Lit,
                votes: None,
            });
        let grouped = list.groups.iter().flat_map(|group| {
            group
                .choices
                .iter()
                .enumerate()
                .map(move |(index, choice)| KitChoice {
                    id: format!("{}-{index}", group.name),
                    label: choice.label.clone(),
                    description: choice.explain.clone(),
                    icon: None,
                    look: ChoiceLook::Idle,
                    votes: None,
                })
        });
        let unavailable = list
            .unavailable
            .iter()
            .enumerate()
            .map(|(index, choice)| KitChoice {
                id: format!("unavailable-{index}"),
                label: choice.label.clone(),
                description: Some(choice.reason.clone()),
                icon: None,
                look: ChoiceLook::Unavailable,
                votes: None,
            });
        Self {
            prompt: prompt.map(str::to_owned),
            layout: ChoiceLayout::Rows,
            choices: suggested.chain(grouped).chain(unavailable).collect(),
        }
    }

    pub fn prompt(&self) -> Option<&str> {
        shown(self.prompt.as_deref())
    }

    /// Each choice is a small pane of fine glass.
    pub fn fill(&self) -> Fill {
        Fill::Fine
    }

    /// Whether every choice keeps an icon column: when any drawn choice has an icon, so the
    /// labels of one list start in one line whether or not their own choice has one.
    pub fn reserves_icon_column(&self) -> bool {
        self.drawn().iter().any(|drawn| drawn.choice.icon.is_some())
    }

    /// The panel's CSS class: its layout, and `kit-choices-iconed` when it keeps an icon column,
    /// which starts every row's label at one x after the icons instead of centring it.
    pub fn class(&self) -> String {
        let iconed = if self.reserves_icon_column() {
            " kit-choices-iconed"
        } else {
            ""
        };
        format!("kit-choices kit-choices-{}{iconed}", self.layout.key())
    }

    /// The choices that have a label, in the server's order.
    pub fn drawn(&self) -> Vec<DrawnChoice<'_>> {
        let locked = self
            .choices
            .iter()
            .any(|choice| choice.look == ChoiceLook::Chosen);
        self.choices
            .iter()
            .filter_map(|choice| shown(Some(&choice.label)).map(|label| (choice, label)))
            .enumerate()
            .map(|(order, (choice, label))| DrawnChoice {
                choice,
                label,
                description: shown(choice.description.as_deref()),
                dimmed: locked && choice.look != ChoiceLook::Chosen,
                order,
            })
            .collect()
    }
}
