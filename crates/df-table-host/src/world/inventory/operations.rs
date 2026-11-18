//! What each inventory operation does to the store, with no HTTP and no logging: the routes and the
//! judged path call these, so there is one validation and one way to change an inventory.

use std::path::Path;

use df_playground::kit::ledger::{Ledger, LedgerItem, LedgerKind};

use super::bag::{Bought, DealPaid, Paid, check_load, deal_transfers, paying, purchase, with_till};
use super::request::{
    Changed, History, HistoryEntry, HistoryQuery, InventoryError, InventoryView, OwnerList,
    PriceRequest, Priced, SetRequest, TransferRequest,
};
use super::store::{read_index, read_inventory, transact};
use super::{
    Inventory, InventoryApplied, InventoryChange, ItemCount, OwnerKind, apply_changes, check_owner,
    exchange, transfer,
};
use crate::event_log::EventLog;
use df_rules::srd_equipment_prices::Weight;

/// `inventory` with the list panel's rows of it for the page: a loot row for each item the owner
/// has at least one of, in the order they are held.
pub(super) fn view_of(inventory: Inventory) -> InventoryView {
    let ledger = Ledger {
        title: None,
        icon: None,
        kind: LedgerKind::Loot,
        items: inventory
            .held()
            .map(|item| LedgerItem {
                id: item.id.clone(),
                name: item.name.clone(),
                quantity: Some(format!("x{}", item.count)),
                price: None,
                picture: None,
                icon: None,
                rarity: None,
                taken: false,
                highlight: false,
            })
            .collect(),
        framed: true,
    };
    InventoryView { inventory, ledger }
}

/// Every owner, from the index.
pub(super) fn list(directory: &Path) -> Result<OwnerList, InventoryError> {
    Ok(OwnerList {
        owners: read_index(directory)?,
    })
}

/// The inventory of `owner`.
pub(super) fn get(directory: &Path, owner: &str) -> Result<InventoryView, InventoryError> {
    check_owner(owner)?;
    read_inventory(directory, owner).map(view_of)
}

/// Makes `request.items` the whole inventory of `request.owner`, which is made when it is new.
pub(super) fn set(directory: &Path, request: SetRequest) -> Result<Changed, InventoryError> {
    let inventory = Inventory::new(&request.owner, request.kind, request.items)?;
    transact(directory, &[request.owner.as_str()], |_| {
        Ok((
            vec![inventory.clone()],
            Changed {
                applied: Vec::new(),
                owners: vec![view_of(inventory)],
            },
        ))
    })
}

/// Gives `owner` a till that can make change ([`with_till`]): its coins topped up to the till's
/// counts, and an inventory made, a shop's with no wares, when it has none (a fare-taker). Answers
/// the inventory after it and the copper added.
pub(super) fn till(directory: &Path, owner: &str) -> Result<(Changed, u64), InventoryError> {
    check_owner(owner)?;
    transact(directory, &[owner], |stored| {
        let mut inventory = match stored.into_iter().next().flatten() {
            Some(inventory) => inventory,
            None => Inventory::new(owner, OwnerKind::Shop, Vec::new())?,
        };
        let (items, added_cp) = with_till(std::mem::take(&mut inventory.items));
        inventory.items = items;
        inventory.check()?;
        Ok((
            vec![inventory.clone()],
            (
                Changed {
                    applied: Vec::new(),
                    owners: vec![view_of(inventory)],
                },
                added_cp,
            ),
        ))
    })
}

/// What a line moves between its speaker and the ground of the place they are at (`owner`, the
/// place's ground, an owner of its own): what the speaker leaves there (thrown, dropped, set down)
/// and what they pick back up from it.
#[derive(Debug, Clone, Copy)]
pub(super) struct GroundMove<'a> {
    pub(super) owner: &'a str,
    pub(super) left: &'a [ItemCount],
    pub(super) picked: &'a [ItemCount],
}

/// `changes` to the inventory of `owner`, all taken or none: refused `over_capacity` when
/// `capacity` is given and they load the owner past it (`check_load`). With `ground`, what the
/// changes leave at the place goes onto its ground (with the owner's words, price and coin; the
/// ground is made when it holds nothing yet) and what they pick up comes off it, in the same
/// transaction: both written, or neither.
pub(super) fn change(
    directory: &Path,
    owner: &str,
    changes: &[InventoryChange],
    capacity: Option<Weight>,
    ground: Option<GroundMove<'_>>,
) -> Result<Changed, InventoryError> {
    check_owner(owner)?;
    let mut owners = vec![owner];
    if let Some(ground) = &ground {
        check_owner(ground.owner)?;
        owners.push(ground.owner);
    }
    transact(directory, &owners, |stored| {
        let mut stored = stored.into_iter();
        let Some(Some(inventory)) = stored.next() else {
            return Err(InventoryError::UnknownOwner {
                owner: owner.to_owned(),
            });
        };
        let (after, mut applied) = apply_changes(&inventory, changes)?;
        if let Some(capacity) = capacity {
            check_load(&inventory, &after, capacity)?;
        }
        let Some(ground) = ground else {
            return Ok((
                vec![after.clone()],
                Changed {
                    applied,
                    owners: vec![view_of(after)],
                },
            ));
        };
        let lying = match stored.next().flatten() {
            Some(lying) => lying,
            None => Inventory::new(ground.owner, OwnerKind::Container, Vec::new())?,
        };
        let mut moves = Vec::new();
        for left in ground.left {
            let words = inventory
                .item(&left.item)
                .ok_or_else(|| InventoryError::UnknownItem {
                    owner: owner.to_owned(),
                    item: left.item.clone(),
                })?;
            moves.push(InventoryChange::Add {
                item: left.item.clone(),
                count: left.count,
                name: Some(words.name.clone()),
                look: Some(words.look.clone()),
                usage: Some(words.usage),
                price_cp: words.price_cp,
                why: format!("left by {owner}"),
            });
        }
        moves.extend(ground.picked.iter().map(|picked| InventoryChange::Remove {
            item: picked.item.clone(),
            count: picked.count,
            why: format!("picked up by {owner}"),
        }));
        let (mut lying_after, lying_applied) = apply_changes(&lying, &moves)?;
        // Money stays money where it lies.
        for left in ground.left {
            let coin = inventory.item(&left.item).and_then(|held| held.coin);
            if let Some(lies) = lying_after
                .items
                .iter_mut()
                .find(|held| held.id == left.item)
            {
                lies.coin = coin;
            }
        }
        applied.extend(lying_applied);
        Ok((
            vec![after.clone(), lying_after.clone()],
            Changed {
                applied,
                owners: vec![view_of(after), view_of(lying_after)],
            },
        ))
    })
}

/// Prices `request.item` of `request.owner` at `request.price_cp` when it has no price yet; a price
/// it has is left alone.
pub(super) fn price(directory: &Path, request: &PriceRequest) -> Result<Priced, InventoryError> {
    check_owner(&request.owner)?;
    transact(directory, &[request.owner.as_str()], |stored| {
        let Some(Some(mut inventory)) = stored.into_iter().next() else {
            return Err(InventoryError::UnknownOwner {
                owner: request.owner.clone(),
            });
        };
        let item = inventory
            .items
            .iter_mut()
            .find(|item| item.id == request.item)
            .ok_or_else(|| InventoryError::UnknownItem {
                owner: request.owner.clone(),
                item: request.item.clone(),
            })?;
        let priced = match item.price_cp {
            Some(price_cp) => Priced {
                price_cp,
                set: false,
            },
            None => {
                item.price_cp = Some(request.price_cp);
                Priced {
                    price_cp: request.price_cp,
                    set: true,
                }
            }
        };
        Ok((vec![inventory], priced))
    })
}

/// `request.count` of `request.item` from one owner to another, both sides changed or neither.
pub(super) fn move_between(
    directory: &Path,
    request: &TransferRequest,
) -> Result<Changed, InventoryError> {
    check_owner(&request.from)?;
    check_owner(&request.to)?;
    if request.from == request.to {
        return Err(InventoryError::SameOwner {
            owner: request.from.clone(),
        });
    }
    transact(directory, &[&request.from, &request.to], |stored| {
        let mut stored = stored.into_iter();
        let (from, to) = (stored.next().flatten(), stored.next().flatten());
        let from = from.ok_or_else(|| InventoryError::UnknownOwner {
            owner: request.from.clone(),
        })?;
        let to = to.ok_or_else(|| InventoryError::UnknownOwner {
            owner: request.to.clone(),
        })?;
        let (from_after, to_after, applied) = transfer(
            &from,
            &to,
            (request.item.as_str(), request.count),
            &request.why,
        )?;
        Ok((
            vec![from_after.clone(), to_after.clone()],
            Changed {
                applied,
                owners: vec![view_of(from_after), view_of(to_after)],
            },
        ))
    })
}

/// One side of a deal to settle: whose it is, what they hand over, and their carrying capacity
/// when they have one (a hero; a resident has none to check).
#[derive(Debug, Clone, Copy)]
pub(super) struct DealSide<'a> {
    pub(super) owner: &'a str,
    pub(super) gives: &'a [ItemCount],
    pub(super) capacity: Option<Weight>,
}

/// A deal between two owners, both sides changed or neither: the goods of `a`'s list move from
/// `a` to `b` and those of `b`'s list from `b` to `a`, and the coins named each way are paid as
/// one payment with change ([`deal_transfers`]), all through one [`exchange`]; refused
/// `over_capacity` when it loads a side past its capacity ([`settled`]). Answers the change (the
/// inventories of `a` and `b` after it, in that order) and the payment, when coins moved.
pub(super) fn settle(
    directory: &Path,
    a: DealSide<'_>,
    b: DealSide<'_>,
    why: &str,
) -> Result<(Changed, Option<DealPaid>), InventoryError> {
    check_owner(a.owner)?;
    check_owner(b.owner)?;
    if a.owner == b.owner {
        return Err(InventoryError::SameOwner {
            owner: a.owner.to_owned(),
        });
    }
    transact(directory, &[a.owner, b.owner], |stored| {
        let (first, second) = both(stored, a.owner, b.owner)?;
        let (first, second, applied, paid) = settled((&first, a), (&second, b), why)?;
        Ok((
            vec![first.clone(), second.clone()],
            (
                Changed {
                    applied,
                    owners: vec![view_of(first), view_of(second)],
                },
                paid,
            ),
        ))
    })
}

/// What a deal between `a` and `b` makes of their inventories ([`deal_transfers`] through
/// [`exchange`]), with nothing written: both after it, each change, and the payment. A side with
/// a carrying capacity is refused `over_capacity` when what it receives loads it past that
/// capacity (`check_load`, as a purchase is: Strength x 15 lb, SRD p.178).
pub(super) fn settled(
    (a, a_side): (&Inventory, DealSide<'_>),
    (b, b_side): (&Inventory, DealSide<'_>),
    why: &str,
) -> Result<
    (
        Inventory,
        Inventory,
        Vec<InventoryApplied>,
        Option<DealPaid>,
    ),
    InventoryError,
> {
    let moved = deal_transfers((a, a_side.gives), (b, b_side.gives))?;
    let (first, second, applied) = exchange((a, &moved.a_gives), (b, &moved.b_gives), why)?;
    for (before, after, capacity) in [(a, &first, a_side.capacity), (b, &second, b_side.capacity)] {
        if let Some(capacity) = capacity {
            check_load(before, after, capacity)?;
        }
    }
    Ok((first, second, applied, moved.paid))
}

/// The two stored inventories of a transaction over `a` and `b`, each refused when it has none.
fn both(
    stored: Vec<Option<Inventory>>,
    a: &str,
    b: &str,
) -> Result<(Inventory, Inventory), InventoryError> {
    let mut stored = stored.into_iter();
    let unknown = |owner: &str| InventoryError::UnknownOwner {
        owner: owner.to_owned(),
    };
    let first = stored.next().flatten().ok_or_else(|| unknown(a))?;
    let second = stored.next().flatten().ok_or_else(|| unknown(b))?;
    Ok((first, second))
}

/// `buyer` buys `qty` of `item` from `shop` ([`purchase`]) under the inventory lock: both sides
/// written, or neither.
pub(super) fn buy(
    directory: &Path,
    (buyer, shop): (&str, &str),
    (item, qty): (&str, u64),
    capacity: Weight,
) -> Result<(Changed, Bought), InventoryError> {
    check_owner(buyer)?;
    check_owner(shop)?;
    if buyer == shop {
        return Err(InventoryError::SameOwner {
            owner: buyer.to_owned(),
        });
    }
    transact(directory, &[buyer, shop], |stored| {
        let (buying, selling) = both(stored, buyer, shop)?;
        let (bought_into, sold_from, applied, bought) =
            purchase(&buying, &selling, (item, qty), capacity)?;
        Ok((
            vec![bought_into.clone(), sold_from.clone()],
            (
                Changed {
                    applied,
                    owners: vec![view_of(bought_into), view_of(sold_from)],
                },
                bought,
            ),
        ))
    })
}

/// `payer` pays `amount` to `payee` ([`paying`]) under the inventory lock: both sides written, or
/// neither.
pub(super) fn pay(
    directory: &Path,
    (payer, payee): (&str, &str),
    amount: df_rules::coins::Price,
    why: &str,
    capacity: Option<Weight>,
) -> Result<(Changed, Paid), InventoryError> {
    check_owner(payer)?;
    check_owner(payee)?;
    if payer == payee {
        return Err(InventoryError::SameOwner {
            owner: payer.to_owned(),
        });
    }
    transact(directory, &[payer, payee], |stored| {
        let (from, to) = both(stored, payer, payee)?;
        let (paid_from, paid_to, applied, paid) = paying((&from, &to), amount, why, capacity)?;
        Ok((
            vec![paid_from.clone(), paid_to.clone()],
            (
                Changed {
                    applied,
                    owners: vec![view_of(paid_from), view_of(paid_to)],
                },
                paid,
            ),
        ))
    })
}

/// `owner` takes in what `change` makes of their inventory with their carrying capacity (a kit's
/// item, a purse of coins) under the inventory lock: written whole or not at all. Answers the
/// change and what the owner now carries.
pub(super) fn take_in(
    directory: &Path,
    owner: &str,
    capacity: Weight,
    change: impl FnOnce(
        &Inventory,
        Weight,
    ) -> Result<(Inventory, Vec<InventoryApplied>, Weight), InventoryError>,
) -> Result<(Changed, Weight), InventoryError> {
    check_owner(owner)?;
    transact(directory, &[owner], |stored| {
        let Some(Some(inventory)) = stored.into_iter().next() else {
            return Err(InventoryError::UnknownOwner {
                owner: owner.to_owned(),
            });
        };
        let (received, applied, weight) = change(&inventory, capacity)?;
        Ok((
            vec![received.clone()],
            (
                Changed {
                    applied,
                    owners: vec![view_of(received)],
                },
                weight,
            ),
        ))
    })
}

/// The change lines the host's log still holds (`WORLD inventory_changed` and `WORLD
/// inventory_set`), oldest first: those of `query.owner` when given, the newest `query.limit`.
pub(super) fn history(log: &EventLog, query: &HistoryQuery) -> Result<History, InventoryError> {
    if let Some(owner) = &query.owner {
        check_owner(owner)?;
    }
    let mut entries: Vec<HistoryEntry> = log
        .after_sequence_in(0, usize::MAX, "WORLD")
        .into_iter()
        .filter(|line| {
            let mut words = line.text.split(' ').skip(1);
            matches!(words.next(), Some("inventory_changed" | "inventory_set"))
                && query.owner.as_ref().is_none_or(|owner| {
                    line.text
                        .split(' ')
                        .any(|word| word.strip_prefix("owner=") == Some(owner.as_str()))
                })
        })
        .map(|line| HistoryEntry {
            sequence: line.sequence,
            at_ms: line.at_ms,
            line: line.text,
        })
        .collect();
    if let Some(limit) = query.limit {
        entries.drain(..entries.len().saturating_sub(limit));
    }
    Ok(History { entries })
}
