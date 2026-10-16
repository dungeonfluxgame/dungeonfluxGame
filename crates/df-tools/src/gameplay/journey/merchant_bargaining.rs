//! Source-bound NPC price quotes. Characterization may affect whether a merchant engages,
//! refuses, or counters, but only a validated exact quote and an explicit later acceptance can
//! change stock or currency.

use serde_json::{Value, json};

/// Use the inventory owner's canonical direction and terms to avoid duplicating price bounds.
pub(in crate::gameplay) type TradeDirection = super::living_scene::SceneTradeDirection;
pub(in crate::gameplay) type TradeTerms = super::living_scene::MerchantTradeTerms;

/// A server-validated quote candidate. The caller must add current source-line provenance and
/// persist it through the scene transaction authority before exposing it as a live quote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct QuoteCandidate {
    pub(in crate::gameplay) direction: TradeDirection,
    pub(in crate::gameplay) offer_id: Option<String>,
    pub(in crate::gameplay) item_id: String,
    pub(in crate::gameplay) seller_id: String,
    pub(in crate::gameplay) buyer_id: String,
    pub(in crate::gameplay) price_cp: u32,
}

/// Listener-safe, journal-derived context for a merchant buying back the same item from the
/// hero who purchased it. This carries no dialogue or private character state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct MerchantBuybackMemory {
    pub(in crate::gameplay) item_name: String,
    pub(in crate::gameplay) last_purchase_price_cp: u32,
    pub(in crate::gameplay) revision: String,
}

/// Player intent about a unique live quote. Callers must first resolve a buyer-bound quote whose
/// item, merchant, source line, expiry, and state revision are still current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum QuoteDecision {
    Accept,
    Decline,
    Counteroffer,
    Unclear,
}

/// Closed rejection vocabulary safe for diagnostics. It contains no words or prompt payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum QuoteRejection {
    InvalidTerms,
    Unavailable,
    NotBargaining,
    MerchantRefused,
    NoMerchantCommitment,
    ItemUnclear,
    MissingAmount,
    AmbiguousAmount,
    InvalidAmount,
    OutOfRange,
}

impl QuoteRejection {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::InvalidTerms => "invalid_terms",
            Self::Unavailable => "unavailable",
            Self::NotBargaining => "not_bargaining",
            Self::MerchantRefused => "merchant_refused",
            Self::NoMerchantCommitment => "no_merchant_commitment",
            Self::ItemUnclear => "item_unclear",
            Self::MissingAmount => "missing_amount",
            Self::AmbiguousAmount => "ambiguous_amount",
            Self::InvalidAmount => "invalid_amount",
            Self::OutOfRange => "out_of_range",
        }
    }
}

/// Validate a present merchant offer or counteroffer against current authoritative terms.
///
/// The buyer/seller IDs and item identity come from trusted caller context. The merchant's exact
/// spoken line must independently state a commitment, identify the item or its clear reference,
/// and contain exactly one integer in-game currency amount. For a hero sale this validates the
/// merchant's bid, even though the hero is the seller. This function creates no
/// durable quote, transfers no item, and charges neither participant.
pub(in crate::gameplay) fn quote_candidate(
    terms: &TradeTerms,
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
    player_words: &str,
    merchant_reply: &str,
) -> Result<QuoteCandidate, QuoteRejection> {
    if !valid_terms(terms)
        || authenticated_hero_id != hero_id_for_terms(terms)
        || addressed_merchant_id != merchant_id_for_terms(terms)
    {
        return Err(QuoteRejection::InvalidTerms);
    }
    if terms.stock_units == 0 {
        return Err(QuoteRejection::Unavailable);
    }
    if !is_bargaining_request(player_words) {
        return Err(QuoteRejection::NotBargaining);
    }
    if is_refusal(merchant_reply)
        || (is_trade_objection(merchant_reply) && !is_present_commitment(merchant_reply))
    {
        return Err(QuoteRejection::MerchantRefused);
    }
    if !is_present_commitment(merchant_reply) {
        return Err(QuoteRejection::NoMerchantCommitment);
    }
    if merchant_reply.contains('?') {
        return Err(QuoteRejection::NoMerchantCommitment);
    }
    let price_cp = explicit_price_cp(merchant_reply)?;
    // The NPC's exact public line must itself bind its price to the selected item or its
    // unambiguous conversational reference. A player's earlier item mention cannot make a
    // separate price line into a quote for that item.
    if !mentions_item(terms, merchant_reply)
        && !refers_to_active_item(merchant_reply)
        && !is_short_price_commitment(merchant_reply)
    {
        return Err(QuoteRejection::ItemUnclear);
    }

    if price_cp < terms.minimum_price_cp || price_cp > terms.maximum_price_cp {
        return Err(QuoteRejection::OutOfRange);
    }

    Ok(QuoteCandidate {
        direction: terms.direction,
        offer_id: terms.offer_id.clone(),
        item_id: terms.item_id.clone(),
        seller_id: terms.seller_id.clone(),
        buyer_id: terms.buyer_id.clone(),
        price_cp,
    })
}

/// Route explicit sale and price negotiation language to the NPC conversation path before the
/// generic physical-action classifier can treat a narrated presentation as an examination.
pub(in crate::gameplay) fn is_trade_dialogue_request(words: &str) -> bool {
    is_selling_request(words)
        || (is_bargaining_request(words)
            && (explicit_price_cp(words).is_ok()
                || [
                    "buy",
                    "how much",
                    "price",
                    "cost",
                    "worth",
                    "cheaper",
                    "discount",
                    "haggle",
                    "bargain",
                    "counteroffer",
                    "counter offer",
                ]
                .iter()
                .any(|phrase| contains_phrase(words, phrase))))
}

/// Resolve an explicit, unconditional purchase at an item's current catalog price. This is only
/// a routing aid; the authoritative stage still resolves the stable item ID, checks current stock
/// and purse, and requires the supplied payment to equal the listing price.
pub(in crate::gameplay) fn fixed_price_purchase(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    words: &str,
) -> Option<(String, u32)> {
    if !is_committed_purchase(words)
        || words.contains('?')
        || has_trade_condition(words)
        || is_hedged_acceptance(words)
    {
        return None;
    }
    let price_cp = explicit_price_cp(words).ok()?;
    let named_ids = terms
        .iter()
        .filter(|term| {
            valid_terms(term)
                && term.direction == TradeDirection::HeroBuys
                && term.buyer_id == authenticated_hero_id
                && term.available
                && term.stock_units > 0
                && term.maximum_price_cp == price_cp
                && mentions_item(term, words)
        })
        .map(|term| term.item_id.as_str())
        .collect::<Vec<_>>();
    let item_id = match unique_ids(named_ids).as_slice() {
        [item_id] => *item_id,
        [] if refers_to_active_item(words) => {
            let candidates = terms
                .iter()
                .filter(|term| {
                    valid_terms(term)
                        && term.direction == TradeDirection::HeroBuys
                        && term.buyer_id == authenticated_hero_id
                        && term.available
                        && term.stock_units > 0
                        && term.maximum_price_cp == price_cp
                })
                .map(|term| term.item_id.as_str())
                .collect::<Vec<_>>();
            match unique_ids(candidates).as_slice() {
                [item_id] => *item_id,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some((item_id.to_owned(), price_cp))
}

fn is_committed_purchase(words: &str) -> bool {
    [
        "i buy",
        "i'll buy",
        "i will buy",
        "i am buying",
        "i'm buying",
        "i purchase",
        "i'll purchase",
        "i will purchase",
        "i take",
        "i'll take",
        "i will take",
    ]
    .iter()
    .any(|phrase| contains_phrase(words, phrase))
}

/// Select a single item from the authenticated hero's trade terms. Named items always win; an
/// unqualified demonstrative is allowed only when it resolves to one in-scope term.
pub(in crate::gameplay) fn referenced_item_id(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
    words: &str,
) -> Option<String> {
    let relevant = terms
        .iter()
        .filter(|term| {
            !term.item_id.trim().is_empty()
                && authenticated_hero_id == hero_id_for_terms(term)
                && addressed_merchant_id == merchant_id_for_terms(term)
        })
        .collect::<Vec<_>>();
    let named_ids = relevant
        .iter()
        .copied()
        .filter(|term| mentions_item(term, words))
        .map(|term| term.item_id.as_str())
        .collect::<Vec<_>>();
    let selected_item_id = match unique_ids(named_ids).as_slice() {
        [item_id] => *item_id,
        [] if refers_to_active_item(words) => {
            let active_ids = unique_ids(
                relevant
                    .iter()
                    .copied()
                    .map(|term| term.item_id.as_str())
                    .collect(),
            );
            match active_ids.as_slice() {
                [item_id] => *item_id,
                _ => return None,
            }
        }
        _ => return None,
    };
    let item_terms = relevant
        .iter()
        .copied()
        .filter(|term| term.item_id == selected_item_id)
        .collect::<Vec<_>>();
    (resolve_direction(&item_terms, words) == Some(TradeDirection::HeroSells))
        .then(|| selected_item_id.to_owned())
}

/// Pick one canonical term and validate the NPC's exact line as a live offer. This prevents a
/// pronoun in one reply from being attached to an arbitrary item when several are in scope.
pub(in crate::gameplay) fn quote_candidate_for_reply(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
    player_words: &str,
    merchant_reply: &str,
) -> Result<QuoteCandidate, QuoteRejection> {
    let relevant = terms
        .iter()
        .filter(|term| {
            authenticated_hero_id == hero_id_for_terms(term)
                && addressed_merchant_id == merchant_id_for_terms(term)
        })
        .collect::<Vec<_>>();
    let named_ids = relevant
        .iter()
        .copied()
        .filter(|term| mentions_item(term, player_words))
        .map(|term| term.item_id.as_str())
        .collect::<Vec<_>>();
    let selected_item_id = match unique_ids(named_ids).as_slice() {
        [item_id] => *item_id,
        [] if refers_to_active_item(player_words) => {
            let identified_by_reply = relevant
                .iter()
                .copied()
                .filter(|term| mentions_item(term, merchant_reply))
                .map(|term| term.item_id.as_str())
                .collect::<Vec<_>>();
            match unique_ids(identified_by_reply).as_slice() {
                [item_id] => *item_id,
                [] => match unique_ids(relevant.iter().map(|term| term.item_id.as_str()).collect())
                    .as_slice()
                {
                    [item_id] => *item_id,
                    _ => return Err(QuoteRejection::ItemUnclear),
                },
                _ => return Err(QuoteRejection::ItemUnclear),
            }
        }
        _ => return Err(QuoteRejection::ItemUnclear),
    };
    let item_terms = relevant
        .iter()
        .copied()
        .filter(|term| term.item_id == selected_item_id)
        .collect::<Vec<_>>();
    let direction =
        resolve_direction(&item_terms, player_words).ok_or(QuoteRejection::ItemUnclear)?;
    let selected = item_terms
        .iter()
        .copied()
        .find(|term| term.direction == direction)
        .ok_or(QuoteRejection::ItemUnclear)?;
    quote_candidate_for_item(
        terms,
        authenticated_hero_id,
        addressed_merchant_id,
        selected.direction,
        &selected.item_id,
        player_words,
        merchant_reply,
    )
}

fn resolve_direction(terms: &[&TradeTerms], words: &str) -> Option<TradeDirection> {
    let sells = is_selling_request(words);
    let buys = is_buying_request(words);
    let canonical_buyback = [
        "buy it back",
        "buy this back",
        "buy these back",
        "buy those back",
        "back from me",
    ]
    .iter()
    .any(|phrase| contains_phrase(words, phrase));
    // Availability also depends on the merchant's purse. An explicit buyback request remains a
    // sale even when that purse is empty; the canonical sell term still binds the direction, and
    // the availability check rejects any quote that the merchant cannot fund.
    let hero_holds_item = terms
        .iter()
        .any(|term| term.direction == TradeDirection::HeroSells);
    match (sells, buys) {
        (true, true) if canonical_buyback && hero_holds_item => Some(TradeDirection::HeroSells),
        (true, false) => Some(TradeDirection::HeroSells),
        (false, true) => Some(TradeDirection::HeroBuys),
        (true, true) => None,
        (false, false) => {
            let directions = terms.iter().map(|term| term.direction).collect::<Vec<_>>();
            let mut unique = Vec::new();
            for direction in directions {
                if !unique.contains(&direction) {
                    unique.push(direction);
                }
            }
            match unique.as_slice() {
                [direction] => Some(*direction),
                _ => None,
            }
        }
    }
}

fn unique_ids(values: Vec<&str>) -> Vec<&str> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn is_buying_request(words: &str) -> bool {
    [
        "buy this",
        "buy it",
        "buy these",
        "buy those",
        "can i buy",
        "could i buy",
        "may i buy",
        "i want to buy",
        "i'd like to buy",
        "i would like to buy",
        "i'll buy",
        "i will buy",
        "i buy",
        "sell it to me",
        "sell this to me",
        "sell these to me",
        "sell those to me",
        "how much for",
        "what is the price of",
        "what's the price of",
    ]
    .iter()
    .any(|phrase| contains_phrase(words, phrase))
        || is_price_bid(words)
}

fn is_price_bid(words: &str) -> bool {
    if explicit_price_cp(words).is_err() {
        return false;
    }
    let tokens = normalized_words(words);
    let Some(unit_index) = tokens
        .iter()
        .rposition(|token| currency_multiplier(token).is_some())
    else {
        return false;
    };
    let offer_before_amount = tokens[..unit_index]
        .iter()
        .any(|token| matches!(token.as_str(), "offer" | "pay" | "give"));
    let item_clause_after_amount = tokens[unit_index + 1..].iter().any(|token| token == "for");
    offer_before_amount && item_clause_after_amount
}

/// Validate a merchant's counteroffer against the exact item and direction of the live quote it
/// answers. The prior receipt supplies context when a player uses an unqualified amount such as
/// “What about 11 gold?”.
pub(in crate::gameplay) fn quote_candidate_for_item(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
    direction: TradeDirection,
    item_id: &str,
    player_words: &str,
    merchant_reply: &str,
) -> Result<QuoteCandidate, QuoteRejection> {
    let selected = terms
        .iter()
        .find(|term| {
            term.direction == direction
                && term.item_id == item_id
                && authenticated_hero_id == hero_id_for_terms(term)
                && addressed_merchant_id == merchant_id_for_terms(term)
        })
        .ok_or(QuoteRejection::ItemUnclear)?;
    quote_candidate(
        selected,
        authenticated_hero_id,
        addressed_merchant_id,
        player_words,
        merchant_reply,
    )
}

/// Narrow a canonical buyback ceiling to below the last committed purchase price. The
/// inventory layer still validates its own item, purse, and canonical maximum; this only
/// prevents a buyback quote from offering the hero the same or more than the merchant received.
pub(in crate::gameplay) fn constrain_buyback_ceiling(
    terms: &mut [TradeTerms],
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
    item_id: &str,
    last_purchase_price_cp: u32,
) -> bool {
    if last_purchase_price_cp == 0 {
        return false;
    }
    let ceiling = last_purchase_price_cp - 1;
    let mut matched = false;
    for term in terms.iter_mut().filter(|term| {
        term.direction == TradeDirection::HeroSells
            && term.item_id == item_id
            && term.seller_id == authenticated_hero_id
            && term.buyer_id == addressed_merchant_id
    }) {
        term.maximum_price_cp = term.maximum_price_cp.min(ceiling);
        term.available &= term.maximum_price_cp >= term.minimum_price_cp;
        term.stock_units = u8::from(term.available);
        matched = true;
    }
    matched
}

/// Apply the last committed exact-item sale as a narrative fact and a stricter quote ceiling.
/// The history projection is derived from accepted scene transactions only and is filtered by
/// place, canonical item, merchant, and authenticated hero before it reaches the NPC.
pub(in crate::gameplay) fn apply_committed_buyback_context(
    current: &df_model::checkpoint::Checkpoint,
    terms: &mut [TradeTerms],
    place_id: &str,
    item_id: &str,
    authenticated_hero_id: &str,
    addressed_merchant_id: &str,
) -> Result<Option<MerchantBuybackMemory>, df_session::submission::RepositoryError> {
    let Some(transaction) = super::living_scene::latest_merchant_item_transaction(
        current,
        place_id,
        item_id,
        addressed_merchant_id,
        authenticated_hero_id,
    )?
    else {
        return Ok(None);
    };
    if transaction.direction
        != super::living_scene::MerchantItemTransactionDirection::MerchantSoldToHero
        || !constrain_buyback_ceiling(
            terms,
            authenticated_hero_id,
            addressed_merchant_id,
            item_id,
            transaction.price_cp,
        )
    {
        return Ok(None);
    }
    let Some(item_name) = terms
        .iter()
        .find(|term| {
            term.direction == TradeDirection::HeroSells
                && term.item_id == item_id
                && term.seller_id == authenticated_hero_id
                && term.buyer_id == addressed_merchant_id
        })
        .map(|term| term.item_name.clone())
    else {
        return Ok(None);
    };
    Ok(Some(MerchantBuybackMemory {
        item_name,
        last_purchase_price_cp: transaction.price_cp,
        revision: format!(
            "{}:{}",
            transaction.revision.epoch().get(),
            transaction.revision.sequence(),
        ),
    }))
}

/// Attach exact, listener-safe journal facts to an already-scoped merchant reply context.
pub(in crate::gameplay) fn attach_buyback_memory(
    context: &mut Value,
    memory: &MerchantBuybackMemory,
) {
    context["recent_committed_purchase"] = json!({
        "item": memory.item_name,
        "price_cp": memory.last_purchase_price_cp,
        "purchase_revision": memory.revision,
        "direction": "merchant_sold_to_hero",
    });
    if let Some(rules) = context
        .get_mut("negotiation_rules")
        .and_then(Value::as_array_mut)
    {
        rules.push(json!("The supplied recent committed purchase is for this same item, merchant, and hero. A request to buy it back is a trade, not a quest reward. Keep a resale margin: offer below that prior sale price, or decline briefly and professionally. The server enforces the exact quote and merchant purse."));
    }
}

/// Classify an explicit player response to one already-resolved live quote.
///
/// A generic affirmation is meaningful only when the caller has proved there is exactly one live
/// quote from the immediately preceding public NPC line for this authenticated hero. A stated
/// amount must equal the quote exactly; a different explicit price is a counteroffer, never an
/// acceptance. This helper does not check quote liveness or commit the purchase/sale.
pub(in crate::gameplay) fn quote_decision(words: &str, quoted_price_cp: u32) -> QuoteDecision {
    if quoted_price_cp == 0 {
        return QuoteDecision::Unclear;
    }
    let normalized = words.to_ascii_lowercase();
    match explicit_price_cp(words) {
        Ok(price_cp) if price_cp != quoted_price_cp => return QuoteDecision::Counteroffer,
        Err(QuoteRejection::AmbiguousAmount | QuoteRejection::InvalidAmount) => {
            return QuoteDecision::Unclear;
        }
        _ => {}
    }
    if is_quote_decline(words) {
        return QuoteDecision::Decline;
    }
    if normalized_words(words)
        .first()
        .is_some_and(|word| word == "no")
    {
        return QuoteDecision::Decline;
    }
    if has_trade_condition(&normalized)
        || normalized.contains('?')
        || contains_phrase(words, "but")
        || is_hedged_acceptance(words)
    {
        return QuoteDecision::Unclear;
    }

    let words_normalized = normalized_words(words);
    let simple_affirmation = matches!(
        words_normalized.join(" ").as_str(),
        "yes" | "yes please" | "okay" | "ok" | "sure" | "fine" | "all right" | "alright"
    );
    let affirmative = simple_affirmation
        || [
            "deal",
            "accept",
            "accepted",
            "agreed",
            "sounds fair",
            "sounds good",
            "that works",
            "that'll do",
            "that will do",
            "sold",
            "go ahead",
            "let's do it",
            "lets do it",
            "make the deal",
            "i'll take it",
            "i will take it",
            "i take it",
            "i'll buy it",
            "i will buy it",
            "i buy it",
            "i'll buy",
            "i will buy",
            "i buy",
            "i can do",
            "i can pay",
            "i will pay",
            "i'll pay",
            "i'll take it for",
            "i will take it for",
            "i accept",
            "i'll sell it",
            "i will sell it",
            "i sell it",
            "i'll sell",
            "i will sell",
            "i sell",
        ]
        .iter()
        .any(|phrase| contains_phrase(words, phrase));
    if !affirmative {
        return QuoteDecision::Unclear;
    }

    match explicit_price_cp(words) {
        Ok(price_cp) if price_cp == quoted_price_cp => QuoteDecision::Accept,
        Ok(_) => QuoteDecision::Counteroffer,
        Err(QuoteRejection::MissingAmount) => QuoteDecision::Accept,
        Err(_) => QuoteDecision::Unclear,
    }
}

/// Extend already listener-filtered NPC facts with public trade context and negotiation rules.
/// Do not pass a raw dossier, private ledger, or unfiltered character record here.
pub(in crate::gameplay) fn reply_context(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    merchant_id: &str,
    typed_words: &str,
    tendered_item_id: Option<&str>,
    listener_facts: &Value,
) -> Option<Value> {
    reply_context_with_quote(
        terms,
        authenticated_hero_id,
        merchant_id,
        typed_words,
        tendered_item_id,
        None,
        listener_facts,
    )
}

pub(in crate::gameplay) fn reply_context_for_quote(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    merchant_id: &str,
    typed_words: &str,
    quote: &super::living_scene::SceneTradeQuote,
    listener_facts: &Value,
) -> Option<Value> {
    reply_context_with_quote(
        terms,
        authenticated_hero_id,
        merchant_id,
        typed_words,
        Some(&quote.item_id),
        Some(quote),
        listener_facts,
    )
}

fn reply_context_with_quote(
    terms: &[TradeTerms],
    authenticated_hero_id: &str,
    merchant_id: &str,
    typed_words: &str,
    tendered_item_id: Option<&str>,
    live_quote: Option<&super::living_scene::SceneTradeQuote>,
    listener_facts: &Value,
) -> Option<Value> {
    let sell_context = live_quote.map_or_else(
        || tendered_item_id.is_some() || is_selling_request(typed_words),
        |quote| quote.direction == TradeDirection::HeroSells,
    );
    let hero_buy_terms = terms
        .iter()
        .filter(|term| {
            term.direction == TradeDirection::HeroBuys
                && hero_id_for_terms(term) == authenticated_hero_id
                && merchant_id_for_terms(term) == merchant_id
        })
        .collect::<Vec<_>>();
    let unique_active_buy_item = (hero_buy_terms.len() == 1 && refers_to_active_item(typed_words))
        .then(|| hero_buy_terms[0].item_id.as_str());
    let available: Vec<&TradeTerms> = terms
        .iter()
        .filter(|term| {
            hero_id_for_terms(term) == authenticated_hero_id
                && merchant_id_for_terms(term) == merchant_id
                && live_quote.is_none_or(|quote| {
                    term.direction == quote.direction && term.item_id == quote.item_id
                })
                && match term.direction {
                    TradeDirection::HeroBuys => {
                        !sell_context
                            && (term.available
                                || mentions_item(term, typed_words)
                                || unique_active_buy_item == Some(term.item_id.as_str()))
                    }
                    TradeDirection::HeroSells => {
                        sell_context
                            && live_quote.map_or_else(
                                || {
                                    tendered_item_id.map_or_else(
                                        || mentions_item(term, typed_words),
                                        |item_id| item_id == term.item_id.as_str(),
                                    )
                                },
                                |quote| quote.item_id == term.item_id,
                            )
                            && tendered_item_id
                                .map_or_else(|| true, |item_id| item_id == term.item_id.as_str())
                    }
                }
        })
        .collect();
    if available.is_empty() || !listener_facts.is_object() {
        return None;
    }

    let mut facts = json!({});
    for key in [
        "npc",
        "how_you_speak",
        "inner_state",
        "attitude_toward_hero",
        "emotional_profile",
        "emotional_state",
        "npc_reaction",
        "emotional_reaction",
        "current_mood",
        "current_reaction",
        "relationship",
        "role_archetype",
    ] {
        if let Some(value) = listener_facts.get(key) {
            facts[key] = value.clone();
        }
    }

    let inventory: Vec<Value> = available
        .iter()
        .map(|term| {
            let mut entry = json!({
                "item": term.item_name,
                "direction": match term.direction {
                    TradeDirection::HeroBuys => "merchant_sells",
                    TradeDirection::HeroSells => "merchant_buys",
                },
            });
            if term.direction == TradeDirection::HeroBuys {
                entry["listed_price_cp"] = json!(term.maximum_price_cp);
            } else {
                entry["minimum_bid_cp"] = json!(term.minimum_price_cp);
                entry["maximum_bid_cp"] = json!(term.maximum_price_cp);
            }
            entry["available"] = json!(term.available);
            entry["stock_units"] = json!(term.stock_units);
            entry
        })
        .collect();
    Some(json!({
        "merchant_facts": facts,
        "available_trades": inventory,
        "live_quote": live_quote.map(|quote| json!({
            "direction": match quote.direction {
                TradeDirection::HeroBuys => "merchant_sells",
                TradeDirection::HeroSells => "merchant_buys",
            },
            "item": available
                .iter()
                .find(|term| term.item_id == quote.item_id)
                .map(|term| term.item_name.as_str()),
            "price_cp": quote.price_cp,
        })),
        "negotiation_rules": [
            "Use the supplied individual role, authored background and experience, competence, motive, temperament, current emotional reaction, and relationship only to shape this character's voice and willingness to engage, refuse, or counter.",
            "Mood may make this individual more or less willing to bargain, but it never sets a numeric price, skill bonus, or guaranteed outcome. Follow current in-game stock and answer only for the named item.",
            "Do not infer ability, honesty, or price from ancestry or race, or generalize about an ancestry. Speak only from this character's explicitly authored and listener-safe personal experience or bias; it never changes the server's price bounds.",
            "A request or player offer does not itself earn a discount. If you choose to make a concrete offer, state one present, unconditional price for one item in one exact in-game currency amount.",
            "A spoken offer is only a proposal. It does not buy, sell, transfer, or charge anything until the player explicitly accepts that exact live quote.",
            "You may answer naturally, make one counteroffer, or decline. Keep any explanation brief and professional, and do not reveal private character facts unless the listener-safe facts say the character has shared them.",
        ],
    }))
}

fn valid_terms(terms: &TradeTerms) -> bool {
    !terms.seller_id.trim().is_empty()
        && !terms.buyer_id.trim().is_empty()
        && terms.seller_id != terms.buyer_id
        && !terms.item_id.trim().is_empty()
        && !terms.item_name.trim().is_empty()
        && terms.minimum_price_cp > 0
        && terms.minimum_price_cp <= terms.maximum_price_cp
        && terms.maximum_price_cp > 0
        && terms.stock_units <= 1
        && terms.available == (terms.stock_units > 0)
        && match terms.direction {
            TradeDirection::HeroBuys => terms
                .offer_id
                .as_ref()
                .is_some_and(|id| !id.trim().is_empty()),
            TradeDirection::HeroSells => true,
        }
}

fn merchant_id_for_terms(terms: &TradeTerms) -> &str {
    match terms.direction {
        TradeDirection::HeroBuys => &terms.seller_id,
        TradeDirection::HeroSells => &terms.buyer_id,
    }
}

fn hero_id_for_terms(terms: &TradeTerms) -> &str {
    match terms.direction {
        TradeDirection::HeroBuys => &terms.buyer_id,
        TradeDirection::HeroSells => &terms.seller_id,
    }
}

fn is_bargaining_request(words: &str) -> bool {
    const REQUESTS: &[&str] = &[
        "bargain",
        "bargaining",
        "haggle",
        "haggling",
        "cheaper",
        "lower price",
        "lower the price",
        "lower it",
        "discount",
        "best price",
        "best you can do",
        "best offer",
        "your best offer",
        "what is your offer",
        "what's your offer",
        "what is your best offer",
        "what's your best offer",
        "your price",
        "what is the price",
        "what's the price",
        "what does it cost",
        "what does this cost",
        "what do these cost",
        "what do those cost",
        "what do they cost",
        "what is the cost",
        "what's the cost",
        "cost",
        "how much does it cost",
        "how much do these cost",
        "how much do those cost",
        "how much do they cost",
        "how much",
        "how much for",
        "what is this worth",
        "what's this worth",
        "what is it worth",
        "what's it worth",
        "what would you pay",
        "what will you pay",
        "what do you offer",
        "what would you offer",
        "what could you offer",
        "what will you offer",
        "will you buy",
        "would you buy",
        "buy this",
        "deal",
        "counteroffer",
        "counter offer",
        "offer",
        "what about",
        "how about",
        "could you lower",
        "would you lower",
        "what if",
        "would you take",
        "would you accept",
        "would you do",
        "can you do",
        "could you do",
        "i can do",
        "i could do",
        "i can offer",
        "i could offer",
        "do you take",
        "can you take",
        "can i buy",
        "could i buy",
        "may i buy",
        "i want to buy",
        "i'd like to buy",
        "i would like to buy",
        "can i sell",
        "could i sell",
        "may i sell",
        "i want to sell",
        "i'd like to sell",
        "i would like to sell",
        "i can pay",
        "i will pay",
        "i'll pay",
        "i offer",
        "i am offering",
        "i buy",
        "i'll buy",
        "i will buy",
        "i sell",
        "i'll sell",
        "i will sell",
        "come down",
        "move on the price",
        "work with me",
    ];
    REQUESTS
        .iter()
        .any(|request| contains_phrase(words, request))
}

fn is_selling_request(words: &str) -> bool {
    [
        "can i sell",
        "could i sell",
        "may i sell",
        "i want to sell",
        "i'd like to sell",
        "i would like to sell",
        "i'll sell",
        "i will sell",
        "i sell",
        "i am selling",
        "i'm selling",
        "sell my",
        "sell these back",
        "sell those back",
        "sell this back",
        "sell it back",
        "buy this back",
        "buy these back",
        "buy those back",
        "buy it back",
        "back from me",
        "buy this from me",
        "buy it from me",
        "would you buy my",
        "will you buy my",
        "can you buy my",
        "could you buy my",
        "how much would you pay",
        "how much will you pay",
        "how much could you pay",
        "how much can you pay",
        "how much would you offer",
        "how much will you offer",
        "how much could you offer",
        "what can i get for my",
        "what could i get for my",
        "what will you give me for",
        "what can you give me for",
        "what could you give me for",
        "what can you offer for my",
        "what could you offer for my",
        "what will you offer for my",
        "how much for my",
        "make an offer for my",
        "take this off my hands",
        "take it off my hands",
        "what would you pay",
        "what will you pay",
        "what could you pay",
        "what would you give me",
        "what will you give me",
        "what could you give me",
        "what would you offer me",
        "what will you offer me",
        "what is this worth to you",
        "what's this worth to you",
    ]
    .iter()
    .any(|phrase| contains_phrase(words, phrase))
}

fn is_refusal(reply: &str) -> bool {
    const REFUSALS: &[&str] = &[
        "not for sale",
        "not interested",
        "no deal",
        "no bargaining",
        "i don't negotiate",
        "i do not negotiate",
        "i refuse",
        "can't agree",
        "cannot agree",
        "won't agree",
        "will not agree",
        "can't take less",
        "cannot take less",
        "can't go lower",
        "cannot go lower",
        "can't come lower",
        "cannot come lower",
    ];
    REFUSALS
        .iter()
        .any(|refusal| contains_phrase(reply, refusal))
}

fn is_trade_objection(reply: &str) -> bool {
    const OBJECTIONS: &[&str] = &[
        "can't do",
        "cannot do",
        "won't take",
        "will not take",
        "can't sell",
        "cannot sell",
        "won't sell",
        "will not sell",
        "not at that price",
        "can't agree",
        "cannot agree",
        "won't agree",
        "will not agree",
    ];
    OBJECTIONS
        .iter()
        .any(|objection| contains_phrase(reply, objection))
}

pub(in crate::gameplay) fn is_merchant_refusal(reply: &str) -> bool {
    is_refusal(reply) || is_trade_objection(reply)
}

fn is_hedged_acceptance(words: &str) -> bool {
    [
        "maybe",
        "perhaps",
        "might",
        "not sure",
        "let me think",
        "i think",
        "i guess",
        "i suppose",
        "could you",
        "would you",
    ]
    .iter()
    .any(|phrase| contains_phrase(words, phrase))
}

fn is_quote_decline(words: &str) -> bool {
    const DECLINES: &[&str] = &[
        "no deal",
        "not interested",
        "never mind",
        "forget it",
        "pass",
        "cancel",
        "i decline",
        "i refuse",
        "don't accept",
        "do not accept",
        "not accept",
        "don't take it",
        "do not take it",
    ];
    DECLINES.iter().any(|phrase| contains_phrase(words, phrase))
}

fn is_present_commitment(reply: &str) -> bool {
    const COMMITMENTS: &[&str] = &[
        "i ask",
        "i have them marked at",
        "i have it marked at",
        "i've got them marked at",
        "i've got it marked at",
        "marked at",
        "i can do",
        "we can do",
        "i'll do",
        "i will do",
        "we'll do",
        "we will do",
        "i'll take",
        "i will take",
        "i can take",
        "we can take",
        "i'll take it for",
        "i will take it for",
        "we'll take it for",
        "we will take it for",
        "i can offer",
        "i offer",
        "we offer",
        "i'll offer",
        "i will offer",
        "we'll offer",
        "we will offer",
        "i can sell it for",
        "i can sell that for",
        "i can sell",
        "i'll sell it for",
        "i will sell it for",
        "i'd sell it for",
        "i can let it go for",
        "i can meet you at",
        "i can come down to",
        "my price is",
        "my offer is",
        "my offer stands at",
        "the price is",
        "they cost",
        "it costs",
        "that costs",
        "the clothes cost",
        "the price comes to",
        "costs",
        "cost",
        "comes to",
        "is priced at",
        "are priced at",
        "it is yours for",
        "it's yours for",
        "i can buy it for",
        "i can buy",
        "we can buy it for",
        "we can buy",
        "i'll buy it for",
        "i'll buy",
        "i will buy it for",
        "i will buy",
        "we'll buy it for",
        "we'll buy",
        "we will buy it for",
        "we will buy",
        "i'd buy it for",
        "i can give",
        "i'll give",
        "i will give",
        "i can pay",
        "i'll pay",
        "i will pay",
        "i'd pay",
        "i accept",
        "i agree",
        "we accept",
        "we agree",
        "works for me",
        "that works",
        "sounds fair",
        "that's fair",
        "yes",
        "okay",
        "ok",
        "sure",
        "deal at",
        "deal",
        "you've got a deal",
        "you have a deal",
        "it's yours",
        "it is yours",
        "this is yours for",
        "you can take it for",
        "you can have it for",
        "you can have them for",
        "yours for",
        "take it or leave it",
        "all right,",
        "alright,",
    ];
    (COMMITMENTS
        .iter()
        .any(|phrase| contains_phrase(reply, phrase))
        || is_short_price_commitment(reply))
        && !has_trade_condition(reply)
        && !is_hedged_offer(reply)
}

fn has_trade_condition(text: &str) -> bool {
    let words = normalized_words(text);
    [
        "if", "unless", "provided", "when", "once", "after", "before",
    ]
    .iter()
    .any(|condition| words.iter().any(|word| word.as_str() == *condition))
        || contains_phrase(text, "as long as")
        || contains_phrase(text, "only if")
}

fn is_hedged_offer(text: &str) -> bool {
    [
        "maybe",
        "perhaps",
        "might",
        "probably",
        "not sure",
        "let me think",
        "i think",
        "i guess",
        "i suppose",
        "i could",
        "we could",
        "i would",
        "i'd",
        "i may",
        "would cost",
        "might cost",
        "may cost",
        "could cost",
        "would be",
        "could be",
        "might be",
        "may be",
        "used to cost",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
}

fn is_short_price_commitment(reply: &str) -> bool {
    if reply.contains('?')
        || has_trade_condition(reply)
        || is_hedged_offer(reply)
        || is_refusal(reply)
        || is_trade_objection(reply)
        || explicit_price_cp(reply).is_err()
    {
        return false;
    }
    let tokens = money_tokens(reply);
    let Some(first_currency) = tokens
        .iter()
        .position(|token| currency_multiplier(&token.text).is_some())
    else {
        return false;
    };
    if amount_start(&tokens, first_currency) != Some(0) {
        return false;
    }
    let Some(last_currency) = tokens
        .iter()
        .rposition(|token| currency_multiplier(&token.text).is_some())
    else {
        return false;
    };
    let mut suffix = last_currency + 1;
    if tokens
        .get(suffix)
        .is_some_and(|token| matches!(token.text.as_str(), "piece" | "pieces" | "coin" | "coins"))
    {
        suffix += 1;
    }
    tokens[suffix..]
        .iter()
        .all(|token| matches!(token.text.as_str(), "," | "then" | "now" | "exactly"))
}

fn mentions_item(terms: &TradeTerms, text: &str) -> bool {
    std::iter::once(terms.item_name.as_str())
        .chain(terms.aliases.iter().map(String::as_str))
        .filter(|name| !name.trim().is_empty())
        .any(|name| contains_phrase(text, name))
}

fn refers_to_active_item(text: &str) -> bool {
    [
        "the set",
        "set",
        "the outfit",
        "outfit",
        "the clothes",
        "clothes",
        "the clothing",
        "clothing",
        "this",
        "that",
        "it",
        "these",
        "those",
        "them",
        "this item",
        "that item",
        "this one",
        "that one",
    ]
    .iter()
    .any(|reference| contains_phrase(text, reference))
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    let words = normalized_words(text);
    let phrase = normalized_words(phrase);
    if phrase.is_empty() || phrase.len() > words.len() {
        return false;
    }
    words.windows(phrase.len()).any(|window| window == phrase)
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub(super) fn explicit_price_cp(reply: &str) -> Result<u32, QuoteRejection> {
    if has_price_qualifier(reply) || has_non_cash_terms(reply) {
        return Err(QuoteRejection::AmbiguousAmount);
    }
    let tokens = money_tokens(reply);
    let mut amounts = Vec::<(usize, usize, u32, u32)>::new();
    let mut saw_currency = false;
    for index in 0..tokens.len() {
        let Some(multiplier) = currency_multiplier(&tokens[index].text) else {
            continue;
        };
        saw_currency = true;
        let mut unit_end = index + 1;
        if tokens.get(unit_end).is_some_and(|token| {
            matches!(token.text.as_str(), "piece" | "pieces" | "coin" | "coins")
        }) {
            unit_end += 1;
        }
        let start = amount_start(&tokens, index).ok_or(QuoteRejection::InvalidAmount)?;
        if tokens[start..index].iter().any(|token| token.text == ",") {
            return Err(QuoteRejection::InvalidAmount);
        }
        if tokens[start].start > 0
            && reply
                .as_bytes()
                .get(tokens[start].start - 1)
                .is_some_and(|byte| matches!(byte, b'-' | b'+'))
        {
            return Err(QuoteRejection::InvalidAmount);
        }
        let amount = parse_number_expression(&tokens[start..index])?;
        let copper = amount
            .0
            .checked_mul(u128::from(multiplier))
            .ok_or(QuoteRejection::InvalidAmount)?;
        if copper % amount.1 != 0 {
            return Err(QuoteRejection::InvalidAmount);
        }
        let copper = u32::try_from(copper / amount.1).map_err(|_| QuoteRejection::InvalidAmount)?;
        amounts.push((start, unit_end, multiplier, copper));
    }
    if amounts.is_empty() {
        return if saw_currency {
            Err(QuoteRejection::MissingAmount)
        } else {
            Err(QuoteRejection::MissingAmount)
        };
    }

    let mut covered = vec![false; tokens.len()];
    for (start, end, _, _) in &amounts {
        covered[*start..*end].fill(true);
    }
    if tokens.iter().enumerate().any(|(index, token)| {
        !covered[index]
            && (token.text.bytes().any(|byte| byte.is_ascii_digit())
                || is_cardinal_word(&token.text))
    }) {
        return Err(QuoteRejection::AmbiguousAmount);
    }
    if amounts.len() > 1 {
        let mut denominations = Vec::new();
        for pair in amounts.windows(2) {
            let (left_start, left_end, denomination, _) = pair[0];
            let (right_start, _, right_denomination, _) = pair[1];
            if left_start >= right_start || denominations.contains(&denomination) {
                return Err(QuoteRejection::AmbiguousAmount);
            }
            denominations.push(denomination);
            let separator = &tokens[left_end..right_start];
            if !matches!(
                separator
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>()
                    .as_slice(),
                ["and"] | [","] | [",", "and"]
            ) {
                return Err(QuoteRejection::AmbiguousAmount);
            }
            if denomination == right_denomination {
                return Err(QuoteRejection::AmbiguousAmount);
            }
        }
    }
    amounts.iter().try_fold(0_u32, |total, (_, _, _, amount)| {
        total
            .checked_add(*amount)
            .ok_or(QuoteRejection::InvalidAmount)
    })
}

#[derive(Clone, Debug)]
struct MoneyToken {
    text: String,
    start: usize,
}

fn money_tokens(text: &str) -> Vec<MoneyToken> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let (start, character) = chars[index];
        if character.is_alphanumeric() {
            let mut end = index + 1;
            while end < chars.len() {
                let current = chars[end].1;
                let previous = chars[end - 1].1;
                let next = chars.get(end + 1).map(|(_, value)| *value);
                if current.is_alphanumeric()
                    || (matches!(current, ',' | '.')
                        && previous.is_ascii_digit()
                        && next.is_some_and(|value| value.is_ascii_digit()))
                {
                    end += 1;
                } else {
                    break;
                }
            }
            let end_byte = chars.get(end).map_or(text.len(), |(byte, _)| *byte);
            tokens.push(MoneyToken {
                text: text[start..end_byte].to_ascii_lowercase(),
                start,
            });
            index = end;
        } else if character == ',' {
            tokens.push(MoneyToken {
                text: ",".to_owned(),
                start,
            });
            index += 1;
        } else {
            index += 1;
        }
    }
    tokens
}

fn amount_start(tokens: &[MoneyToken], unit_index: usize) -> Option<usize> {
    let mut start = unit_index;
    while start > 0 && is_number_expression_word(&tokens[start - 1].text) {
        start -= 1;
    }
    while start < unit_index && tokens[start].text == "and" {
        start += 1;
    }
    (start < unit_index).then_some(start)
}

fn is_number_expression_word(word: &str) -> bool {
    is_cardinal_word(word) || matches!(word, "and" | "half")
}

fn is_cardinal_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "zero"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "ten"
            | "eleven"
            | "twelve"
            | "thirteen"
            | "fourteen"
            | "fifteen"
            | "sixteen"
            | "seventeen"
            | "eighteen"
            | "nineteen"
            | "twenty"
            | "thirty"
            | "forty"
            | "fifty"
            | "sixty"
            | "seventy"
            | "eighty"
            | "ninety"
            | "hundred"
            | "thousand"
    ) || word.bytes().any(|byte| byte.is_ascii_digit())
}

fn parse_number_expression(tokens: &[MoneyToken]) -> Result<(u128, u128), QuoteRejection> {
    let words = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>();
    if words.is_empty() {
        return Err(QuoteRejection::MissingAmount);
    }
    if words.len() == 1 && words[0].bytes().any(|byte| byte.is_ascii_digit()) {
        return parse_decimal(words[0]);
    }

    let mut words = words;
    if words.first() == Some(&"and") {
        words.remove(0);
    }
    let half_index = words.iter().position(|word| *word == "half");
    if let Some(index) = half_index {
        if index + 1 != words.len() && !(index + 2 == words.len() && words[index + 1] == "a") {
            return Err(QuoteRejection::InvalidAmount);
        }
        let prefix = &words[..index];
        let prefix = if prefix.last() == Some(&"a") || prefix.last() == Some(&"an") {
            &prefix[..prefix.len() - 1]
        } else {
            prefix
        };
        let prefix = if prefix.last() == Some(&"and") {
            &prefix[..prefix.len() - 1]
        } else {
            prefix
        };
        let whole = if prefix.is_empty() || prefix == ["a"] || prefix == ["an"] {
            0
        } else {
            parse_cardinal(prefix).ok_or(QuoteRejection::InvalidAmount)?
        };
        let numerator = u128::from(whole)
            .checked_mul(2)
            .and_then(|value| value.checked_add(1))
            .ok_or(QuoteRejection::InvalidAmount)?;
        return Ok((numerator, 2));
    }
    let integer = parse_cardinal(&words).ok_or(QuoteRejection::InvalidAmount)?;
    Ok((u128::from(integer), 1))
}

fn parse_decimal(number: &str) -> Result<(u128, u128), QuoteRejection> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let whole = parse_grouped_integer(whole).ok_or(QuoteRejection::InvalidAmount)?;
    if fraction.is_empty() {
        return Ok((u128::from(whole), 1));
    }
    if fraction.len() > 6 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(QuoteRejection::InvalidAmount);
    }
    let denominator = 10_u128
        .checked_pow(fraction.len() as u32)
        .ok_or(QuoteRejection::InvalidAmount)?;
    let fractional = fraction
        .parse::<u128>()
        .map_err(|_| QuoteRejection::InvalidAmount)?;
    let numerator = u128::from(whole)
        .checked_mul(denominator)
        .and_then(|value| value.checked_add(fractional))
        .ok_or(QuoteRejection::InvalidAmount)?;
    Ok((numerator, denominator))
}

fn parse_cardinal(words: &[&str]) -> Option<u32> {
    if words.is_empty() {
        return None;
    }
    let mut total = 0_u32;
    let mut current = 0_u32;
    for word in words {
        let value = match *word {
            "and" => continue,
            "a" | "an" => 1,
            "zero" => 0,
            "one" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            "seven" => 7,
            "eight" => 8,
            "nine" => 9,
            "ten" => 10,
            "eleven" => 11,
            "twelve" => 12,
            "thirteen" => 13,
            "fourteen" => 14,
            "fifteen" => 15,
            "sixteen" => 16,
            "seventeen" => 17,
            "eighteen" => 18,
            "nineteen" => 19,
            "twenty" => 20,
            "thirty" => 30,
            "forty" => 40,
            "fifty" => 50,
            "sixty" => 60,
            "seventy" => 70,
            "eighty" => 80,
            "ninety" => 90,
            "hundred" => {
                current = current.max(1).checked_mul(100)?;
                continue;
            }
            "thousand" => {
                total = total.checked_add(current.max(1).checked_mul(1_000)?)?;
                current = 0;
                continue;
            }
            _ => return None,
        };
        current = current.checked_add(value)?;
    }
    total.checked_add(current)
}

fn has_price_qualifier(text: &str) -> bool {
    [
        "about",
        "around",
        "approximately",
        "roughly",
        "under",
        "over",
        "between",
    ]
    .iter()
    .any(|word| {
        contains_phrase(text, word)
            && !(matches!(*word, "about")
                && (contains_phrase(text, "what about") || contains_phrase(text, "how about")))
    }) || [
        "less than",
        "more than",
        "at least",
        "at most",
        "no more than",
        "no less than",
        "up to",
        "as low as",
        "as high as",
        "starting at",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
}

fn has_non_cash_terms(text: &str) -> bool {
    [
        "plus",
        "tax",
        "fee",
        "fees",
        "deposit",
        "interest",
        "credit",
        "later",
        "tomorrow",
        "next week",
        "next month",
        "next day",
        "on credit",
        "barter",
        "trade",
        "swap",
        "favor",
        "bonus",
        "extra",
        "included",
        "include",
        "free",
        "throw in",
        "on top",
        "along with",
        "in exchange",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
}

fn parse_grouped_integer(number: &str) -> Option<u32> {
    if number.is_empty() || number.contains('.') {
        return None;
    }
    let parts: Vec<&str> = number.split(',').collect();
    let first = parts.first()?;
    if first.is_empty()
        || !first.bytes().all(|byte| byte.is_ascii_digit())
        || (parts.len() > 1 && first.len() > 3)
        || parts
            .iter()
            .skip(1)
            .any(|part| part.len() != 3 || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    number.replace(',', "").parse().ok()
}

fn currency_multiplier(unit: &str) -> Option<u32> {
    match unit.trim_matches('-').to_ascii_lowercase().as_str() {
        "cp" | "copper" | "coppers" => Some(1),
        "sp" | "silver" | "silvers" => Some(10),
        "ep" | "electrum" | "electrums" => Some(50),
        "gp" | "gold" | "golds" => Some(100),
        "pp" | "platinum" | "platinums" => Some(1_000),
        _ => None,
    }
}
