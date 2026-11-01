//! Cold pinned identities reload from the book's append-only log, with complete frozen identity.

use super::*;

/// Bounds a frozen dispatch route/tariff record independently of run length.
pub(super) const MAXIMUM_PINNED_LINE_BYTES: usize = 256 * 1024;
/// Leaves room for all fifteen u64 quantity/money lines and settlement/revision metadata.
pub(super) const PINNED_EVENT_HEADROOM_BYTES: usize = 8 * 1024;

pub(super) enum Resolution {
    Stored,
    Duplicate,
    Conflict,
}

pub(super) fn price_basis(table: &PriceTable, spec: &CallSpec) -> Value {
    let rows: Vec<Value> = table
        .json()
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["provider"] == spec.provider.label() && row["model"] == spec.model)
        .cloned()
        .collect();
    json!({"schema": prices::PRICING_SCHEMA, "currency": "USD", "prices": rows})
}

pub(super) fn prices(basis: &Value) -> Result<PriceTable, PinnedError> {
    PriceTable::parse(&basis.to_string()).map_err(|_| PinnedError::PriceBasisUnavailable)
}

pub(super) fn matches_route(slot: &PinnedSlot, spec: &CallSpec) -> bool {
    match slot {
        PinnedSlot::Open(stored, _) => &stored.spec == spec,
        PinnedSlot::Unsent(stored, _) => stored.as_ref().is_none_or(|stored| stored == spec),
        PinnedSlot::Settled(record) => {
            let event = &record.event;
            event.provider == spec.provider
                && event.model == spec.model
                && event.cause == spec.cause
                && event.detail == word(&spec.detail)
                && record.maxima == spec.maxima
                && spec.follow_ons.is_empty()
        }
    }
}

fn same_frozen(first: &PinnedRecord, next: &PinnedLogLine) -> bool {
    let (a, b) = (&first.event, &next.event);
    a.call_id == b.call_id
        && a.run == b.run
        && a.session == b.session
        && a.provider == b.provider
        && a.model == b.model
        && a.cause == b.cause
        && a.detail == b.detail
        && first.maxima == next.maxima
        && first.price_basis == next.price_basis
}

pub(super) fn resolve(
    resolved: &mut BTreeMap<String, PinnedSlot>,
    line: PinnedReplayLine,
) -> Resolution {
    match line {
        PinnedReplayLine::Open(next) => {
            if !next.pinned_open
                || !is_pinned_call_id(&next.call_id)
                || prices(&next.price_basis).is_err()
            {
                return Resolution::Conflict;
            }
            match resolved.get(&next.call_id) {
                None => {
                    resolved.insert(
                        next.call_id.clone(),
                        PinnedSlot::Open(next, Arc::new(AtomicBool::new(true))),
                    );
                    Resolution::Stored
                }
                Some(PinnedSlot::Open(first, _))
                    if first.run == next.run
                        && first.session == next.session
                        && first.spec == next.spec
                        && first.price_basis == next.price_basis =>
                {
                    Resolution::Duplicate
                }
                Some(PinnedSlot::Settled(first))
                    if first.event.run == next.run
                        && first.event.session == next.session
                        && first.price_basis == next.price_basis
                        && matches_route(&PinnedSlot::Settled(first.clone()), &next.spec) =>
                {
                    Resolution::Duplicate
                }
                Some(PinnedSlot::Unsent(spec, _)) if spec.as_ref() == Some(&next.spec) => {
                    Resolution::Duplicate
                }
                _ => Resolution::Conflict,
            }
        }
        PinnedReplayLine::Settled(mut next) => {
            if !next.event.is_consistent()
                || !is_pinned_call_id(&next.event.call_id)
                || (!next.price_basis.is_null()
                    && PriceTable::parse(&next.price_basis.to_string()).is_err())
            {
                return Resolution::Conflict;
            }
            if !next.price_basis.is_null() {
                let quantities: Vec<_> = next
                    .event
                    .lines
                    .iter()
                    .map(|line| (line.category, line.quantity))
                    .collect();
                let Ok(prices) = prices(&next.price_basis) else {
                    return Resolution::Conflict;
                };
                let (lines, micro_usd) =
                    price_isolated(&prices, next.event.provider, &next.event.model, &quantities);
                if lines != next.event.lines || micro_usd != next.event.micro_usd {
                    return Resolution::Conflict;
                }
            }
            if next.maxima.is_empty() {
                next.maxima = next
                    .event
                    .lines
                    .iter()
                    .map(|line| (line.category, line.quantity))
                    .collect();
            }
            if next.event.source == UsageSource::Unknown {
                let quantities: Vec<_> = next
                    .event
                    .lines
                    .iter()
                    .map(|line| (line.category, line.quantity))
                    .collect();
                if !same_report(
                    next.event.source,
                    next.event.usable,
                    &next.maxima,
                    &next.event,
                ) || quantities.is_empty()
                {
                    return Resolution::Conflict;
                }
            }
            next.maxima.sort();
            let id = next.event.call_id.clone();
            let replaced = match resolved.get(&id) {
                None => false,
                Some(PinnedSlot::Settled(first)) => {
                    if !same_frozen(first, &next) {
                        return Resolution::Conflict;
                    }
                    if next.pinned_revision < first.revision {
                        return Resolution::Duplicate;
                    }
                    if next.pinned_revision == first.revision {
                        return if next.event == first.event {
                            Resolution::Duplicate
                        } else {
                            Resolution::Conflict
                        };
                    }
                    true
                }
                Some(PinnedSlot::Open(opening, _)) => {
                    if opening.run != next.event.run
                        || opening.session != next.event.session
                        || opening.price_basis != next.price_basis
                        || !matches_route(
                            &PinnedSlot::Settled(PinnedRecord {
                                revision: next.pinned_revision,
                                maxima: next.maxima.clone(),
                                event: next.event.clone(),
                                price_basis: next.price_basis.clone(),
                                durable: Arc::new(AtomicBool::new(true)),
                            }),
                            &opening.spec,
                        )
                    {
                        return Resolution::Conflict;
                    }
                    false
                }
                _ => return Resolution::Conflict,
            };
            resolved.insert(
                id,
                PinnedSlot::Settled(PinnedRecord {
                    revision: next.pinned_revision,
                    maxima: next.maxima,
                    event: next.event,
                    price_basis: next.price_basis,
                    durable: Arc::new(AtomicBool::new(true)),
                }),
            );
            if replaced {
                Resolution::Duplicate
            } else {
                Resolution::Stored
            }
        }
        PinnedReplayLine::Unsent(next) => match resolved.get(&next.call_id) {
            None => {
                resolved.insert(
                    next.call_id,
                    PinnedSlot::Unsent(next.spec, Arc::new(AtomicBool::new(true))),
                );
                Resolution::Stored
            }
            Some(PinnedSlot::Open(opening, _)) if next.spec.as_ref() == Some(&opening.spec) => {
                resolved.insert(
                    next.call_id,
                    PinnedSlot::Unsent(next.spec, Arc::new(AtomicBool::new(true))),
                );
                Resolution::Stored
            }
            Some(PinnedSlot::Unsent(spec, _)) if spec == &next.spec => Resolution::Duplicate,
            _ => Resolution::Conflict,
        },
    }
}

/// A cold scan uses constant retained state (one identity). Called only on the adapter worker.
pub(super) fn lookup(path: &Path, id: &str) -> Result<Option<PinnedSlot>, PinnedError> {
    let file = fs::File::open(path).map_err(|_| PinnedError::ProjectionFull)?;
    let mut resolved = BTreeMap::new();
    let mut reader = io::BufReader::new(file);
    let limit =
        u64::try_from(MAXIMUM_PINNED_LINE_BYTES + 1).map_err(|_| PinnedError::CorruptLog)?;
    let mut bytes = Vec::with_capacity(MAXIMUM_PINNED_LINE_BYTES + 1);
    loop {
        bytes.clear();
        let read = io::Read::take(&mut reader, limit)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| PinnedError::ProjectionFull)?;
        if read == 0 {
            break;
        }
        if read > MAXIMUM_PINNED_LINE_BYTES {
            return Err(PinnedError::CorruptLog);
        }
        if let Ok(next) = serde_json::from_slice::<PinnedLogLine>(&bytes) {
            if next.event.call_id == id {
                resolve(&mut resolved, PinnedReplayLine::Settled(next));
            }
        } else if let Ok(next) = serde_json::from_slice::<PinnedOpeningLine>(&bytes) {
            if next.call_id == id {
                resolve(&mut resolved, PinnedReplayLine::Open(next));
            }
        } else if let Ok(next) = serde_json::from_slice::<PinnedUnsentLine>(&bytes)
            && next.call_id == id
            && next.pinned_unsent
        {
            resolve(&mut resolved, PinnedReplayLine::Unsent(next));
        }
    }
    Ok(resolved.remove(id))
}
