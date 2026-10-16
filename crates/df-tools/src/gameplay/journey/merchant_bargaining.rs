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
    // The NPC's exact public line must itself bind its price to the selected item or its
    // unambiguous conversational reference. A player's earlier item mention cannot make a
    // separate price line into a quote for that item.
    if !mentions_item(terms, merchant_reply) && !refers_to_active_item(merchant_reply) {
        return Err(QuoteRejection::ItemUnclear);
    }

    let price_cp = explicit_price_cp(merchant_reply)?;
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
                    "price",
                    "cost",
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
            valid_terms(term)
                && authenticated_hero_id == hero_id_for_terms(term)
                && addressed_merchant_id == merchant_id_for_terms(term)
                && (term.direction == TradeDirection::HeroSells) == is_selling_request(words)
        })
        .collect::<Vec<_>>();
    let named = relevant
        .iter()
        .copied()
        .filter(|term| mentions_item(term, words))
        .collect::<Vec<_>>();
    match named.as_slice() {
        [term] => Some(term.item_id.clone()),
        [] if refers_to_active_item(words) && relevant.len() == 1 => {
            Some(relevant[0].item_id.clone())
        }
        _ => None,
    }
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
    let direction = if is_selling_request(player_words) {
        TradeDirection::HeroSells
    } else {
        TradeDirection::HeroBuys
    };
    let relevant = terms
        .iter()
        .filter(|term| {
            term.direction == direction
                && authenticated_hero_id == hero_id_for_terms(term)
                && addressed_merchant_id == merchant_id_for_terms(term)
        })
        .collect::<Vec<_>>();
    let named = relevant
        .iter()
        .copied()
        .filter(|term| mentions_item(term, player_words))
        .collect::<Vec<_>>();
    let selected = match named.as_slice() {
        [term] => *term,
        [] if refers_to_active_item(player_words) => {
            let identified_by_reply = relevant
                .iter()
                .copied()
                .filter(|term| mentions_item(term, merchant_reply))
                .collect::<Vec<_>>();
            match identified_by_reply.as_slice() {
                [term] => *term,
                [] if relevant.len() == 1 => relevant[0],
                _ => return Err(QuoteRejection::ItemUnclear),
            }
        }
        _ => return Err(QuoteRejection::ItemUnclear),
    };
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
        "sell",
        "i can offer",
        "i could offer",
        "i offer",
        "i am offering",
        "i'm offering",
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
    ];
    OBJECTIONS
        .iter()
        .any(|objection| contains_phrase(reply, objection))
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
    COMMITMENTS
        .iter()
        .any(|phrase| contains_phrase(reply, phrase))
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
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
}

fn mentions_item(terms: &TradeTerms, text: &str) -> bool {
    std::iter::once(terms.item_name.as_str())
        .chain(terms.aliases.iter().map(String::as_str))
        .filter(|name| !name.trim().is_empty())
        .any(|name| contains_phrase(text, name))
}

fn refers_to_active_item(text: &str) -> bool {
    [
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
    let bytes = reply.as_bytes();
    let mut index = 0;
    let mut found = None;
    let mut unqualified_number = false;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_digit() || bytes[index] == b',' || bytes[index] == b'.')
        {
            index += 1;
        }
        let number = &reply[start..index];
        let signed = start > 0 && matches!(bytes[start - 1], b'-' | b'+');
        let mut unit_start = index;
        while unit_start < bytes.len() && bytes[unit_start].is_ascii_whitespace() {
            unit_start += 1;
        }
        let mut unit_end = unit_start;
        while unit_end < bytes.len()
            && (bytes[unit_end].is_ascii_alphabetic() || bytes[unit_end] == b'-')
        {
            unit_end += 1;
        }
        if let Some(multiplier) = currency_multiplier(&reply[unit_start..unit_end]) {
            if found.is_some() {
                return Err(QuoteRejection::AmbiguousAmount);
            }
            let whole = if signed {
                None
            } else {
                parse_grouped_integer(number)
            }
            .ok_or(QuoteRejection::InvalidAmount)?;
            let copper = whole
                .checked_mul(multiplier)
                .ok_or(QuoteRejection::InvalidAmount)?;
            found = Some(u32::try_from(copper).map_err(|_| QuoteRejection::InvalidAmount)?);
        } else {
            // A second bare numeral could be another price ("not 900; I'll do 1,200 cp") or a
            // quantity ("two for 1,200 cp"). Require one unambiguous amount for a single item.
            unqualified_number = true;
        }
    }
    match found {
        Some(_) if unqualified_number => Err(QuoteRejection::AmbiguousAmount),
        Some(price_cp) => Ok(price_cp),
        None => Err(QuoteRejection::MissingAmount),
    }
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
    .any(|word| contains_phrase(text, word))
        || [
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
