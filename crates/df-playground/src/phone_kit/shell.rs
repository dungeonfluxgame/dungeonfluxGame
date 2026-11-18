//! The phone's frame: the header with the hero, the bottom tab bar, the pills, the health bar, the
//! buttons, the pop-up toast and the empty and loading states.
//!
//! Each type is what the host would send for the piece (every part is sample data today); the
//! decisions the drawing needs are the functions here, so native tests cover them.

use serde::{Deserialize, Serialize};

use crate::limits::PHONE_TAB_SLOTS;
use crate::overlay::rules::{HealthTier, bar_percent, health_tier};
use crate::phone_kit::glyph::PhoneGlyph;

/// The hero at the top of the screen: portrait, name and where they are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneHeader {
    pub name: String,
    /// The line under the name: the place, or the goal.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// A picture of the media library; the first letter of the name stands in without one.
    #[serde(default)]
    pub portrait: Option<String>,
    #[serde(default)]
    pub hp: Option<HpBar>,
}

/// A hero's hit points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HpBar {
    pub current: u32,
    pub maximum: u32,
    #[serde(default)]
    pub temporary: u32,
}

impl HpBar {
    /// The fill of the bar, in percent.
    pub fn percent(&self) -> f32 {
        bar_percent(self.current, self.maximum)
    }

    /// The share of the track the temporary points take after the fill, kept inside the track.
    pub fn temporary_percent(&self) -> f32 {
        if self.maximum == 0 {
            return 0.0;
        }
        let share = self.temporary as f32 * 100.0 / self.maximum as f32;
        share.min(100.0 - self.percent()).max(0.0)
    }

    /// The colour of the bar: green, amber below half, red below a quarter.
    pub fn tier(&self) -> HealthTier {
        health_tier(self.current, self.maximum)
    }

    /// The words of the bar: `24 / 27 HP`, with the temporary points when there are some.
    pub fn text(&self) -> String {
        if self.temporary == 0 {
            format!("{} / {} HP", self.current, self.maximum)
        } else {
            format!(
                "{} / {} HP (+{} temporary)",
                self.current, self.maximum, self.temporary
            )
        }
    }
}

/// A panel of the play screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKey {
    Story,
    Act,
    Talk,
    Roll,
    Inventory,
    Clues,
    Map,
    Hero,
    Notebook,
}

impl TabKey {
    /// Every panel, in the order of the bar.
    pub const ALL: [Self; 9] = [
        Self::Story,
        Self::Act,
        Self::Talk,
        Self::Roll,
        Self::Inventory,
        Self::Clues,
        Self::Map,
        Self::Hero,
        Self::Notebook,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Story => "Story",
            Self::Act => "Act",
            Self::Talk => "Talk",
            Self::Roll => "Roll",
            Self::Inventory => "Bag",
            Self::Clues => "Clues",
            Self::Map => "Map",
            Self::Hero => "Hero",
            Self::Notebook => "Notes",
        }
    }

    pub fn glyph(self) -> PhoneGlyph {
        match self {
            Self::Story => PhoneGlyph::Story,
            Self::Act => PhoneGlyph::Act,
            Self::Talk => PhoneGlyph::Talk,
            Self::Roll => PhoneGlyph::Roll,
            Self::Inventory => PhoneGlyph::Bag,
            Self::Clues => PhoneGlyph::Clues,
            Self::Map => PhoneGlyph::Map,
            Self::Hero => PhoneGlyph::Hero,
            Self::Notebook => PhoneGlyph::Notebook,
        }
    }

    /// The word that ends the CSS class, the same as the serde name.
    /// The page the tab opens, in the words of `nav::page_action`.
    pub fn page_key(self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Act => "act",
            Self::Talk => "talk",
            Self::Roll => "roll",
            Self::Inventory => "bag",
            Self::Clues => "clues",
            Self::Map => "map",
            Self::Hero => "hero",
            Self::Notebook => "notes",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Act => "act",
            Self::Talk => "talk",
            Self::Roll => "roll",
            Self::Inventory => "inventory",
            Self::Clues => "clues",
            Self::Map => "map",
            Self::Hero => "hero",
            Self::Notebook => "notebook",
        }
    }
}

/// The bottom bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabBar {
    /// The panel that is open; none when the page open is not one of the bar's (the creator, a
    /// fight), which lights "more".
    pub active: Option<TabKey>,
    /// The panels the bar offers, in order.
    pub tabs: Vec<TabKey>,
    /// A count on a tab: a roll waiting, a new note.
    #[serde(default)]
    pub badges: Vec<TabBadge>,
    /// Whether the "more" sheet is open.
    #[serde(default)]
    pub more_open: bool,
    /// The panels that work now; the others are drawn dimmed and do nothing. Empty means all.
    #[serde(default)]
    pub working: Vec<TabKey>,
    /// Pages the "more" sheet lists after the panels that do not fit.
    #[serde(default)]
    pub links: Vec<MoreLink>,
}

/// A page the "more" sheet reaches that is not one of the bar's panels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoreLink {
    pub label: String,
    /// What a tap asks for (`nav::page_action`).
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabBadge {
    pub tab: TabKey,
    pub count: u8,
    /// Something on the tab waits on the player (a question, a roll): a pulsing dot.
    #[serde(default)]
    pub waiting: bool,
}

/// Which tabs sit on the bar and which wait behind "more".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabLayout {
    pub bar: Vec<TabKey>,
    pub more: Vec<TabKey>,
}

impl TabBar {
    /// Up to `PHONE_TAB_SLOTS` tabs are all on the bar. More than that, the bar keeps the first
    /// ones and a "more" slot; a panel that is open but would sit behind "more" takes the last
    /// bar slot, so the bar always shows where the player is.
    pub fn layout(&self) -> TabLayout {
        if self.tabs.len() <= PHONE_TAB_SLOTS {
            return TabLayout {
                bar: self.tabs.clone(),
                more: Vec::new(),
            };
        }
        let on_bar = PHONE_TAB_SLOTS - 1;
        let mut bar: Vec<TabKey> = self.tabs[..on_bar].to_vec();
        let mut more: Vec<TabKey> = self.tabs[on_bar..].to_vec();
        if let Some(active) = self.active
            && let Some(position) = more.iter().position(|tab| *tab == active)
        {
            let displaced = std::mem::replace(&mut bar[on_bar - 1], active);
            more.remove(position);
            more.insert(0, displaced);
        }
        TabLayout { bar, more }
    }

    /// Whether something on `tab` waits on the player.
    pub fn waits(&self, tab: TabKey) -> bool {
        self.badges
            .iter()
            .any(|badge| badge.tab == tab && badge.waiting)
    }

    pub fn badge(&self, tab: TabKey) -> Option<u8> {
        self.badges
            .iter()
            .find(|badge| badge.tab == tab && badge.count > 0)
            .map(|badge| badge.count)
    }

    pub fn works(&self, tab: TabKey) -> bool {
        self.working.is_empty() || self.working.contains(&tab)
    }
}

/// How a pill is toned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillTone {
    #[default]
    Neutral,
    Good,
    Warn,
    Danger,
    Accent,
}

impl PillTone {
    pub fn key(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Good => "good",
            Self::Warn => "warn",
            Self::Danger => "danger",
            Self::Accent => "accent",
        }
    }
}

/// A small rounded mark: a status or a count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pill {
    pub text: String,
    #[serde(default)]
    pub tone: PillTone,
    #[serde(default)]
    pub glyph: Option<PhoneGlyph>,
}

/// How a button is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonKind {
    /// Brass and chamfered: the one thing the screen wants done.
    #[default]
    Primary,
    /// Glass.
    Secondary,
    Danger,
}

impl ButtonKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Danger => "danger",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonSpec {
    pub label: String,
    #[serde(default)]
    pub kind: ButtonKind,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Why the button is off, shown under it.
    #[serde(default)]
    pub reason: Option<String>,
    /// What a tap on it asks the page for (`data-action`); none for a sample.
    #[serde(default)]
    pub action: Option<String>,
}

fn enabled() -> bool {
    true
}

/// A pop-up the game raises: something worth keeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toast {
    /// What it is: "A name learned", "A new place".
    pub kind: String,
    pub text: String,
    pub keep_label: String,
}

/// What a panel shows when it has nothing yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyState {
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub glyph: Option<PhoneGlyph>,
}

/// What a panel shows while the host answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadingState {
    pub text: String,
}
