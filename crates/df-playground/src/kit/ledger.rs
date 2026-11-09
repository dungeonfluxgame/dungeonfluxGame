//! The list panel of the right column: the loot or chest contents list (component 17) and the shop
//! price list (component 18), one widget with two row kinds (renders 07, 17, 20). A gold gradient
//! title over a diamond rule, then rows of an icon or picture, the name tinted by rarity, and the
//! quantity or a dotted leader and the price.
//!
//! The quantity and the price are the server's words; the page never works a figure out.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::{Fill, Rarity, shown};

/// Which rows the list has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    /// Found things with a picture or icon and a quantity.
    #[default]
    Loot,
    /// Goods with a dotted leader and a price.
    Shop,
}

impl LedgerKind {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Loot => "loot",
            Self::Shop => "shop",
        }
    }
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerItem {
    pub id: String,
    pub name: String,
    /// "×2", "(37)": a loot row's count, as the server worded it.
    #[serde(default)]
    pub quantity: Option<String>,
    /// "300 gp": a shop row's price, as the server worded it.
    #[serde(default)]
    pub price: Option<String>,
    /// A picture from the media library, drawn instead of the icon.
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub icon: Option<HomeIcon>,
    #[serde(default)]
    pub rarity: Option<Rarity>,
    /// Taken by a hero: the row fades and shows a check.
    #[serde(default)]
    pub taken: bool,
    /// The row under discussion, in a lit rim.
    #[serde(default)]
    pub highlight: bool,
}

/// The list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub icon: Option<HomeIcon>,
    #[serde(default)]
    pub kind: LedgerKind,
    pub items: Vec<LedgerItem>,
    /// Whether the rows sit on a glass panel; `false` floats them on the plate (render 20).
    #[serde(default = "framed")]
    pub framed: bool,
}

fn framed() -> bool {
    true
}

/// A row as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerRow<'a> {
    pub item: &'a LedgerItem,
    pub name: &'a str,
    /// The quantity of a loot row or the price of a shop row.
    pub figure: Option<&'a str>,
    pub order: usize,
}

impl Ledger {
    pub fn title(&self) -> Option<&str> {
        shown(self.title.as_deref())
    }

    /// The rows that have a name, in the server's order, each with the figure of its kind.
    pub fn rows(&self) -> Vec<LedgerRow<'_>> {
        self.items
            .iter()
            .filter_map(|item| shown(Some(&item.name)).map(|name| (item, name)))
            .enumerate()
            .map(|(order, (item, name))| LedgerRow {
                item,
                name,
                figure: match self.kind {
                    LedgerKind::Loot => shown(item.quantity.as_deref()),
                    LedgerKind::Shop => shown(item.price.as_deref()),
                },
                order,
            })
            .collect()
    }

    /// A framed list blurs the plate; a floating one has no glass.
    pub fn fill(&self) -> Fill {
        if self.framed {
            Fill::Blurred
        } else {
            Fill::Solid
        }
    }

    /// A list without rows is not drawn.
    pub fn is_drawn(&self) -> bool {
        !self.rows().is_empty()
    }
}
