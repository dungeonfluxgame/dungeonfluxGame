//! Weight, baggage space and buying (`planning/town-to-tomb.md` part C).
//!
//! An item weighs what the SRD catalog says one unit weighs (`df_rules::srd_equipment_prices`), a
//! coin 1/50 lb (SRD p.89); an item the catalog does not list has no known weight and is named as
//! unweighed. A container the owner holds (an item with a catalog capacity) holds the items stowed
//! in it ([`Stowed`]) within its capacity: a weight limit (backpack 30 lb, pouch 6 lb) or a list of
//! the only things it takes (a quiver's 20 arrows). A container with only a volume or a liquid
//! measure (a chest, a flask) holds no items here: the SRD gives items no volume to measure them
//! by. A container is never put in another.
//!
//! A purchase ([`purchase`], the async [`buy`]) is one exchange between the buyer and the shop
//! through [`exchange`], the transfer an agreed deal settles with: the buyer's coins (the SRD
//! change rule, `Coins::pay`) for the goods and the shop's exact change. A pack arrives as its
//! contents (SRD pp.96-97), stowed in the pack's own container as far as they fit and carried
//! beside it otherwise. Other goods go into the buyer's containers that could hold them as far as
//! they have room, and the rest is carried as it is. The purchase is refused whole, with nothing
//! moved, when the shop does not sell it, the buyer's coins are short, the shop cannot make change
//! or the load would pass the buyer's carrying capacity (`over_capacity`, SRD p.178); a count or
//! weight past u64 is `overflow`, never zero. Each purchase or refusal is one `INVENTORY` line.

use df_rules::coins::{CoinError, Coins, Denomination, Price};
use df_rules::srd_equipment_prices::{
    COIN_WEIGHT, Capacity, EquipmentItem, Weight, carrying_capacity, equipment, item_id,
};
use serde::Serialize;

use super::operations;
use super::routes::record_changed;
use super::store;
use super::{
    Inventory, InventoryApplied, InventoryChange, InventoryError, InventoryItem, ItemCount,
    ItemUse, OwnerKind, Stowed, apply_changes, exchange, logged_id,
};
use crate::blocking::run_blocking;
use crate::state::AppState;

/// The weight of one unit of `item`: a coin's, or its catalog unit weight; none when the catalog
/// does not list it.
fn unit_weight(item: &InventoryItem) -> Option<Weight> {
    if item.coin.is_some() {
        return Some(COIN_WEIGHT);
    }
    equipment(&item.id).map(EquipmentItem::unit_weight)
}

/// The capacity of `id` as a holder of items: a catalog container with a weight limit or a list
/// of the things it takes.
fn holder(id: &str) -> Option<Capacity> {
    equipment(id)
        .and_then(|entry| entry.capacity)
        .filter(|capacity| capacity.weight.is_some() || !capacity.only.is_empty())
}

/// Whether `id` is a container of the catalog.
fn is_container(id: &str) -> bool {
    equipment(id).is_some_and(|entry| entry.capacity.is_some())
}

/// The catalog id of `id` (an inventory id may be a kit's name for it).
fn catalog_id(id: &str) -> Option<&'static str> {
    equipment(id).map(|entry| entry.id)
}

/// How many of `container` the owner holds.
fn held(inventory: &Inventory, id: &str) -> u64 {
    inventory.item(id).map_or(0, |item| item.count)
}

fn stowed_in<'a>(inventory: &'a Inventory, container: &'a str) -> impl Iterator<Item = &'a Stowed> {
    inventory
        .stowed
        .iter()
        .filter(move |stowed| stowed.container == container)
}

/// The typed refusal of a weight, share or count of `owner`'s `item` that would not fit in u64.
fn overflow(owner: &str, item: &str) -> InventoryError {
    InventoryError::Overflow {
        owner: owner.to_owned(),
        item: item.to_owned(),
    }
}

/// The weight of `count` of `item`, or none when its weight is not known; `overflow` when it does
/// not fit.
fn weight_of(
    inventory: &Inventory,
    item: &InventoryItem,
    count: u64,
) -> Result<Option<Weight>, InventoryError> {
    unit_weight(item)
        .map(|each| {
            each.checked_times(count)
                .ok_or_else(|| overflow(&inventory.owner, &item.id))
        })
        .transpose()
}

/// What the things stowed in `container` weigh.
fn load_of(inventory: &Inventory, container: &str) -> Result<Weight, InventoryError> {
    stowed_in(inventory, container).try_fold(Weight::ZERO, |load, stowed| {
        let weight = match inventory.item(&stowed.item) {
            Some(item) => weight_of(inventory, item, stowed.count)?.unwrap_or(Weight::ZERO),
            None => Weight::ZERO,
        };
        load.checked_add(weight)
            .ok_or_else(|| overflow(&inventory.owner, container))
    })
}

/// A container that takes only some things, as shares: each listed thing is `whole / most`
/// shares, and one container holds `whole` (a Map or Scroll Case: 10 shares, a sheet of paper 1, a
/// sheet of parchment 2).
fn whole_shares(only: &[(&str, u32)]) -> u64 {
    only.iter()
        .fold(1, |whole, (_, most)| lcm(whole, u64::from(*most)))
}

fn lcm(a: u64, b: u64) -> u64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x.max(1) * b
}

/// The shares of `item` in a container taking only `only`, or none when it is not listed.
fn share_of(only: &[(&str, u32)], item: &str) -> Option<u64> {
    let id = catalog_id(item)?;
    let most = only.iter().find(|(listed, _)| *listed == id)?.1;
    Some(whole_shares(only) / u64::from(most))
}

/// The shares used of `container` (one taking only some things) and the shares it holds.
fn shares(
    inventory: &Inventory,
    container: &str,
    only: &[(&str, u32)],
) -> Result<(u64, u64), InventoryError> {
    let too_many = || overflow(&inventory.owner, container);
    let used = stowed_in(inventory, container).try_fold(0_u64, |used, stowed| {
        let each = share_of(only, &stowed.item).unwrap_or(0);
        each.checked_mul(stowed.count)
            .and_then(|these| used.checked_add(these))
            .ok_or_else(too_many)
    })?;
    let most = whole_shares(only)
        .checked_mul(held(inventory, container))
        .ok_or_else(too_many)?;
    Ok((used, most))
}

/// What `container` holds by weight, all of them together; none when it has no weight limit.
fn weight_limit(inventory: &Inventory, container: &str) -> Result<Option<Weight>, InventoryError> {
    holder(container)
        .and_then(|capacity| capacity.weight)
        .map(|limit| {
            limit
                .checked_times(held(inventory, container))
                .ok_or_else(|| overflow(&inventory.owner, container))
        })
        .transpose()
}

/// How many more of `item` fit in `container`: none when the owner holds no such container, it
/// does not take the item, or the item's weight is not known.
fn room(inventory: &Inventory, container: &str, item: &str) -> Result<u64, InventoryError> {
    let Some(capacity) = holder(container).filter(|_| held(inventory, container) > 0) else {
        return Ok(0);
    };
    if is_container(item) {
        return Ok(0);
    }
    if !capacity.only.is_empty() {
        let Some(each) = share_of(capacity.only, item) else {
            return Ok(0);
        };
        let (used, most) = shares(inventory, container, capacity.only)?;
        return Ok(most.saturating_sub(used) / each);
    }
    let Some(limit) = weight_limit(inventory, container)? else {
        return Ok(0);
    };
    let Some(each) = inventory.item(item).and_then(unit_weight) else {
        return Ok(0);
    };
    // A load over the limit (a container that lost a twin) has no room, not a negative one.
    let free = limit
        .milli_pounds()
        .saturating_sub(load_of(inventory, container)?.milli_pounds());
    Ok(free.checked_div(each.milli_pounds()).unwrap_or(u64::MAX))
}

/// Whether `container` could take one `item` when it is empty.
fn could_hold(inventory: &Inventory, container: &str, item: &str) -> Result<bool, InventoryError> {
    let Some(capacity) = holder(container) else {
        return Ok(false);
    };
    if !capacity.only.is_empty() {
        return Ok(share_of(capacity.only, item).is_some());
    }
    let limit = weight_limit(inventory, container)?;
    let each = inventory.item(item).and_then(unit_weight);
    Ok(!is_container(item) && matches!((limit, each), (Some(limit), Some(each)) if each <= limit))
}

/// Whether what is stowed in `container` is within its capacity.
fn fits(inventory: &Inventory, container: &str) -> Result<bool, InventoryError> {
    let Some(capacity) = holder(container) else {
        return Ok(false);
    };
    if !capacity.only.is_empty() {
        let (used, most) = shares(inventory, container, capacity.only)?;
        return Ok(used <= most);
    }
    let limit = weight_limit(inventory, container)?;
    let load = load_of(inventory, container)?;
    Ok(limit.is_some_and(|limit| load <= limit))
}

/// Keeps what `inventory` has stowed true after a change: an entry whose container is gone, or
/// that holds none, goes; an item whose count fell keeps in its containers no more than it has
/// (the newest entries give up first, as what is carried loose is used first); a container whose
/// count fell keeps what still fits, the rest coming out to be carried.
pub(super) fn settle_stowed(inventory: &mut Inventory) -> Result<(), InventoryError> {
    let counts: Vec<(String, u64)> = inventory
        .items
        .iter()
        .map(|item| (item.id.clone(), item.count))
        .collect();
    let count_of = |id: &str| {
        counts
            .iter()
            .find(|(held, _)| held == id)
            .map_or(0, |(_, count)| *count)
    };
    inventory
        .stowed
        .retain(|stowed| stowed.count > 0 && count_of(&stowed.container) > 0);
    for (item, count) in &counts {
        let mut left = *count;
        for stowed in inventory
            .stowed
            .iter_mut()
            .filter(|stowed| stowed.item == *item)
        {
            stowed.count = stowed.count.min(left);
            left -= stowed.count;
        }
    }
    inventory.stowed.retain(|stowed| stowed.count > 0);
    let containers: Vec<String> = inventory
        .stowed
        .iter()
        .map(|stowed| stowed.container.clone())
        .collect();
    for container in containers {
        while !fits(inventory, &container)? {
            let Some(last) = inventory
                .stowed
                .iter()
                .rposition(|stowed| stowed.container == container)
            else {
                break;
            };
            inventory.stowed.remove(last);
        }
    }
    Ok(())
}

/// Puts `count` of `item` into `container`, beside any of it already there.
fn stow(
    inventory: &mut Inventory,
    item: &str,
    container: &str,
    count: u64,
) -> Result<(), InventoryError> {
    if count == 0 {
        return Ok(());
    }
    let owner = inventory.owner.clone();
    match inventory
        .stowed
        .iter_mut()
        .find(|stowed| stowed.item == item && stowed.container == container)
    {
        Some(stowed) => {
            stowed.count = stowed
                .count
                .checked_add(count)
                .ok_or_else(|| overflow(&owner, item))?;
        }
        None => inventory.stowed.push(Stowed {
            item: item.to_owned(),
            container: container.to_owned(),
            count,
        }),
    }
    Ok(())
}

/// What everything `inventory` holds weighs, and the ids of what has no known weight.
fn carried(inventory: &Inventory) -> Result<(Weight, Vec<String>), InventoryError> {
    let mut unweighed = Vec::new();
    let mut total = Weight::ZERO;
    for item in inventory.held() {
        match weight_of(inventory, item, item.count)? {
            Some(weight) => {
                total = total
                    .checked_add(weight)
                    .ok_or_else(|| overflow(&inventory.owner, &item.id))?;
            }
            None => unweighed.push(item.id.clone()),
        }
    }
    Ok((total, unweighed))
}

/// The coins among `items`: each coin item's denomination and count.
pub(crate) fn coins_in(items: &[InventoryItem]) -> impl Iterator<Item = (Denomination, u64)> + '_ {
    items
        .iter()
        .filter_map(|item| item.coin.map(|coin| (coin, item.count)))
}

/// The coin `items` counts its money in: the most valuable denomination it holds at least one of.
/// The Shop page, the Bag and the judge's prices all quote in it.
pub(crate) fn counting_coin(items: &[InventoryItem]) -> Option<Denomination> {
    coins_in(items)
        .filter(|(_, count)| *count > 0)
        .map(|(coin, _)| coin)
        .max_by_key(|coin| coin.copper())
}

/// `coins` (denomination and count) as one purse of `owner`'s; `overflow` past u64.
pub(crate) fn purse_of(
    owner: &str,
    coins: impl IntoIterator<Item = (Denomination, u64)>,
) -> Result<Coins, InventoryError> {
    coins
        .into_iter()
        .try_fold(Coins::default(), |purse, (coin, count)| {
            purse.with_added(coin, count)
        })
        .map_err(|_| overflow(owner, "coins"))
}

/// The copper value of `inventory`'s coins.
pub(super) fn coins_cp(inventory: &Inventory) -> Result<u64, InventoryError> {
    purse_cp(&inventory.owner, &inventory.items)
}

/// The coin items of `items` that hand over `coins`: of each denomination, its items in held
/// order, as many of each as it holds.
fn coin_items(items: &[InventoryItem], coins: Coins) -> Vec<ItemCount> {
    let mut named = Vec::new();
    for denomination in Denomination::ALL {
        let mut owed = coins.count(denomination);
        for item in items.iter().filter(|item| item.coin == Some(denomination)) {
            let count = owed.min(item.count);
            if count > 0 {
                named.push(ItemCount {
                    item: item.id.clone(),
                    count,
                });
                owed -= count;
            }
        }
    }
    named
}

/// The coin items of `owner`'s `items` that make exactly `price` in the fewest coins
/// (`Coins::exact`); none when its coins cannot make it exactly (no change is given back).
pub(crate) fn exact_coins(
    owner: &str,
    items: &[InventoryItem],
    price: Price,
) -> Result<Option<Vec<ItemCount>>, InventoryError> {
    let exact = purse_of(owner, coins_in(items))?
        .exact(price)
        .map_err(|_| overflow(owner, "coins"))?;
    Ok(exact.map(|coins| coin_items(items, coins)))
}

/// The coin `id` is for an owner holding `held`: the held item's coin, else the coin a standard
/// coin id names (`copper-piece`, [`standard_coin_of`]), so an amount said in a coin the owner
/// does not hold is still that coin; none for goods.
pub(crate) fn coin_of(held: &[InventoryItem], id: &str) -> Option<Denomination> {
    match held.iter().find(|item| item.id == id) {
        Some(item) => item.coin,
        None => standard_coin_of(id),
    }
}

/// The copper value of the coins of `owner`, who holds `held`.
pub(crate) fn purse_cp(owner: &str, held: &[InventoryItem]) -> Result<u64, InventoryError> {
    purse_of(owner, coins_in(held))?
        .total_copper()
        .ok_or_else(|| overflow(owner, "coins"))
}

/// `items`, one side of a deal of `owner` (who holds `held`), as its goods and the copper value of
/// its coins ([`coin_of`]): `overflow` past u64.
pub(crate) fn goods_and_coins(
    owner: &str,
    held: &[InventoryItem],
    items: &[ItemCount],
) -> Result<(Vec<ItemCount>, u64), InventoryError> {
    let mut goods = Vec::new();
    let mut coins = Vec::new();
    for named in items {
        match coin_of(held, &named.item) {
            Some(coin) => coins.push((coin, named.count)),
            None => goods.push(named.clone()),
        }
    }
    let coins_cp = purse_of(owner, coins)?
        .total_copper()
        .ok_or_else(|| overflow(owner, "coins"))?;
    Ok((goods, coins_cp))
}

/// What an agreed deal paid in coins ([`deal_transfers`]): who paid the copper the coins named
/// came to, net; the coins handed over and the change back, in copper; and the payer's coins
/// before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DealPaid {
    pub(super) payer: String,
    pub(super) cp: u64,
    pub(super) handed_cp: u64,
    pub(super) change_cp: u64,
    pub(super) coins_before_cp: u64,
}

/// What an agreed deal moves ([`deal_transfers`]): the items one side hands the other each way,
/// and the payment, when coins moved.
pub(super) struct DealTransfers {
    pub(super) a_gives: Vec<ItemCount>,
    pub(super) b_gives: Vec<ItemCount>,
    pub(super) paid: Option<DealPaid>,
}

/// What an agreed deal between `a` and `b` moves: each side's goods as named, and the coins named
/// on both sides netted into one payment by the side that owes more, made by the change rule
/// ([`coin_terms`]: the payer's coins for the amount, the payee's exact change back), whatever
/// coin the terms named it in. The payment is none when the coins named each way are worth the
/// same. Refused `insufficient_coin`, `no_change` or `overflow`; nothing is moved here.
pub(super) fn deal_transfers(
    (a, a_gives): (&Inventory, &[ItemCount]),
    (b, b_gives): (&Inventory, &[ItemCount]),
) -> Result<DealTransfers, InventoryError> {
    let (mut a_list, a_cp) = goods_and_coins(&a.owner, &a.items, a_gives)?;
    let (mut b_list, b_cp) = goods_and_coins(&b.owner, &b.items, b_gives)?;
    let (payer, payee, cp) = match a_cp.cmp(&b_cp) {
        std::cmp::Ordering::Equal => {
            return Ok(DealTransfers {
                a_gives: a_list,
                b_gives: b_list,
                paid: None,
            });
        }
        std::cmp::Ordering::Greater => (a, b, a_cp - b_cp),
        std::cmp::Ordering::Less => (b, a, b_cp - a_cp),
    };
    let coins = coin_terms(payer, payee, Price::from_copper(cp))?;
    let paid = DealPaid {
        payer: payer.owner.clone(),
        cp,
        handed_cp: cp + coins.change_cp,
        change_cp: coins.change_cp,
        coins_before_cp: coins.coins_before_cp,
    };
    if payer.owner == a.owner {
        a_list.extend(coins.handed);
        b_list.extend(coins.change);
    } else {
        b_list.extend(coins.handed);
        a_list.extend(coins.change);
    }
    Ok(DealTransfers {
        a_gives: a_list,
        b_gives: b_list,
        paid: Some(paid),
    })
}

/// What `inventory` carries, in pounds for a log line (`overflow` past u64).
pub(super) fn carried_pounds(inventory: &Inventory) -> Result<String, InventoryError> {
    carried(inventory).map(|(weight, _)| pounds(weight))
}

/// What one purchase came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bought {
    pub owner: String,
    pub shop: String,
    pub item: String,
    pub qty: u64,
    pub price: Price,
    /// The buyer's coins before and after, in copper.
    pub coins_before_cp: u64,
    pub coins_after_cp: u64,
    /// What the buyer carries after the purchase, and their carrying capacity.
    pub weight: Weight,
    pub capacity: Weight,
}

/// The goods of a purchase stowed in `buyer`: a pack's contents in place of the pack (the pack's
/// own line taken out), stowed in its own container as far as they fit; other goods in the
/// buyer's containers as far as they fit, the rest carried.
fn arrange(
    buyer: Inventory,
    entry: &EquipmentItem,
    (item, qty): (&str, u64),
) -> Result<(Inventory, Vec<InventoryApplied>), InventoryError> {
    if entry.contents.is_empty() {
        return stow_goods(buyer, item, qty).map(|buyer| (buyer, Vec::new()));
    }
    let why = format!("unpacked from {}", entry.name);
    let mut changes = vec![InventoryChange::Remove {
        item: item.to_owned(),
        count: qty,
        why: why.clone(),
    }];
    for (id, count) in entry.contents {
        let content = equipment(id).ok_or_else(|| InventoryError::UnknownItem {
            owner: buyer.owner.clone(),
            item: (*id).to_owned(),
        })?;
        changes.push(InventoryChange::Add {
            item: (*id).to_owned(),
            count: u64::from(*count)
                .checked_mul(qty)
                .ok_or_else(|| overflow(&buyer.owner, id))?,
            name: Some(content.name.to_owned()),
            look: Some(format!("{}, from a {}.", content.name, entry.name)),
            usage: Some(if content.used_up {
                ItemUse::Consumed
            } else {
                ItemUse::Kept
            }),
            price_cp: content.price_for(1).map(Price::copper),
            why: why.clone(),
        });
    }
    let (mut buyer, applied) = apply_changes(&buyer, &changes)?;
    // The pack is gone once unpacked: no line of none of it is kept (a bag never shows "x0" of a
    // pack it opened).
    buyer.items.retain(|held| held.id != item || held.count > 0);
    let containers: Vec<&str> = entry
        .contents
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| holder(id).is_some())
        .collect();
    for (id, count) in entry.contents {
        // The counts were checked when they were added.
        let mut left = u64::from(*count) * qty;
        for container in &containers {
            let fitting = room(&buyer, container, id)?.min(left);
            stow(&mut buyer, id, container, fitting)?;
            left -= fitting;
        }
    }
    Ok((buyer, applied))
}

/// `qty` of `item`, just added to `buyer`, put into the buyer's containers that could hold it as
/// far as they have room, in order: those that take only some things first (a quiver for arrows),
/// then the others as held. What does not fit is carried as it is; only the carrying capacity
/// limits what a hero takes.
fn stow_goods(mut buyer: Inventory, item: &str, qty: u64) -> Result<Inventory, InventoryError> {
    let mut candidates: Vec<String> = Vec::new();
    for held in buyer.held() {
        if held.id != item && could_hold(&buyer, &held.id, item)? {
            candidates.push(held.id.clone());
        }
    }
    candidates.sort_by_key(|id| holder(id).is_some_and(|capacity| capacity.only.is_empty()));
    let mut left = qty;
    for container in &candidates {
        let fitting = room(&buyer, container, item)?.min(left);
        stow(&mut buyer, item, container, fitting)?;
        left -= fitting;
    }
    Ok(buyer)
}

/// The goods and coins a purchase of `qty` of `item` from `shop` by `buyer` moves.
struct Terms {
    entry: &'static EquipmentItem,
    price: Price,
    buyer_gives: Vec<ItemCount>,
    shop_gives: Vec<ItemCount>,
    coins_before_cp: u64,
}

/// What `buyer` pays and `shop` hands over for `qty` of `item`, checked: the shop sells it from
/// the catalog and has that many, the price is whole, the buyer can pay, the shop can make change.
fn terms(
    buyer: &Inventory,
    shop: &Inventory,
    (item, qty): (&str, u64),
) -> Result<Terms, InventoryError> {
    if qty == 0 {
        return Err(InventoryError::ZeroCount {
            item: item.to_owned(),
        });
    }
    if shop.kind != OwnerKind::Shop {
        return Err(InventoryError::NotAShop {
            owner: shop.owner.clone(),
        });
    }
    let not_for_sale = || InventoryError::NotForSale {
        shop: shop.owner.clone(),
        item: item.to_owned(),
    };
    let entry = equipment(item)
        .filter(|entry| entry.cost.is_some())
        .ok_or_else(not_for_sale)?;
    let ware = shop
        .item(item)
        .filter(|ware| ware.coin.is_none() && ware.count > 0)
        .ok_or_else(not_for_sale)?;
    if ware.count < qty {
        return Err(InventoryError::Insufficient {
            owner: shop.owner.clone(),
            item: item.to_owned(),
            have: ware.count,
            asked: qty,
        });
    }
    let price = entry
        .price_for(qty)
        .ok_or_else(|| InventoryError::UnevenBundle {
            item: item.to_owned(),
            count: qty,
            per: entry.per,
        })?;
    let coins = coin_terms(buyer, shop, price)?;
    let mut shop_gives = vec![ItemCount {
        item: item.to_owned(),
        count: qty,
    }];
    shop_gives.extend(coins.change);
    Ok(Terms {
        entry,
        price,
        buyer_gives: coins.handed,
        shop_gives,
        coins_before_cp: coins.coins_before_cp,
    })
}

/// The coins of a payment of `price` from `payer` to `payee`: the coin items the payer hands over
/// (the SRD change rule, `Coins::pay`), the coin items of the payee's exact change, and the
/// payer's coins before, in copper.
struct CoinTerms {
    handed: Vec<ItemCount>,
    change: Vec<ItemCount>,
    change_cp: u64,
    coins_before_cp: u64,
}

/// What `payer` hands `payee` for `price` and the change `payee` gives back: `insufficient_coin`
/// when the payer's coins are short, `no_change` when the payee's coins cannot make the change.
fn coin_terms(
    payer: &Inventory,
    payee: &Inventory,
    price: Price,
) -> Result<CoinTerms, InventoryError> {
    let purse = purse_of(&payer.owner, coins_in(&payer.items))?;
    let coins_before_cp = coins_cp(payer)?;
    let payment = purse.pay(price).map_err(|error| match error {
        CoinError::Insufficient { cost_cp, purse_cp } => InventoryError::InsufficientCoin {
            owner: payer.owner.clone(),
            cost_cp,
            purse_cp,
        },
        CoinError::Overflow => overflow(&payer.owner, "coins"),
    })?;
    let no_change = || InventoryError::NoChange {
        shop: payee.owner.clone(),
        change_cp: payment.change_cp,
    };
    let change = exact_coins(
        &payee.owner,
        &payee.items,
        Price::from_copper(payment.change_cp),
    )?
    .ok_or_else(no_change)?;
    Ok(CoinTerms {
        handed: coin_items(&payer.items, payment.handed),
        change,
        change_cp: payment.change_cp,
        coins_before_cp,
    })
}

/// What one payment came to: the payer's coins before and after, in copper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Paid {
    pub(super) coins_before_cp: u64,
    pub(super) coins_after_cp: u64,
}

/// `payer` pays `amount` to `payee` (a toll, a fare): the payer's coins for the payee's exact
/// change through [`exchange`], as a purchase's coins move. Refused whole: `zero_count`,
/// `insufficient_coin`, `no_change`, and `over_capacity` when `capacity` is given and the change
/// would load the payer past it.
pub(super) fn paying(
    (payer, payee): (&Inventory, &Inventory),
    amount: Price,
    why: &str,
    capacity: Option<Weight>,
) -> Result<(Inventory, Inventory, Vec<InventoryApplied>, Paid), InventoryError> {
    if amount.copper() == 0 {
        return Err(InventoryError::ZeroCount {
            item: "coins".to_owned(),
        });
    }
    let coins = coin_terms(payer, payee, amount)?;
    let (paid_from, paid_to, applied) =
        exchange((payer, &coins.handed), (payee, &coins.change), why)?;
    if let Some(capacity) = capacity {
        check_load(payer, &paid_from, capacity)?;
    }
    let paid = Paid {
        coins_before_cp: coins.coins_before_cp,
        coins_after_cp: coins_cp(&paid_from)?,
    };
    Ok((paid_from, paid_to, applied, paid))
}

/// `payer` pays `amount_cp` copper's worth of coins to `payee` ([`paying`]) in one locked
/// transaction; a hero's load is checked against their carrying capacity, an owner with no hero
/// sheet (a resident) has none to check. One `INVENTORY pay owner= to= cp= coins_before=
/// coins_after= why=` line, each change one `WORLD inventory_changed` line; a refusal changes
/// nothing and is one `INVENTORY pay_refused owner= to= cp= reason=` line. Answers the change
/// (both owners' inventories after it, and each change taken in `applied`).
pub(super) async fn pay_coins(
    app: &AppState,
    (payer, payee): (&str, &str),
    amount_cp: u64,
    why: &str,
) -> Result<super::request::Changed, InventoryError> {
    let result = async {
        let capacity = match capacity_of(app, payer).await {
            Ok(capacity) => Some(capacity),
            Err(InventoryError::NoCarryingCapacity { .. }) => None,
            Err(error) => return Err(error),
        };
        let directory = store::inventory_directory(app);
        let (from, to, words) = (payer.to_owned(), payee.to_owned(), why.to_owned());
        run_blocking(
            move || {
                operations::pay(
                    &directory,
                    (&from, &to),
                    Price::from_copper(amount_cp),
                    &words,
                    capacity,
                )
            },
            InventoryError::Stopped,
        )
        .await
    }
    .await;
    match result {
        Ok((changed, paid)) => {
            record_changed(app, &changed);
            app.record_line(format!(
                "INVENTORY pay owner={} to={} cp={amount_cp} coins_before={} coins_after={} \
                 why={}",
                logged_id(payer),
                logged_id(payee),
                paid.coins_before_cp,
                paid.coins_after_cp,
                why_id(why)
            ));
            Ok(changed)
        }
        Err(error) => {
            app.record_line(format!(
                "INVENTORY pay_refused owner={} to={} cp={amount_cp} reason={}",
                logged_id(payer),
                logged_id(payee),
                error.key()
            ));
            Err(error)
        }
    }
}

/// `buyer` buys `qty` of `item` from `shop`, as the module doc says: both inventories after it,
/// each change taken, and what it came to. Refused whole with a typed reason.
pub(super) fn purchase(
    buyer: &Inventory,
    shop: &Inventory,
    (item, qty): (&str, u64),
    capacity: Weight,
) -> Result<(Inventory, Inventory, Vec<InventoryApplied>, Bought), InventoryError> {
    let terms = terms(buyer, shop, (item, qty))?;
    let why = format!("bought from {}", shop.owner);
    let (bought_into, sold_from, mut applied) =
        exchange((buyer, &terms.buyer_gives), (shop, &terms.shop_gives), &why)?;
    let (bought_into, unpacked) = arrange(bought_into, terms.entry, (item, qty))?;
    applied.extend(unpacked);
    let weight = check_load(buyer, &bought_into, capacity)?;
    let bought = Bought {
        owner: buyer.owner.clone(),
        shop: shop.owner.clone(),
        item: item.to_owned(),
        qty,
        price: terms.price,
        coins_before_cp: terms.coins_before_cp,
        coins_after_cp: coins_cp(&bought_into)?,
        weight,
        capacity,
    };
    Ok((bought_into, sold_from, applied, bought))
}

/// What `after` carries, refused (`over_capacity`) when it passes `capacity` and is more than
/// `before` carried: a load already over is not made worse.
pub(super) fn check_load(
    before: &Inventory,
    after: &Inventory,
    capacity: Weight,
) -> Result<Weight, InventoryError> {
    let (was, _) = carried(before)?;
    let (weight, _) = carried(after)?;
    if weight > capacity && weight > was {
        return Err(InventoryError::OverCapacity {
            owner: before.owner.clone(),
            milli_pounds: weight.milli_pounds(),
            capacity_milli_pounds: capacity.milli_pounds(),
        });
    }
    Ok(weight)
}

/// `inventory`'s owner receives `count` of the catalog item `item` from no one (a starting kit),
/// arranged as a purchase is: a pack as its contents in its own container, other goods in the
/// owner's containers as far as they have room and the rest carried, and refused `over_capacity`
/// past `capacity`. An item the catalog does not list is
/// `not_in_catalog`. Answers the inventory after it, each change taken, and what it now carries.
pub(super) fn receiving(
    inventory: &Inventory,
    (item, count): (&str, u64),
    why: &str,
    capacity: Weight,
) -> Result<(Inventory, Vec<InventoryApplied>, Weight), InventoryError> {
    let entry = equipment(item).ok_or_else(|| InventoryError::NotInCatalog {
        item: item.to_owned(),
    })?;
    let add = InventoryChange::Add {
        item: item.to_owned(),
        count,
        name: Some(entry.name.to_owned()),
        look: Some(format!("{}, {}.", entry.name, why.trim())),
        usage: Some(if entry.used_up {
            ItemUse::Consumed
        } else {
            ItemUse::Kept
        }),
        price_cp: entry.price_for(1).map(Price::copper),
        why: why.to_owned(),
    };
    let (received, mut applied) = apply_changes(inventory, &[add])?;
    let (received, unpacked) = arrange(received, entry, (item, count))?;
    applied.extend(unpacked);
    let weight = check_load(inventory, &received, capacity)?;
    Ok((received, applied, weight))
}

/// The least of each coin a till holds, so its keeper can make change (`planning/town-to-tomb.md`
/// part C, Change): 20 copper and 20 silver pieces make every change below a gold piece twice
/// over, and 10 gold pieces every change below a platinum piece (SRD p.89 coin values). The SRD
/// sets no purse for a keeper; this is the table's own rule.
const TILL: [(Denomination, u64); 3] = [
    (Denomination::Cp, 20),
    (Denomination::Sp, 20),
    (Denomination::Gp, 10),
];

/// `items` with each coin of [`TILL`] it holds less of topped up to the till's count: added to the
/// first item of that coin, else as a new standard coin item ([`standard_coin`]); a coin whose
/// standard id names another item is left as it is. Answers the items and the copper added.
pub(super) fn with_till(mut items: Vec<InventoryItem>) -> (Vec<InventoryItem>, u64) {
    let mut added_cp = 0;
    for (coin, least) in TILL {
        let held: u64 = coins_in(&items)
            .filter(|(held, _)| *held == coin)
            .map(|(_, count)| count)
            .sum();
        let wanted = least.saturating_sub(held);
        if wanted == 0 {
            continue;
        }
        let (id, name) = standard_coin(coin);
        match items.iter().position(|item| item.coin == Some(coin)) {
            Some(at) => items[at].count += wanted,
            None if items.iter().any(|item| item.id == id) => continue,
            None => items.push(InventoryItem {
                id: id.to_owned(),
                name: name.to_owned(),
                look: format!("{name}, the till's change."),
                usage: ItemUse::Consumed,
                count: wanted,
                price_cp: None,
                coin: Some(coin),
            }),
        }
        added_cp += wanted * coin.copper();
    }
    (items, added_cp)
}

/// The coin a standard coin item id names (`gold-piece`), if it is one.
pub fn standard_coin_of(id: &str) -> Option<Denomination> {
    Denomination::ALL
        .into_iter()
        .find(|coin| standard_coin(*coin).0 == id)
}

/// The id of the standard coin item of `coin` (`copper-piece`).
pub(super) fn standard_coin_id(coin: Denomination) -> &'static str {
    standard_coin(coin).0
}

/// The standard coin item of `coin` (SRD p.89 coin names): its id, and its name.
fn standard_coin(coin: Denomination) -> (&'static str, &'static str) {
    match coin {
        Denomination::Cp => ("copper-piece", "Copper Pieces"),
        Denomination::Sp => ("silver-piece", "Silver Pieces"),
        Denomination::Ep => ("electrum-piece", "Electrum Pieces"),
        Denomination::Gp => ("gold-piece", "Gold Pieces"),
        Denomination::Pp => ("platinum-piece", "Platinum Pieces"),
    }
}

/// `inventory`'s owner gains `count` coins of `coin`: added to the first coin item of that coin
/// they hold (a count of none included), else to a new standard coin item ([`standard_coin`]).
/// Coins weigh 1/50 lb each (SRD p.89), so the load is checked as for any gain (`over_capacity`).
/// An item with the standard id that is not that coin is `coin_mismatch`.
pub(super) fn gaining_coins(
    inventory: &Inventory,
    (coin, count): (Denomination, u64),
    why: &str,
    capacity: Weight,
) -> Result<(Inventory, Vec<InventoryApplied>, Weight), InventoryError> {
    let (standard, name) = standard_coin(coin);
    let held = inventory.items.iter().find(|item| item.coin == Some(coin));
    let id = held.map_or(standard, |item| item.id.as_str()).to_owned();
    if held.is_none() && inventory.item(&id).is_some() {
        return Err(InventoryError::CoinMismatch {
            owner: inventory.owner.clone(),
            item: id,
        });
    }
    let add = InventoryChange::Add {
        item: id.clone(),
        count,
        name: Some(name.to_owned()),
        look: Some(format!("{name}, {}.", why.trim())),
        usage: Some(ItemUse::Consumed),
        price_cp: None,
        why: why.to_owned(),
    };
    let (mut gained, applied) = apply_changes(inventory, &[add])?;
    if let Some(item) = gained.items.iter_mut().find(|item| item.id == id) {
        item.coin = Some(coin);
    }
    let weight = check_load(inventory, &gained, capacity)?;
    Ok((gained, applied, weight))
}

/// Hero `owner` gains `count` coins of `coin` ([`gaining_coins`]): starting gold, a quest's
/// advance. One `INVENTORY gain_coins owner= coin= count= why= weight= capacity=` line (`why` as an
/// id), each change one `WORLD inventory_changed` line; a refusal changes nothing and is one
/// `INVENTORY gain_coins_refused owner= coin= count= reason=` line.
pub(crate) async fn gain_coins(
    app: &AppState,
    owner: &str,
    coin: Denomination,
    count: u64,
    why: &str,
) -> Result<Vec<InventoryApplied>, InventoryError> {
    let words = why.to_owned();
    let result = take_in(app, owner, move |inventory, capacity| {
        gaining_coins(inventory, (coin, count), &words, capacity)
    })
    .await;
    match result {
        Ok((changed, weight, capacity)) => {
            app.record_line(format!(
                "INVENTORY gain_coins owner={} coin={} count={count} why={} weight={} capacity={}",
                logged_id(owner),
                coin.key(),
                why_id(why),
                pounds(weight),
                pounds(capacity)
            ));
            Ok(changed.applied)
        }
        Err(error) => {
            app.record_line(format!(
                "INVENTORY gain_coins_refused owner={} coin={} count={count} reason={}",
                logged_id(owner),
                coin.key(),
                error.key()
            ));
            Err(error)
        }
    }
}

/// What `change` makes of hero `owner`'s inventory with their carrying capacity, taken in whole
/// under the inventory lock (`operations::take_in`), each change one `WORLD inventory_changed`
/// line. Answers the change, what the owner now carries, and their capacity.
async fn take_in(
    app: &AppState,
    owner: &str,
    change: impl FnOnce(
        &Inventory,
        Weight,
    ) -> Result<(Inventory, Vec<InventoryApplied>, Weight), InventoryError>
    + Send
    + 'static,
) -> Result<(super::request::Changed, Weight, Weight), InventoryError> {
    let capacity = capacity_of(app, owner).await?;
    let directory = store::inventory_directory(app);
    let owner_id = owner.to_owned();
    let (changed, weight) = run_blocking(
        move || operations::take_in(&directory, &owner_id, capacity, change),
        InventoryError::Stopped,
    )
    .await?;
    record_changed(app, &changed);
    Ok((changed, weight, capacity))
}

/// `why` for a log line as an id: its item id (`srd_equipment_prices::item_id`), with every run of
/// characters that are not ASCII letters or digits one `-` ("from the hero's starting equipment"
/// is `from-the-heros-starting-equipment`), cut to the id's length; `-` when nothing is left.
pub(super) fn why_id(why: &str) -> String {
    let mut id = String::new();
    for character in item_id(why).chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            id.push(character);
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let mut id = id.trim_end_matches('-').to_owned();
    id.truncate(df_content::world_bible::MAXIMUM_ID_CHARS);
    let id = id.trim_end_matches('-');
    if id.is_empty() {
        "-".to_owned()
    } else {
        id.to_owned()
    }
}

/// Hero `owner` receives `count` of `item` ([`receiving`]): the starting kit's way in, with no
/// coins. One `INVENTORY receive owner= item= qty= why= weight= capacity=` line (`why` as an id:
/// its words joined by `-`, `?` when that is no id), each change one `WORLD inventory_changed`
/// line; a refusal is one `INVENTORY receive_refused owner= item= qty= reason=` line.
pub(crate) async fn receive(
    app: &AppState,
    owner: &str,
    item: &str,
    count: u64,
    why: &str,
) -> Result<Vec<InventoryApplied>, InventoryError> {
    let (item_id, words) = (item.to_owned(), why.to_owned());
    let result = take_in(app, owner, move |inventory, capacity| {
        receiving(inventory, (&item_id, count), &words, capacity)
    })
    .await;
    match result {
        Ok((changed, weight, capacity)) => {
            app.record_line(format!(
                "INVENTORY receive owner={} item={} qty={count} why={} weight={} capacity={}",
                logged_id(owner),
                logged_id(item),
                why_id(why),
                pounds(weight),
                pounds(capacity)
            ));
            Ok(changed.applied)
        }
        Err(error) => {
            app.record_line(format!(
                "INVENTORY receive_refused owner={} item={} qty={count} reason={}",
                logged_id(owner),
                logged_id(item),
                error.key()
            ));
            Err(error)
        }
    }
}

/// The carrying capacity of hero `owner` from their sheet (Strength and size, SRD p.178), or
/// `no_carrying_capacity` when they have no sheet or no admitted size.
pub(super) async fn capacity_of(app: &AppState, owner: &str) -> Result<Weight, InventoryError> {
    let none = || InventoryError::NoCarryingCapacity {
        owner: owner.to_owned(),
    };
    let summary = crate::creation::hero_summary(app, owner)
        .await
        .map_err(|_| none())?
        .ok_or_else(none)?;
    let size = summary.size.ok_or_else(none)?;
    Ok(carrying_capacity(summary.strength, size))
}

/// `buyer` (a hero's character id) buys `qty` of `item` from `shop` (its keeper's id): coins and
/// goods move together through the deal's exchange, or nothing moves. One `INVENTORY buy owner=
/// item= qty= from= to= coins_before= coins_after= weight= capacity=` line (coins in copper,
/// weights in pounds), each change one `WORLD inventory_changed` line; a refusal is one `INVENTORY
/// buy_refused owner= item= qty= from= reason=` line.
pub(crate) async fn buy(
    app: &AppState,
    buyer: &str,
    shop: &str,
    item: &str,
    qty: u32,
) -> Result<Bought, InventoryError> {
    let result = async {
        let capacity = capacity_of(app, buyer).await?;
        let directory = store::inventory_directory(app);
        let (buyer_id, shop_id, item_id) = (buyer.to_owned(), shop.to_owned(), item.to_owned());
        run_blocking(
            move || {
                operations::buy(
                    &directory,
                    (&buyer_id, &shop_id),
                    (&item_id, u64::from(qty)),
                    capacity,
                )
            },
            InventoryError::Stopped,
        )
        .await
    }
    .await;
    match result {
        Ok((changed, bought)) => {
            record_changed(app, &changed);
            app.record_line(format!(
                "INVENTORY buy owner={} item={} qty={qty} from={} to={} coins_before={} \
                 coins_after={} weight={} capacity={}",
                logged_id(buyer),
                logged_id(item),
                logged_id(shop),
                logged_id(buyer),
                bought.coins_before_cp,
                bought.coins_after_cp,
                pounds(bought.weight),
                pounds(bought.capacity)
            ));
            Ok(bought)
        }
        Err(error) => {
            app.record_line(format!(
                "INVENTORY buy_refused owner={} item={} qty={qty} from={} reason={}",
                logged_id(buyer),
                logged_id(item),
                logged_id(shop),
                error.key()
            ));
            Err(error)
        }
    }
}

/// A weight for a log line: pounds with their fraction, no unit.
fn pounds(weight: Weight) -> String {
    weight.to_string().trim_end_matches(" lb").to_owned()
}

/// One line of a bag: `count` of `item`, carried (`container` none) or in a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BagLine {
    pub item: String,
    pub name: String,
    pub count: u64,
    /// One unit's weight, and these units'; none for an item the catalog does not weigh.
    pub unit_weight: Option<Weight>,
    pub weight: Option<Weight>,
    pub container: Option<String>,
}

/// How full a container is that takes only some things, in shares: a quiver holds 20 shares, an
/// arrow is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ShareLoad {
    pub used: u64,
    pub most: u64,
}

/// One container of a bag: what its contents weigh against what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerLoad {
    pub container: String,
    pub name: String,
    /// How many of this container the owner has; the capacity is all of them together.
    pub count: u64,
    pub load: Weight,
    /// The weight it holds (backpack, pouch, sack, basket).
    pub weight_capacity: Option<Weight>,
    /// For one that takes only some things (quiver, crossbow bolt case, map or scroll case).
    pub shares: Option<ShareLoad>,
}

/// A coin item of a bag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoinLine {
    pub item: String,
    pub name: String,
    pub coin: Denomination,
    pub count: u64,
}

/// What a hero carries: each item by where it is, each container's load, the coins, and the total
/// weight against the carrying capacity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BagView {
    pub owner: String,
    pub lines: Vec<BagLine>,
    pub containers: Vec<ContainerLoad>,
    pub coins: Vec<CoinLine>,
    pub coins_cp: u64,
    /// Everything carried, coins included (50 coins weigh a pound, SRD p.89).
    pub weight: Weight,
    /// The ids of held items the catalog gives no weight, left out of `weight`.
    pub unweighed: Vec<String>,
    /// Strength × 15 lb (SRD p.178); none when the hero has no sheet or no admitted size.
    pub capacity: Option<Weight>,
}

/// The bag of `inventory` with the carrying capacity `capacity`.
pub(super) fn view_of_bag(
    inventory: &Inventory,
    capacity: Option<Weight>,
) -> Result<BagView, InventoryError> {
    let mut lines = Vec::new();
    let mut coins = Vec::new();
    for item in inventory.held() {
        if let Some(coin) = item.coin {
            coins.push(CoinLine {
                item: item.id.clone(),
                name: item.name.clone(),
                coin,
                count: item.count,
            });
            continue;
        }
        let line = |count: u64, container: Option<String>| {
            Ok::<_, InventoryError>(BagLine {
                item: item.id.clone(),
                name: item.name.clone(),
                count,
                unit_weight: unit_weight(item),
                weight: weight_of(inventory, item, count)?,
                container,
            })
        };
        let mut loose = item.count;
        for stowed in inventory
            .stowed
            .iter()
            .filter(|stowed| stowed.item == item.id)
        {
            loose -= stowed.count.min(loose);
            lines.push(line(stowed.count, Some(stowed.container.clone()))?);
        }
        if loose > 0 {
            lines.push(line(loose, None)?);
        }
    }
    let mut containers = Vec::new();
    for item in inventory.held() {
        let Some(capacity) = holder(&item.id) else {
            continue;
        };
        let shares = if capacity.only.is_empty() {
            None
        } else {
            let (used, most) = shares(inventory, &item.id, capacity.only)?;
            Some(ShareLoad { used, most })
        };
        containers.push(ContainerLoad {
            container: item.id.clone(),
            name: item.name.clone(),
            count: item.count,
            load: load_of(inventory, &item.id)?,
            weight_capacity: weight_limit(inventory, &item.id)?,
            shares,
        });
    }
    let (weight, unweighed) = carried(inventory)?;
    Ok(BagView {
        owner: inventory.owner.clone(),
        lines,
        containers,
        coins,
        coins_cp: coins_cp(inventory)?,
        weight,
        unweighed,
        capacity,
    })
}

/// What hero `hero` carries ([`BagView`]); `unknown_owner` when they have no inventory. A hero
/// with no sheet or admitted size has no capacity in the view. Read-only.
pub(crate) async fn bag_view(app: &AppState, hero: &str) -> Result<BagView, InventoryError> {
    let capacity = match capacity_of(app, hero).await {
        Ok(capacity) => Some(capacity),
        Err(InventoryError::NoCarryingCapacity { .. }) => None,
        Err(error) => return Err(error),
    };
    let directory = store::inventory_directory(app);
    let owner = hero.to_owned();
    let inventory = run_blocking(
        move || store::read_inventory(&directory, &owner),
        InventoryError::Stopped,
    )
    .await?;
    view_of_bag(&inventory, capacity)
}

/// One ware of a shop: how many it has, its price for `per` units and one unit's weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StockLine {
    pub item: String,
    pub name: String,
    pub in_stock: u64,
    /// What `per` units cost: the catalog's for a catalog item, the shop's own for another.
    pub price: Price,
    pub per: u32,
    pub unit_weight: Option<Weight>,
    /// Bought by a tap (`buy`): a catalog item. Another ware is bought by talking.
    pub buyable: bool,
}

/// A shop's stock, for the phone's Shop page. The shop's id is its keeper's character id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShopView {
    pub keeper: String,
    /// The coin the shop counts in: the most valuable coin it holds.
    pub coin: Option<Denomination>,
    pub lines: Vec<StockLine>,
}

/// The stock of `inventory`, a shop.
pub(super) fn view_of_shop(inventory: &Inventory) -> Result<ShopView, InventoryError> {
    if inventory.kind != OwnerKind::Shop {
        return Err(InventoryError::NotAShop {
            owner: inventory.owner.clone(),
        });
    }
    let lines = inventory
        .held()
        .filter(|item| item.coin.is_none())
        .filter_map(|item| {
            let catalog = equipment(&item.id);
            let line = match catalog.and_then(|entry| Some((entry, entry.cost?))) {
                Some((entry, cost)) => StockLine {
                    item: item.id.clone(),
                    name: item.name.clone(),
                    in_stock: item.count,
                    price: cost,
                    per: entry.per,
                    unit_weight: Some(entry.unit_weight()),
                    buyable: true,
                },
                None => StockLine {
                    item: item.id.clone(),
                    name: item.name.clone(),
                    in_stock: item.count,
                    price: Price::from_copper(item.price_cp?),
                    per: 1,
                    unit_weight: catalog.map(EquipmentItem::unit_weight),
                    buyable: false,
                },
            };
            Some(line)
        })
        .collect();
    Ok(ShopView {
        keeper: inventory.owner.clone(),
        coin: counting_coin(&inventory.items),
        lines,
    })
}

/// The stock of shop `shop` ([`ShopView`]); `unknown_owner` or `not_a_shop` otherwise. Read-only.
pub(crate) async fn shop_view(app: &AppState, shop: &str) -> Result<ShopView, InventoryError> {
    let directory = store::inventory_directory(app);
    let owner = shop.to_owned();
    let inventory = run_blocking(
        move || store::read_inventory(&directory, &owner),
        InventoryError::Stopped,
    )
    .await?;
    view_of_shop(&inventory)
}
