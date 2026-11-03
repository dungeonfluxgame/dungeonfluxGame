//! The cost view of the plate interval: plates per hour and what an hour of them costs (D18, RES9,
//! H.8).
//!
//! The interval is a knob with a price, so the price is shown where the knob is. The view is
//! computed from the interval and from the price and the calls of each stage a plate uses (the
//! plate and the Locate passes), and it is an upper bound: a quiet table pays for no plate at all
//! (RES9), and Locate may stop after the first phrase of an item or run to the third, so the calls
//! of a stage are a range. A stage whose lane records no price (a plate through the Codex
//! subscription) makes the figure partial: the view then says so and shows calls per hour for that
//! stage instead of a dollar amount, since "n/a" would hide what is spent (H.8).
//!
//! It shows recorded prices and never limits anything: the ledger records and never blocks
//! (coding style section 8, pattern 4). Money is whole micro-dollars. It reads no clock and no
//! price file: the host hands each stage's price in from the broker's purposes.

use serde::{Deserialize, Serialize};

use crate::plate_timing::PlateTiming;
use crate::ports::PurposeName;

/// What one call of a stage costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "price", rename_all = "snake_case")]
pub enum StagePrice {
    Priced { micro_dollars: u64 },
    Unpriced,
}

/// One stage a plate uses: its purpose, its price and how many calls one plate makes of it, as a
/// least and a most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageUse {
    pub purpose: PurposeName,
    pub price: StagePrice,
    pub calls_least_per_plate: u32,
    pub calls_most_per_plate: u32,
}

/// What an hour of one stage comes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageHour {
    pub purpose: PurposeName,
    pub price: StagePrice,
    pub calls_least_per_hour: u64,
    pub calls_most_per_hour: u64,
    /// Absent for an unpriced stage.
    pub micro_dollars_least_per_hour: Option<u64>,
    pub micro_dollars_most_per_hour: Option<u64>,
}

/// Whether every stage has a price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// Every stage has a price: the dollar figures are the whole upper bound.
    AllStagesPriced,
    /// At least one stage has no price: the dollar figures leave it out, and its calls per hour
    /// are what the view can say.
    SomeStagesUnpriced,
}

impl CostBasis {
    pub fn key(self) -> &'static str {
        match self {
            Self::AllStagesPriced => "all_stages_priced",
            Self::SomeStagesUnpriced => "some_stages_unpriced",
        }
    }
}

/// Plates per hour and the cost per hour at the plate interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostView {
    pub interval_s: u32,
    /// The most plates an hour can need: an upper bound, since a quiet table pays for none.
    pub plates_per_hour_at_most: u32,
    pub stages: Vec<StageHour>,
    pub basis: CostBasis,
    /// The priced stages together, in micro-dollars an hour: least and most, both upper bounds on
    /// a quiet table.
    pub priced_micro_dollars_least_per_hour: u64,
    pub priced_micro_dollars_most_per_hour: u64,
}

impl CostView {
    /// The view for `timing` and the stages a plate uses.
    pub fn estimate(timing: PlateTiming, uses: &[StageUse]) -> Self {
        let plates = u64::from(timing.plates_per_hour_at_most());
        let stages: Vec<StageHour> = uses
            .iter()
            .map(|stage| {
                let least_calls = plates.saturating_mul(u64::from(stage.calls_least_per_plate));
                let most_calls = plates.saturating_mul(u64::from(stage.calls_most_per_plate));
                let dollars = |calls: u64| match stage.price {
                    StagePrice::Priced { micro_dollars } => {
                        Some(calls.saturating_mul(micro_dollars))
                    }
                    StagePrice::Unpriced => None,
                };
                StageHour {
                    purpose: stage.purpose.clone(),
                    price: stage.price,
                    calls_least_per_hour: least_calls,
                    calls_most_per_hour: most_calls,
                    micro_dollars_least_per_hour: dollars(least_calls),
                    micro_dollars_most_per_hour: dollars(most_calls),
                }
            })
            .collect();
        let basis = if stages
            .iter()
            .any(|stage| stage.price == StagePrice::Unpriced)
        {
            CostBasis::SomeStagesUnpriced
        } else {
            CostBasis::AllStagesPriced
        };
        let sum = |pick: fn(&StageHour) -> Option<u64>| {
            stages
                .iter()
                .filter_map(pick)
                .fold(0_u64, u64::saturating_add)
        };
        Self {
            interval_s: timing.plate_interval_s(),
            plates_per_hour_at_most: timing.plates_per_hour_at_most(),
            priced_micro_dollars_least_per_hour: sum(|stage| stage.micro_dollars_least_per_hour),
            priced_micro_dollars_most_per_hour: sum(|stage| stage.micro_dollars_most_per_hour),
            stages,
            basis,
        }
    }
}
