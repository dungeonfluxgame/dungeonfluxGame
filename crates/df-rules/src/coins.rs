//! In-game purse: five coin denominations (cp, sp, ep, gp, pp) as u64 counts, the SRD 5.2.1 coin
//! values (SRD p.89: 1 sp = 10 cp, 1 ep = 50 cp, 1 gp = 100 cp, 1 pp = 1000 cp), checked add, and
//! checked subtract with change. `Price` is a cost in copper pieces, the unit of every SRD
//! equipment price in `srd_equipment_prices`.
//!
//! Owns: the purse arithmetic and its change rule (see `Coins::subtract`).
//! Never does: move items, make shop offers, haggle, or write table state. The atomic item-and-coin
//! move is a table effect (D.7) that calls these functions; it is not defined here.
//! Fails: with a typed `CoinError` (`Insufficient` or `Overflow`) and leaves the purse unchanged.
//! Every purse holds a copper total that fits in u64, so `total_copper` is `Some` for any `Coins`
//! this module builds, including one deserialized from input. No operation panics, clamps or
//! substitutes a default amount.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Copper value of each denomination in purse order: cp, sp, ep, gp, pp (SRD p.89). Each value
/// divides the next one, which is what makes the tender search in `smallest_tender` exact.
const COPPER_VALUES: [u64; 5] = [1, 10, 50, 100, 1000];

/// Why a coin operation failed. Both variants leave the purse it was called on unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoinError {
    /// The purse cannot pay the price: `purse_cp` is less than `cost_cp`.
    Insufficient { cost_cp: u64, purse_cp: u64 },
    /// An amount or count would not fit in u64, so the purse cannot hold it.
    Overflow,
}

impl fmt::Display for CoinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Insufficient { cost_cp, purse_cp } => write!(
                formatter,
                "not enough coins: the price is {cost_cp} cp and the purse holds {purse_cp} cp"
            ),
            Self::Overflow => write!(
                formatter,
                "the coins would total more than a purse can hold (the copper total passes the \
                    largest number a purse can record)"
            ),
        }
    }
}

impl std::error::Error for CoinError {}

/// A price in copper pieces (cp). SRD p.89: 100 cp is 1 gp.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Price {
    copper: u64,
}

impl Price {
    /// A price of `copper` copper pieces. Any u64 is a valid price, including zero.
    pub const fn from_copper(copper: u64) -> Self {
        Self { copper }
    }

    /// The price in copper pieces.
    pub const fn copper(self) -> u64 {
        self.copper
    }
}

/// An in-game purse: counts of copper, silver, electrum, gold and platinum pieces (SRD p.89).
///
/// The counts are not required to be in lowest terms: a purse of 12 cp is valid and stays that way
/// when a zero-cost payment leaves it unchanged. The only invariant is that the copper total fits
/// in u64, which every constructor and operation checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(try_from = "CoinsRecord")]
pub struct Coins {
    cp: u64,
    sp: u64,
    ep: u64,
    gp: u64,
    pp: u64,
}

/// The wire shape of `Coins`. Deserializing a `Coins` goes through `Coins::new`, so input whose
/// total does not fit in u64 is refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoinsRecord {
    cp: u64,
    sp: u64,
    ep: u64,
    gp: u64,
    pp: u64,
}

impl TryFrom<CoinsRecord> for Coins {
    type Error = CoinError;

    fn try_from(record: CoinsRecord) -> Result<Self, Self::Error> {
        Self::new(record.cp, record.sp, record.ep, record.gp, record.pp)
    }
}

impl Coins {
    /// A purse with the given counts. Fails with `Overflow` when the copper total does not fit in
    /// u64.
    pub fn new(cp: u64, sp: u64, ep: u64, gp: u64, pp: u64) -> Result<Self, CoinError> {
        Self::from_stock([cp, sp, ep, gp, pp])
    }

    /// The purse worth `copper` copper pieces in the fewest coins: as many platinum pieces as fit,
    /// then gold, electrum, silver and copper. Every u64 amount has such a purse, so this cannot
    /// fail.
    pub const fn from_copper(copper: u64) -> Self {
        let pp = copper / 1000;
        let rest = copper % 1000;
        let gp = rest / 100;
        let rest = rest % 100;
        let ep = rest / 50;
        let rest = rest % 50;
        let sp = rest / 10;
        let cp = rest % 10;
        Self { cp, sp, ep, gp, pp }
    }

    /// Copper pieces held.
    pub const fn cp(self) -> u64 {
        self.cp
    }

    /// Silver pieces held.
    pub const fn sp(self) -> u64 {
        self.sp
    }

    /// Electrum pieces held.
    pub const fn ep(self) -> u64 {
        self.ep
    }

    /// Gold pieces held.
    pub const fn gp(self) -> u64 {
        self.gp
    }

    /// Platinum pieces held.
    pub const fn pp(self) -> u64 {
        self.pp
    }

    /// The value of the whole purse in copper pieces. `None` only if the total does not fit in
    /// u64, which no purse built by this module can reach.
    pub fn total_copper(self) -> Option<u64> {
        copper_total(&self.stock())
    }

    /// Whether the purse holds at least `cost`.
    pub fn can_afford(self, cost: Price) -> bool {
        self.total_copper()
            .is_some_and(|total| total >= cost.copper())
    }

    /// The purse with `other` added, denomination by denomination. No change is made: coins
    /// stay in the counts they came in. Fails with `Overflow` when a count or the total does not
    /// fit.
    pub fn checked_add(self, other: Coins) -> Result<Coins, CoinError> {
        let mut sum = [0_u64; 5];
        for ((slot, mine), theirs) in sum.iter_mut().zip(self.stock()).zip(other.stock()) {
            *slot = mine.checked_add(theirs).ok_or(CoinError::Overflow)?;
        }
        Self::from_stock(sum)
    }

    /// Pays `cost` from the purse and returns what is left, or fails with `Insufficient` (the purse
    /// holds less than the cost) or `Overflow`. The purse is never changed by a failed call.
    ///
    /// The change rule, in order:
    /// 1. Tender. The coins handed over are the smallest amount at or above the cost that the purse
    ///    can make from its own coins (SRD p.89 denominations). Among equal amounts the tender uses
    ///    the fewest coins. When the purse can pay the cost exactly, the tender is exactly the cost
    ///    and there is no change. `smallest_tender` finds it exactly: each denomination divides the
    ///    next, so at each level only the most top coins that fit, or one more, can be part of the
    ///    smallest tender, and the lower coins are searched only for the first of those.
    /// 2. Change. The overpaid copper is returned as the fewest coins (`from_copper`), in cp, sp,
    ///    ep, gp and pp. A tender that does not overpay returns no change.
    /// 3. Remainder. The purse keeps every coin the tender did not hand over, so its counts change
    ///    only by the handed coins and the change. A cost of zero returns the purse unchanged.
    ///
    /// The copper left is always the copper held minus the cost, with nothing lost or invented.
    pub fn subtract(self, cost: Price) -> Result<Coins, CoinError> {
        let owed_cp = cost.copper;
        let purse_cp = self.total_copper().ok_or(CoinError::Overflow)?;
        let insufficient = CoinError::Insufficient {
            cost_cp: owed_cp,
            purse_cp,
        };
        if owed_cp > purse_cp {
            return Err(insufficient);
        }
        let stock = self.stock();
        let tender = smallest_tender(&stock, COPPER_VALUES.len(), owed_cp)?.ok_or(insufficient)?;
        let change_cp = tender
            .paid_cp
            .checked_sub(owed_cp)
            .ok_or(CoinError::Overflow)?;
        let change = Coins::from_copper(change_cp).stock();
        let mut left = [0_u64; 5];
        for (index, slot) in left.iter_mut().enumerate() {
            let kept = stock[index]
                .checked_sub(tender.handed[index])
                .ok_or(insufficient)?;
            *slot = kept.checked_add(change[index]).ok_or(CoinError::Overflow)?;
        }
        Self::from_stock(left)
    }

    /// The counts in purse order: cp, sp, ep, gp, pp.
    fn stock(self) -> [u64; 5] {
        [self.cp, self.sp, self.ep, self.gp, self.pp]
    }

    /// A purse from counts in purse order, refused when the copper total does not fit in u64.
    fn from_stock(stock: [u64; 5]) -> Result<Self, CoinError> {
        let [cp, sp, ep, gp, pp] = stock;
        let coins = Self { cp, sp, ep, gp, pp };
        coins.total_copper().ok_or(CoinError::Overflow)?;
        Ok(coins)
    }
}

/// The copper total of counts in purse order, or `None` if any part or the sum leaves u64.
fn copper_total(stock: &[u64; 5]) -> Option<u64> {
    let mut total = 0_u64;
    for (count, value) in stock.iter().zip(COPPER_VALUES) {
        total = total.checked_add(count.checked_mul(value)?)?;
    }
    Some(total)
}

/// The coins one payment hands over: the copper they make (`paid_cp`), how many coins that is
/// (`coins`), and the count of each denomination in purse order (`handed`). The fields are
/// `pub(crate)` so the tests can check the search against an exhaustive one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Tender {
    pub(crate) paid_cp: u64,
    pub(crate) coins: u64,
    pub(crate) handed: [u64; 5],
}

impl Tender {
    const EMPTY: Tender = Tender {
        paid_cp: 0,
        coins: 0,
        handed: [0; 5],
    };

    /// This tender plus `count` coins of denomination `index` worth `count_cp` in total.
    fn with_coins(mut self, index: usize, count: u64, count_cp: u64) -> Result<Tender, CoinError> {
        self.paid_cp = self
            .paid_cp
            .checked_add(count_cp)
            .ok_or(CoinError::Overflow)?;
        self.coins = self.coins.checked_add(count).ok_or(CoinError::Overflow)?;
        self.handed[index] = count;
        Ok(self)
    }
}

/// The smaller of two candidate tenders by (copper paid, coin count). On a full tie the current
/// best is kept, so the result does not depend on search order beyond the rule itself.
fn keep_smaller(best: Option<Tender>, candidate: Tender) -> Option<Tender> {
    match best {
        Some(current)
            if (current.paid_cp, current.coins) <= (candidate.paid_cp, candidate.coins) =>
        {
            Some(current)
        }
        _ => Some(candidate),
    }
}

/// The tender of least value at or above `owed_cp` that the first `levels` denominations of
/// `COPPER_VALUES` can make from `stock`, with the fewest coins among equal values. `Ok(None)` when
/// those denominations cannot reach `owed_cp`.
///
/// Proof sketch. Let `v` be the value of the top level, `n` its count and `q = owed / v`. In a
/// tender of least value no more than `q + 1` top coins are used, since dropping one of `q + 2`
/// top coins still pays the owed amount. When fewer than `min(n, q)` top coins are used and the
/// lower coins reach at least `v`, a lower subset worth exactly `v` exists (each lower value
/// divides `v`, so the running sum hits `v` exactly), and swapping it for one top coin keeps the
/// value and lowers the coin count. So the top count is `min(n, q)` or `min(n, q + 1)`, and each
/// choice leaves a smaller problem for the lower levels.
pub(crate) fn smallest_tender(
    stock: &[u64; 5],
    levels: usize,
    owed_cp: u64,
) -> Result<Option<Tender>, CoinError> {
    if owed_cp == 0 {
        return Ok(Some(Tender::EMPTY));
    }
    if levels == 0 {
        return Ok(None);
    }
    // `levels` is at most COPPER_VALUES.len(): callers pass that length and recursion only lowers
    // it, so `top` indexes the five-element arrays.
    let top = levels - 1;
    let value = COPPER_VALUES[top];
    let available = stock[top];
    let whole = owed_cp / value;
    let mut best = None;

    // Candidate: as many top coins as fit (never more than the purse holds), then the rest from
    // the lower coins. When the top coins are exhausted the rest is larger than `value` and the
    // lower coins decide whether it can be reached.
    let taken = available.min(whole);
    let taken_cp = taken.checked_mul(value).ok_or(CoinError::Overflow)?;
    let rest = owed_cp.checked_sub(taken_cp).ok_or(CoinError::Overflow)?;
    if let Some(lower) = smallest_tender(stock, top, rest)? {
        best = keep_smaller(best, lower.with_coins(top, taken, taken_cp)?);
    }

    // Candidate: one top coin more than the owed amount needs, and no lower coins.
    if available > whole {
        let coins = whole + 1;
        let paid_cp = coins.checked_mul(value).ok_or(CoinError::Overflow)?;
        let mut handed = [0_u64; 5];
        handed[top] = coins;
        best = keep_smaller(
            best,
            Tender {
                paid_cp,
                coins,
                handed,
            },
        );
    }
    Ok(best)
}
