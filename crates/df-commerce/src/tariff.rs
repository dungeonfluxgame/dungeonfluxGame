//! The supplier price table and bill comparison, moved out of the cost engine so the money core
//! and the cost book share one parser (planning/finance-ledger-research.md 29.1 request to C).
//!
//! The validation, the exact arithmetic and the bill reader live here. Decoding the price file's
//! JSON text into [`RawPriceRow`]s is the native caller's step: this crate takes no JSON
//! dependency, because adding one would change the root lock file. TODO(FIN-10): when the lock is
//! free, add `serde_json` here and a `PriceTable::parse(text)` beside [`PriceTable::from_rows`].

use std::collections::BTreeMap;

use crate::rounding::Ratio;

/// One entry of the price file as the caller decoded it.
#[derive(Clone, Copy, Debug)]
pub struct RawPriceRow<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub category: &'a str,
    pub usd: &'a str,
    pub per: u64,
    pub as_of: &'a str,
    pub source: &'a str,
}

/// Why a price file was refused: the entry's index and a closed reason, never the entry's text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TariffError {
    Entry { index: usize, reason: &'static str },
    Duplicate { index: usize },
}

/// One dated, sourced rate: `usd` for every `per` units, exact in nano-dollars.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Price {
    pub nano_usd: u64,
    pub per: u64,
    pub as_of: String,
    pub source: String,
}

impl Price {
    /// The exact price of `quantity` units in micro-dollars: `quantity * usd / per`.
    pub fn micro_dollars(&self, quantity: u64) -> Ratio {
        Ratio {
            numerator: u128::from(quantity) * u128::from(self.nano_usd),
            denominator: u128::from(self.per) * 1_000,
        }
    }

    /// The price in whole micro-dollars, rounded half up per line (the cost book's way; the money
    /// core prices an operation by summing the exact ratios and rounding once).
    pub fn micro_dollars_half_up(&self, quantity: u64) -> u64 {
        self.micro_dollars(quantity)
            .round_half_up()
            .unwrap_or(u64::MAX)
    }
}

/// `text` as whole nano-dollars: digits, optionally a point and at most nine decimals.
pub fn parse_nano_usd(text: &str) -> Option<u64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || fraction.len() > 9
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<9}").parse().ok()?
    };
    whole.checked_mul(1_000_000_000)?.checked_add(fraction)
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

fn is_word(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 40
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The dated prices by provider, model and category. A lookup is a pure read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PriceTable {
    entries: BTreeMap<(String, String, String), Price>,
}

impl PriceTable {
    /// Validates every row; any malformed row refuses the whole table.
    pub fn from_rows(rows: &[RawPriceRow<'_>]) -> Result<Self, TariffError> {
        let mut entries = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            let refuse = |reason| TariffError::Entry { index, reason };
            if !is_word(row.provider) {
                return Err(refuse("provider"));
            }
            if !is_word(row.category) {
                return Err(refuse("category"));
            }
            if row.model.is_empty()
                || row.model.len() > 120
                || !row.model.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(refuse("model"));
            }
            let nano_usd = parse_nano_usd(row.usd).ok_or(refuse("usd"))?;
            if row.per == 0 {
                return Err(refuse("per"));
            }
            if !is_date(row.as_of) {
                return Err(refuse("as_of"));
            }
            if !row.source.starts_with("https://") || row.source.len() > 400 {
                return Err(refuse("source"));
            }
            let price = Price {
                nano_usd,
                per: row.per,
                as_of: row.as_of.to_owned(),
                source: row.source.to_owned(),
            };
            let key = (
                row.provider.to_owned(),
                row.model.to_owned(),
                row.category.to_owned(),
            );
            if entries.insert(key, price).is_some() {
                return Err(TariffError::Duplicate { index });
            }
        }
        Ok(Self { entries })
    }

    pub fn lookup(&self, provider: &str, model: &str, category: &str) -> Option<&Price> {
        self.entries
            .get(&(provider.to_owned(), model.to_owned(), category.to_owned()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every row, in key order.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &str, &str, &Price)> {
        self.entries
            .iter()
            .map(|((provider, model, category), price)| {
                (provider.as_str(), model.as_str(), category.as_str(), price)
            })
    }
}

/// Parses a bill in the documented comparison form: a `provider,model,category,usd` header, then
/// one row per line item, amounts in dollars (at most six decimals; repeated rows add up).
pub fn parse_bill(csv: &str) -> Result<BTreeMap<(String, String, String), u64>, String> {
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());
    match lines.next().map(str::trim) {
        Some("provider,model,category,usd") => {}
        _ => return Err("the bill must start with the header provider,model,category,usd".into()),
    }
    let mut rows = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let row = index + 2;
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let [provider, model, category, amount] = fields[..] else {
            return Err(format!("bill row {row} must have four fields"));
        };
        if !is_word(provider) {
            return Err(format!("bill row {row}: unknown provider"));
        }
        if !is_word(category) {
            return Err(format!("bill row {row}: unknown category"));
        }
        let nanos = parse_nano_usd(amount)
            .filter(|nanos| nanos % 1_000 == 0)
            .ok_or_else(|| {
                format!("bill row {row}: usd must be dollars with at most six decimals")
            })?;
        let total: &mut u64 = rows
            .entry((provider.to_owned(), model.to_owned(), category.to_owned()))
            .or_default();
        *total = total.saturating_add(nanos / 1_000);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(usd: &'a str, per: u64) -> RawPriceRow<'a> {
        RawPriceRow {
            provider: "openai",
            model: "gpt-6-luna",
            category: "text_input",
            usd,
            per,
            as_of: "2026-10-24",
            source: "https://developers.openai.com/api/docs/pricing",
        }
    }

    #[test]
    fn a_price_is_exact_in_nano_dollars_and_rounds_half_up_per_line() {
        let table = PriceTable::from_rows(&[row("0.10", 1_000_000)]).expect("table");
        let price = table
            .lookup("openai", "gpt-6-luna", "text_input")
            .expect("price");
        assert_eq!(price.micro_dollars_half_up(5), 1);
        assert_eq!(price.micro_dollars_half_up(4), 0);
        assert_eq!(price.micro_dollars_half_up(1_000_000), 100_000);
    }

    #[test]
    fn one_malformed_row_refuses_the_whole_table() {
        assert_eq!(
            PriceTable::from_rows(&[row("1.", 1)]),
            Err(TariffError::Entry {
                index: 0,
                reason: "usd"
            })
        );
        assert_eq!(
            PriceTable::from_rows(&[row("1", 0)]),
            Err(TariffError::Entry {
                index: 0,
                reason: "per"
            })
        );
        assert_eq!(
            PriceTable::from_rows(&[row("1", 1), row("2", 1)]),
            Err(TariffError::Duplicate { index: 1 })
        );
    }

    #[test]
    fn a_bill_reads_in_the_documented_form_and_adds_repeated_rows() {
        let bill = parse_bill("provider,model,category,usd\nopenai,gpt-6-luna,text_input,0.5\nopenai,gpt-6-luna,text_input,0.25\n").expect("bill");
        assert_eq!(
            bill.get(&(
                "openai".to_owned(),
                "gpt-6-luna".to_owned(),
                "text_input".to_owned()
            )),
            Some(&750_000)
        );
        assert!(parse_bill("nope").is_err());
        assert!(
            parse_bill("provider,model,category,usd\nopenai,m,text_input,0.0000001\n").is_err()
        );
    }
}
