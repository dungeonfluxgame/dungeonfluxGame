//! The party portrait strip (component 6): a card per hero and the open seats, tall full-bleed
//! portraits in one glass bar (render 13) or compact glass cards standing apart, each with the
//! class line and a glowing health bar in the hero's accent (render 02).
//!
//! Every rule of `overlay::rules` stays: every party member is drawn (K.13) with their look when
//! the scene gave one, the class line, the bar's fill and its tier. This adds the two layouts.

use serde::{Deserialize, Serialize};

use crate::fixture::PartyMember;
use crate::kit::{Fill, shown};
use crate::overlay::rules::{
    HealthTier, StripCard, bar_percent, class_line, health_tier, strip_cards,
};
use crate::overlay::{Glyph, SeatStrip};

/// More cards than this share the bar in the compact layout, dense: one row with no class line.
/// How a name or class wraps inside a card is the stylesheet's, never a letter count.
pub const TALL_CARDS_AT_MOST: usize = 6;
/// The condition chips a tall card shows in its top-right corner, over the portrait, before a
/// "+N" chip counts the rest.
pub const TALL_CONDITIONS_AT_MOST: usize = 2;
/// The condition chips a compact card shows: one line on its top rim, which a second chip at the
/// text floor would not fit beside a "+N" chip.
pub const COMPACT_CONDITIONS_AT_MOST: usize = 1;

/// How the cards are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyLayout {
    /// Full-bleed portraits with the name and class over a foot gradient (render 13).
    Tall,
    /// A square portrait beside the name, class line and hit points (render 02).
    Compact,
}

impl PartyLayout {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Tall => "tall",
            Self::Compact => "compact",
        }
    }
}

/// The strip.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartyStrip {
    pub party: Vec<PartyMember>,
    pub seats: Option<SeatStrip>,
    /// Tall up to six cards and compact past them when left out.
    pub layout: Option<PartyLayout>,
    /// Whether "HP 42/42" is written beside the bar.
    pub show_hp_text: bool,
    /// Whether the cards share one glass bar (render 13) or stand apart as glass cards of their
    /// own (render 02). The tall layout is framed and the compact one loose when left out.
    pub framed: Option<bool>,
    /// The positions in `party` (counted from 0) of the heroes the host shows downed, from the
    /// condition it projected: their portraits lose their colour. A position, not a display name,
    /// so two heroes of one name are never confused. The page never infers it from the hit points.
    pub downed: Vec<usize>,
}

/// What one hero's card shows.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroCard<'a> {
    pub name: &'a str,
    /// The class, without the level.
    pub class_name: Option<String>,
    /// "Lv. 3", kept whole on its line.
    pub level: Option<String>,
    pub glyph: Option<Glyph>,
    pub portrait: Option<&'a str>,
    /// A `#rrggbb` colour from the scene; the draw passes it through `style::css_color`.
    pub accent: Option<&'a str>,
    pub hit_points_text: String,
    pub bar_percent: f32,
    pub tier: HealthTier,
    /// Shown downed by the host: the portrait is drawn without colour.
    pub downed: bool,
    pub conditions: &'a [String],
}

/// The condition chips a card draws: the first conditions, and a "+N" chip that counts the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionChips<'a> {
    pub shown: Vec<&'a str>,
    pub more: Option<String>,
}

impl<'a> HeroCard<'a> {
    /// Every condition the host projected that has words, in its order.
    pub fn conditions(&self) -> Vec<&'a str> {
        self.conditions
            .iter()
            .filter_map(|condition| shown(Some(condition)))
            .collect()
    }

    /// The first `at_most` conditions and "+N" for the rest, when there are more.
    pub fn condition_chips(&self, at_most: usize) -> ConditionChips<'a> {
        let conditions = self.conditions();
        let hidden = conditions.len().saturating_sub(at_most);
        ConditionChips {
            shown: conditions.into_iter().take(at_most).collect(),
            more: (hidden > 0).then(|| format!("+{hidden}")),
        }
    }
}

/// One card of the strip.
#[derive(Debug, Clone, PartialEq)]
pub enum KitCard<'a> {
    Hero(HeroCard<'a>),
    Open,
}

impl PartyStrip {
    pub fn layout(&self) -> PartyLayout {
        self.layout.unwrap_or_else(|| {
            let count = self.party.len()
                + self
                    .seats
                    .as_ref()
                    .map_or(0, |seats| usize::from(seats.open));
            if count > TALL_CARDS_AT_MOST {
                PartyLayout::Compact
            } else {
                PartyLayout::Tall
            }
        })
    }

    pub fn framed(&self) -> bool {
        self.framed
            .unwrap_or_else(|| self.layout() == PartyLayout::Tall)
    }

    /// The bar is one large blurring pane; loose cards are each a small pane of fine glass.
    pub fn fill(&self) -> Fill {
        if self.framed() {
            Fill::Blurred
        } else {
            Fill::Fine
        }
    }

    /// How many condition chips a card shows before its "+N" chip.
    pub fn conditions_at_most(&self) -> usize {
        match self.layout() {
            PartyLayout::Tall => TALL_CONDITIONS_AT_MOST,
            PartyLayout::Compact => COMPACT_CONDITIONS_AT_MOST,
        }
    }

    /// Past six cards the strip keeps one row by dropping the class line and the hit-point
    /// words, so a full table never takes a second row of the screen.
    pub fn is_dense(&self) -> bool {
        self.cards().len() > TALL_CARDS_AT_MOST
    }

    /// The cards in order: every hero, then the open seats.
    pub fn cards(&self) -> Vec<KitCard<'_>> {
        strip_cards(&self.party, self.seats.as_ref())
            .into_iter()
            .enumerate()
            .map(|(position, card)| match card {
                StripCard::Open => KitCard::Open,
                StripCard::Hero(member, look) => KitCard::Hero(HeroCard {
                    name: &member.name,
                    class_name: look.and_then(|look| class_line(look.class.as_deref(), None)),
                    level: look.and_then(|look| class_line(None, look.level)),
                    glyph: look.and_then(|look| look.glyph),
                    portrait: look.and_then(|look| look.portrait.as_deref()),
                    accent: look
                        .and_then(|look| look.accent.as_deref())
                        .or(member.color.as_deref()),
                    hit_points_text: format!(
                        "HP {}/{}",
                        member.hit_points, member.maximum_hit_points
                    ),
                    bar_percent: bar_percent(member.hit_points, member.maximum_hit_points),
                    tier: health_tier(member.hit_points, member.maximum_hit_points),
                    downed: self.downed.contains(&position),
                    conditions: &member.conditions,
                }),
            })
            .collect()
    }
}
