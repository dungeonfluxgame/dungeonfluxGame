//! The price table of the cost engine: dated, sourced rates by provider, model, category and
//! context tier, read from `content/pricing.json` (schema `dungeonflux.pricing/1`).
//!
//! The row validation and the exact arithmetic are `df_commerce::tariff`'s, so the cost book and
//! the money core read one table the same way; this module adds the game's closed provider and
//! category vocabularies, the file format and the context tiers.
//!
//! # Context tiers
//!
//! A row may carry `context_max_tokens`: the largest context (input tokens of the call) the row
//! prices. A row without it (or with 0) prices any context. A call is priced by the smallest tier
//! that covers its context and holds a row for the category; a context above every stored tier of
//! a category is unpriced, never priced at a rate that does not cover it.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use df_commerce::{
    rounding::Ratio,
    tariff::{self, RawPriceRow, TariffError},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Category, CostLine, Provider, word};

/// The price table's schema name; a file with any other is refused.
pub(in crate::gameplay) const PRICING_SCHEMA: &str = "dungeonflux.pricing/1";

/// Why a price file was refused. It names the entry, never the file's other contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PricingError {
    Unreadable,
    Json(String),
    Schema,
    Currency,
    Entry { index: usize, reason: &'static str },
    Duplicate { index: usize },
}

impl PricingError {
    pub(in crate::gameplay) fn reason(&self) -> String {
        match self {
            Self::Unreadable => "unreadable".to_owned(),
            Self::Json(detail) => format!("json:{}", word(detail)),
            Self::Schema => format!("schema_not_{PRICING_SCHEMA}"),
            Self::Currency => "currency_not_usd".to_owned(),
            Self::Entry { index, reason } => format!("entry_{index}_{reason}"),
            Self::Duplicate { index } => format!("entry_{index}_duplicate"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceFile {
    schema: String,
    currency: String,
    prices: Vec<PriceEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceEntry {
    provider: String,
    model: String,
    category: String,
    usd: String,
    per: u64,
    as_of: String,
    source: String,
    /// The largest context in tokens this row prices; 0 or absent prices any context.
    #[serde(default)]
    context_max_tokens: u64,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "a note for the owner who edits the file; never read"
    )]
    note: String,
}

/// Whole nano-dollars as the shortest exact dollar text (`0.042`, `5`).
pub(super) fn nano_usd_text(nanos: u64) -> String {
    let fraction = format!("{:09}", nanos % 1_000_000_000);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        (nanos / 1_000_000_000).to_string()
    } else {
        format!("{}.{fraction}", nanos / 1_000_000_000)
    }
}

/// The key a context tier is stored under: an unbounded row sorts last.
const fn tier_key(context_max_tokens: u64) -> u64 {
    if context_max_tokens == 0 {
        u64::MAX
    } else {
        context_max_tokens
    }
}

/// The input tokens of a call, which is the context its tier is chosen by: every input category,
/// cached and cache-written included.
pub(in crate::gameplay) fn context_tokens(quantities: &[(Category, u64)]) -> u64 {
    quantities
        .iter()
        .filter(|(category, _)| {
            matches!(
                category,
                Category::TextInput
                    | Category::TextCachedInput
                    | Category::TextCacheWrite
                    | Category::ImageInput
                    | Category::ImageCachedInput
            )
        })
        .fold(0_u64, |sum, (_, quantity)| sum.saturating_add(*quantity))
}

/// The dated prices, by provider, model, category and context tier. A lookup is a pure read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct PriceTable {
    /// One tariff table per context tier, the tier's largest context as the key (see
    /// [`tier_key`]); the iteration order is the lookup order.
    tiers: BTreeMap<u64, tariff::PriceTable>,
}

impl PriceTable {
    /// Parses and validates a whole price file; any malformed entry refuses the whole file.
    pub(in crate::gameplay) fn parse(text: &str) -> Result<Self, PricingError> {
        let file: PriceFile = serde_json::from_str(text).map_err(|error| {
            PricingError::Json(format!("line_{}_column_{}", error.line(), error.column()))
        })?;
        if file.schema != PRICING_SCHEMA {
            return Err(PricingError::Schema);
        }
        if file.currency != "USD" {
            return Err(PricingError::Currency);
        }
        let mut by_tier: BTreeMap<u64, Vec<(usize, RawPriceRow<'_>)>> = BTreeMap::new();
        for (index, entry) in file.prices.iter().enumerate() {
            let refuse = |reason| PricingError::Entry { index, reason };
            if Provider::parse(&entry.provider).is_none() {
                return Err(refuse("provider"));
            }
            if Category::parse(&entry.category).is_none() {
                return Err(refuse("category"));
            }
            let row = RawPriceRow {
                provider: &entry.provider,
                model: &entry.model,
                category: &entry.category,
                usd: &entry.usd,
                per: entry.per,
                as_of: &entry.as_of,
                source: &entry.source,
            };
            // The first malformed entry in file order is the one named.
            tariff::PriceTable::from_rows(&[row]).map_err(|error| match error {
                TariffError::Entry { reason, .. } => refuse(reason),
                TariffError::Duplicate { .. } => refuse("row"),
            })?;
            by_tier
                .entry(tier_key(entry.context_max_tokens))
                .or_default()
                .push((index, row));
        }
        let mut tiers = BTreeMap::new();
        for (tier, entries) in by_tier {
            let rows: Vec<RawPriceRow<'_>> = entries.iter().map(|(_, row)| *row).collect();
            let table = tariff::PriceTable::from_rows(&rows).map_err(|error| match error {
                TariffError::Duplicate { index } => PricingError::Duplicate {
                    index: entries[index].0,
                },
                TariffError::Entry { index, reason } => PricingError::Entry {
                    index: entries[index].0,
                    reason,
                },
            })?;
            tiers.insert(tier, table);
        }
        Ok(Self { tiers })
    }

    /// The rate of one line for a call of `context` input tokens, if the table has it.
    pub(in crate::gameplay) fn lookup(
        &self,
        provider: Provider,
        model: &str,
        category: Category,
        context: u64,
    ) -> Option<&tariff::Price> {
        self.tiers
            .iter()
            .filter(|(tier, _)| context <= **tier)
            .find_map(|(_, table)| table.lookup(provider.label(), model, category.label()))
    }

    /// The rows of the table, tiers included.
    pub(in crate::gameplay) fn len(&self) -> usize {
        self.tiers.values().map(tariff::PriceTable::len).sum()
    }

    /// The exact price of `quantity` units of one line, in micro-dollars, unrounded.
    pub(in crate::gameplay) fn exact(
        &self,
        provider: Provider,
        model: &str,
        category: Category,
        quantity: u64,
        context: u64,
    ) -> Option<Ratio> {
        self.lookup(provider, model, category, context)
            .map(|price| price.micro_dollars(quantity))
    }

    /// The price of a whole call in micro-dollars: the exact lines summed, then rounded half up
    /// once. `None` when any line has no rate, so a quote is never a partial price.
    pub(in crate::gameplay) fn quote(
        &self,
        provider: Provider,
        model: &str,
        quantities: &[(Category, u64)],
    ) -> Option<u64> {
        let context = context_tokens(quantities);
        let mut total = Ratio {
            numerator: 0,
            denominator: 1,
        };
        for (category, quantity) in quantities {
            let line = self.exact(provider, model, *category, *quantity, context)?;
            total = total.checked_add(line).ok()?;
        }
        total.round_half_up().ok()
    }

    /// Every rate with its date, source and tier, for `dev cost`.
    pub(in crate::gameplay) fn json(&self) -> Value {
        let mut rows: Vec<(u64, &str, &str, &str, &tariff::Price)> = self
            .tiers
            .iter()
            .flat_map(|(tier, table)| {
                table.rows().map(move |(provider, model, category, price)| {
                    (*tier, provider, model, category, price)
                })
            })
            .collect();
        rows.sort_by_key(|(tier, provider, model, category, _)| {
            (*provider, *model, *category, *tier)
        });
        rows.into_iter()
            .map(|(tier, provider, model, category, price)| {
                json!({
                    "provider": provider,
                    "model": model,
                    "category": category,
                    "usd": nano_usd_text(price.nano_usd),
                    "per": price.per,
                    "context_max_tokens": if tier == u64::MAX { 0 } else { tier },
                    "as_of": price.as_of,
                    "source": price.source,
                })
            })
            .collect()
    }
}

/// How the price file was read at startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PricingStatus {
    /// No book has been opened yet.
    NotLoaded,
    Loaded {
        path: PathBuf,
        entries: usize,
    },
    /// No file: every line is unpriced.
    Missing {
        path: PathBuf,
    },
    /// A malformed file is refused whole: every line is unpriced.
    Refused {
        path: PathBuf,
        reason: String,
    },
}

impl PricingStatus {
    pub(super) fn json(&self) -> Value {
        match self {
            Self::NotLoaded => json!({"status": "not_loaded"}),
            Self::Loaded { path, entries } => {
                json!({"status": "loaded", "path": path.display().to_string(), "entries": entries})
            }
            Self::Missing { path } => {
                json!({"status": "missing", "path": path.display().to_string()})
            }
            Self::Refused { path, reason } => json!({
                "status": "refused",
                "path": path.display().to_string(),
                "reason": reason,
            }),
        }
    }
}

/// Reads the price file at `path`. Never fails: a missing or refused file gives an empty table.
pub(in crate::gameplay) fn load_prices(path: &Path) -> (PriceTable, PricingStatus) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (
                PriceTable::default(),
                PricingStatus::Missing {
                    path: path.to_owned(),
                },
            );
        }
        Err(_) => {
            return (
                PriceTable::default(),
                PricingStatus::Refused {
                    path: path.to_owned(),
                    reason: PricingError::Unreadable.reason(),
                },
            );
        }
    };
    match PriceTable::parse(&text) {
        Ok(table) => {
            let entries = table.len();
            (
                table,
                PricingStatus::Loaded {
                    path: path.to_owned(),
                    entries,
                },
            )
        }
        Err(error) => (
            PriceTable::default(),
            PricingStatus::Refused {
                path: path.to_owned(),
                reason: error.reason(),
            },
        ),
    }
}

/// The lines of one call at `table`'s rates, rounded with the book's carried remainders: each
/// line's exact price is added to its carry, the result is rounded half up to whole
/// micro-dollars, and what is left over (below half a micro-dollar either way) is carried to the
/// next call of the same line. Over any run the rounded amounts sum to the exact total within
/// half a micro-dollar per line, not within half a micro-dollar per call.
pub(super) fn price_carrying(
    table: &PriceTable,
    carries: &mut BTreeMap<(Provider, String, Category), Carry>,
    provider: Provider,
    model: &str,
    quantities: &[(Category, u64)],
) -> (Vec<CostLine>, u64) {
    let context = context_tokens(quantities);
    let lines: Vec<CostLine> = quantities
        .iter()
        .map(|(category, quantity)| {
            let micro_usd = table
                .exact(provider, model, *category, *quantity, context)
                .map(|exact| {
                    let key = (provider, model.to_owned(), *category);
                    carries.entry(key).or_default().round(exact)
                });
            CostLine {
                category: *category,
                quantity: *quantity,
                micro_usd,
            }
        })
        .collect();
    let total = lines.iter().fold(0_u64, |sum, line| {
        sum.saturating_add(line.micro_usd.unwrap_or(0))
    });
    (lines, total)
}

/// The part of a line's exact price not yet paid out in whole micro-dollars: `remainder` over
/// `denominator`, in `[-denominator / 2, denominator / 2)`. A new rate (another denominator)
/// starts a new carry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Carry {
    remainder: i128,
    denominator: u128,
}

impl Carry {
    /// Rounds `exact` plus the carry half up to whole micro-dollars and keeps the rest.
    fn round(&mut self, exact: Ratio) -> u64 {
        if self.denominator != exact.denominator {
            *self = Self {
                remainder: 0,
                denominator: exact.denominator,
            };
        }
        let (Ok(denominator), Ok(numerator)) = (
            i128::try_from(exact.denominator),
            i128::try_from(exact.numerator),
        ) else {
            // Beyond any real quantity: no carry, saturate.
            *self = Self::default();
            return exact.round_half_up().unwrap_or(u64::MAX);
        };
        let Some(accumulated) = self.remainder.checked_add(numerator) else {
            *self = Self::default();
            return exact.round_half_up().unwrap_or(u64::MAX);
        };
        let Some(doubled) = accumulated
            .checked_mul(2)
            .and_then(|doubled| doubled.checked_add(denominator))
        else {
            *self = Self::default();
            return exact.round_half_up().unwrap_or(u64::MAX);
        };
        let rounded = doubled.div_euclid(denominator * 2);
        self.remainder = accumulated - rounded * denominator;
        u64::try_from(rounded).unwrap_or(u64::MAX)
    }
}
