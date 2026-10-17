use std::time::Duration;

use serde_json::json;

use crate::gameplay::storyteller::Storyteller;
use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};

use super::merchant_bargaining::{
    HERO_TRADE_INSTRUCTIONS, HeroLine, HeroTradeIntent, HeroTradeMeaning, ListedQuoteAction,
    MerchantBuybackMemory, MerchantMeaning, MerchantReplyCommitment, OfferChoices, QuoteDecision,
    QuoteRejection, SERVICE_DECISION_INSTRUCTIONS, TradeDirection, TradeExchange, TradeFocus,
    TradeItemChoices, TradeJudgement, TradeTerms, attach_bulk_ask, attach_buyback_memory,
    bulk_ask_from_picks, bundle_discount_ask, constrain_buyback_ceiling, discount_ask,
    discount_willingness, explicit_price_cp, firm_buyback_quote_cp, fixed_price_purchase,
    hero_intent_from_judgement, hero_judgement_state, hero_stated_price_cp, judge_trade_item,
    judged_bulk_picks, listed_quote_action, merchant_commitment_from_judgement, parse_bulk_picks,
    quote_candidate, quote_candidate_for_item, quote_candidate_for_reply, quote_decision,
    referenced_item_id, reply_context, reply_context_for_quote, revalidate_quote,
    service_decision_from_judgement, service_facts, set_bundle_offer, set_discount_offer,
    sold_out_purchase_item, sold_out_refusal, trade_influence_applies, unlisted_service_quote_cp,
};
use crate::gameplay::approach_model::{ApproachModel, JevUnavailable};
use crate::gameplay::social::InfluenceWillingness;

const CLOTHES_ITEM: &str = "item-clothes-travelers";

fn terms(direction: TradeDirection) -> TradeTerms {
    TradeTerms {
        direction,
        seller_id: if direction == TradeDirection::HeroBuys {
            "marek".into()
        } else {
            "sigrun".into()
        },
        buyer_id: if direction == TradeDirection::HeroBuys {
            "sigrun".into()
        } else {
            "marek".into()
        },
        offer_id: (direction == TradeDirection::HeroBuys).then(|| "clothes-travelers".into()),
        item_id: CLOTHES_ITEM.into(),
        item_name: "fine clothes".into(),
        aliases: vec!["traveler clothes".into(), "travelers clothes".into()],
        minimum_price_cp: if direction == TradeDirection::HeroSells {
            1
        } else {
            1_000
        },
        maximum_price_cp: if direction == TradeDirection::HeroSells {
            1_200
        } else {
            1_500
        },
        available: true,
        stock_units: 1,
        stock_revision: 1,
        purse_revision: 1,
        counterparty_name: "Marek".into(),
        party_hero_names: vec!["Brakka".into(), "Sigrun".into()],
    }
}

/// A hero line with no usable Jev label. The line is neutral: it is not a trade.
fn unjudged_line(words: &str) -> HeroLine<'_> {
    HeroLine {
        intent: hero_intent_from_judgement(Err(JevUnavailable::ProviderUnavailable)),
        words,
    }
}

/// A negotiation turn with no usable Jev label on either side.
fn unjudged_turn<'a>(player_words: &'a str, merchant_reply: &'a str) -> TradeExchange<'a> {
    TradeExchange {
        hero: unjudged_line(player_words),
        merchant: merchant_commitment_from_judgement(Err(JevUnavailable::ProviderUnavailable)),
        merchant_reply,
    }
}

/// A hero line typed from Jev's label, as the service builds it when Jev answers.
fn jev_line<'a>(hero_label: &'static str, words: &'a str) -> HeroLine<'a> {
    HeroLine {
        intent: HeroTradeIntent::from_label(hero_label).expect("hero label is in the Jev set"),
        words,
    }
}

/// A negotiation turn typed from Jev's labels, as the service builds it when Jev answers.
fn jev_turn<'a>(
    hero_label: &'static str,
    player_words: &'a str,
    merchant_label: &'static str,
    merchant_reply: &'a str,
) -> TradeExchange<'a> {
    TradeExchange {
        hero: jev_line(hero_label, player_words),
        merchant: MerchantReplyCommitment::from_label(merchant_label)
            .expect("merchant label is in the Jev set"),
        merchant_reply,
    }
}

/// Reply context with no trade focus: Jev picked no item and the hero tendered none.
fn no_focus() -> TradeFocus<'static> {
    TradeFocus {
        tendered_item_id: None,
        picked_item_id: None,
    }
}

/// Reply context in which Jev picked this item and the hero tendered none.
fn focus_picked(item_id: &str) -> TradeFocus<'_> {
    TradeFocus {
        tendered_item_id: None,
        picked_item_id: Some(item_id),
    }
}

#[test]
fn seller_quotes_are_exact_bounded_and_directional() {
    let purchase = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &jev_turn(
            "ask_price",
            "Can you do better on the fine clothes?",
            "firm_price",
            "I can do the fine clothes for 1,200 copper pieces.",
        ),
    )
    .unwrap();
    assert_eq!(purchase.item_id, CLOTHES_ITEM);
    assert_eq!(purchase.offer_id.as_deref(), Some("clothes-travelers"));
    assert_eq!(purchase.seller_id, "marek");
    assert_eq!(purchase.buyer_id, "sigrun");
    assert_eq!(purchase.price_cp, 1_200);

    let buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &jev_turn(
            "sell_to_merchant",
            "Would you take this fine clothes item back?",
            "firm_price",
            "I can buy the fine clothes for 12 gold.",
        ),
    )
    .unwrap();
    assert_eq!(buyback.item_id, CLOTHES_ITEM);
    assert_eq!(buyback.seller_id, "sigrun");
    assert_eq!(buyback.buyer_id, "marek");
    assert_eq!(buyback.price_cp, 1_200);

    let requested_buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &jev_turn(
            "ask_price",
            "What's your offer for the fine clothes?",
            "firm_price",
            "I'll pay 12 gold for the fine clothes.",
        ),
    )
    .unwrap();
    assert_eq!(requested_buyback, buyback);

    let contextual = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &jev_turn(
            "ask_price",
            "Could you do better for this?",
            "firm_price",
            "I can do it for 1,200 copper.",
        ),
    )
    .unwrap();
    assert_eq!(contextual.item_id, CLOTHES_ITEM);
    assert_eq!(contextual.price_cp, 1_200);

    let natural_buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &jev_turn(
            "ask_price",
            "What would you pay for the fine clothes?",
            "firm_price",
            "I offer 12 gold for the fine clothes.",
        ),
    )
    .unwrap();
    assert_eq!(natural_buyback.price_cp, 1_200);

    let natural_buy = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &jev_turn(
            "ask_price",
            "Can you do better on the fine clothes?",
            "firm_price",
            "I'll take 1,200 copper for the fine clothes.",
        ),
    )
    .unwrap();
    assert_eq!(natural_buy.price_cp, 1_200);
}

#[test]
fn combined_show_and_sell_uses_the_canonical_held_item() {
    let request = "I show Marek the travelers clothes I just bought, still unworn. Marek, I want to sell these back to you for 1 GP.";
    let sale_terms = terms(TradeDirection::HeroSells);
    assert_eq!(
        referenced_item_id(
            std::slice::from_ref(&sale_terms),
            "sigrun",
            "marek",
            jev_line("sell_to_merchant", request),
            Some(CLOTHES_ITEM),
        )
        .as_deref(),
        Some(CLOTHES_ITEM)
    );
    let candidate = quote_candidate_for_reply(
        &[sale_terms],
        "sigrun",
        "marek",
        &jev_turn(
            "sell_to_merchant",
            request,
            "firm_price",
            "I can pay 1 GP for the travelers clothes.",
        ),
        Some(CLOTHES_ITEM),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.item_id, CLOTHES_ITEM);
    assert_eq!(candidate.price_cp, 100);
}

#[test]
fn exact_buyback_uses_owned_item_history_and_stays_below_the_prior_purchase() {
    let request = "Marek, will you buy these travelers clothes back from me? What would you pay?";
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.available = false;
    buy_terms.stock_units = 0;
    let mut sale_terms = terms(TradeDirection::HeroSells);
    sale_terms.maximum_price_cp = 300;
    let mut trade_terms = [buy_terms, sale_terms];

    assert_eq!(
        referenced_item_id(
            &trade_terms,
            "sigrun",
            "marek",
            jev_line("sell_to_merchant", request),
            Some(CLOTHES_ITEM),
        )
        .as_deref(),
        Some(CLOTHES_ITEM)
    );
    assert!(constrain_buyback_ceiling(
        &mut trade_terms,
        "sigrun",
        "marek",
        CLOTHES_ITEM,
        200,
    ));
    let sale = trade_terms
        .iter()
        .find(|term| term.direction == TradeDirection::HeroSells)
        .unwrap();
    assert_eq!(sale.maximum_price_cp, 199);
    assert_eq!(sale.stock_units, 1);
    assert_eq!(sale.available, true);

    let candidate = quote_candidate_for_reply(
        &trade_terms,
        "sigrun",
        "marek",
        &jev_turn(
            "sell_to_merchant",
            request,
            "firm_price",
            "I can pay 199 copper for the travelers clothes.",
        ),
        Some(CLOTHES_ITEM),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.price_cp, 199);
    assert_eq!(
        quote_candidate_for_reply(
            &trade_terms,
            "sigrun",
            "marek",
            &jev_turn(
                "sell_to_merchant",
                request,
                "firm_price",
                "I can pay 2 gold for the travelers clothes.",
            ),
            Some(CLOTHES_ITEM),
        ),
        Err(QuoteRejection::OutOfRange)
    );

    let mut context = reply_context(
        &trade_terms,
        "sigrun",
        "marek",
        jev_line("sell_to_merchant", request),
        TradeFocus {
            tendered_item_id: Some(CLOTHES_ITEM),
            picked_item_id: Some(CLOTHES_ITEM),
        },
        &json!({"npc": {"name": "Marek"}}),
    )
    .unwrap();
    attach_buyback_memory(
        &mut context,
        &MerchantBuybackMemory {
            item_name: "fine clothes".into(),
            last_purchase_price_cp: 200,
            revision: "1:6".into(),
        },
    );
    assert_eq!(context["recent_committed_purchase"]["item"], "fine clothes");
    assert_eq!(context["recent_committed_purchase"]["price_cp"], 200);
    assert_eq!(
        context["recent_committed_purchase"]["purchase_revision"],
        "1:6"
    );
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| {
                rule.as_str()
                    .is_some_and(|rule| rule.contains("trade, not a quest reward"))
            })
    );
    assert!(context["merchant_facts"].get("private_matter").is_none());
}

#[test]
fn buyback_direction_survives_an_unfunded_merchant_term() {
    let request = "Marek, will you buy these travelers clothes back from me? What would you pay?";
    let mut sale = terms(TradeDirection::HeroSells);
    sale.available = false;
    sale.stock_units = 0;
    assert_eq!(
        referenced_item_id(
            &[sale],
            "sigrun",
            "marek",
            jev_line("sell_to_merchant", request),
            Some(CLOTHES_ITEM),
        )
        .as_deref(),
        Some(CLOTHES_ITEM)
    );
}

#[test]
fn buyer_bid_is_not_misread_as_the_hero_selling_the_merchant_item() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    let trade_terms = [buy_terms, terms(TradeDirection::HeroSells)];
    let request = "I offer 1 GP for those travelers clothes.";
    assert_eq!(
        referenced_item_id(
            &trade_terms,
            "sigrun",
            "marek",
            jev_line("offer_price", request),
            Some(CLOTHES_ITEM),
        ),
        None
    );
    let candidate = quote_candidate_for_reply(
        &trade_terms,
        "sigrun",
        "marek",
        &jev_turn(
            "offer_price",
            request,
            "firm_price",
            "I can sell the travelers clothes for 2 GP.",
        ),
        Some(CLOTHES_ITEM),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.price_cp, 200);
}

#[test]
fn counteroffer_keeps_the_live_quote_item_and_trade_direction() {
    let candidate = quote_candidate_for_item(
        &[terms(TradeDirection::HeroSells)],
        "sigrun",
        "marek",
        TradeDirection::HeroSells,
        CLOTHES_ITEM,
        &jev_turn(
            "counter_offer",
            "What about 11 GP?",
            "firm_price",
            "I can meet you at 12 GP for the travelers clothes.",
        ),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.item_id, CLOTHES_ITEM);
    assert_eq!(candidate.price_cp, 1_200);
}

#[test]
fn quote_needs_a_judged_merchant_commitment_and_one_price() {
    let request = "Could you lower the price on the fine clothes?";
    for (merchant_label, reply, expected) in [
        (
            "hedges_or_asks",
            "Maybe, what did you have in mind?",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "hedges_or_asks",
            "Maybe I can do the fine clothes for 1,200 copper.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "hedges_or_asks",
            "I think I can do the fine clothes for 1,200 copper.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "refuses",
            "No, I can't do 900 copper.",
            QuoteRejection::MerchantRefused,
        ),
        (
            "conditional_or_future",
            "I can do the fine clothes for 1,200 copper if you buy both.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "firm_price",
            "I can do 1,200 copper or 12 gold.",
            QuoteRejection::AmbiguousAmount,
        ),
        (
            "refuses",
            "I can't agree to one gold.",
            QuoteRejection::MerchantRefused,
        ),
        (
            "refuses",
            "I can't take less for them.",
            QuoteRejection::MerchantRefused,
        ),
        (
            "hedges_or_asks",
            "I could do one gold.",
            QuoteRejection::NoMerchantCommitment,
        ),
    ] {
        assert_eq!(
            quote_candidate(
                &terms(TradeDirection::HeroBuys),
                "sigrun",
                "marek",
                &jev_turn("ask_price", request, merchant_label, reply),
            ),
            Err(expected),
            "{reply}"
        );
    }
    let mut two_gp_terms = terms(TradeDirection::HeroBuys);
    two_gp_terms.minimum_price_cp = 100;
    two_gp_terms.maximum_price_cp = 200;
    assert_eq!(
        quote_candidate(
            &two_gp_terms,
            "sigrun",
            "marek",
            &jev_turn(
                "not_trade",
                "Tell me about the fine clothes.",
                "firm_price",
                "I can do the fine clothes for 1,200 copper.",
            ),
        ),
        Err(QuoteRejection::NotBargaining)
    );
    assert_eq!(
        quote_candidate(
            &two_gp_terms,
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                "How much for the fine clothes?",
                "firm_price",
                "The set costs two gold pieces. It’s practical traveler’s clothing in sturdy wool and linen.",
            ),
        )
        .unwrap()
        .price_cp,
        200
    );
    assert_eq!(
        quote_candidate_for_reply(
            std::slice::from_ref(&two_gp_terms),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                "How much for the travelers clothes?",
                "firm_price",
                "The set costs two gold pieces. It’s practical traveler’s clothing in sturdy wool and linen.",
            ),
            Some(CLOTHES_ITEM),
        )
        .unwrap()
        .price_cp,
        200
    );
    assert_eq!(
        quote_candidate(
            &two_gp_terms,
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                "How much for the travelers clothes?",
                "firm_price",
                "Two gold pieces, then.",
            ),
        )
        .unwrap()
        .price_cp,
        200
    );
    for reply in [
        "I ask two gold pieces for the travelers clothes.",
        "I have them marked at two gold pieces.",
    ] {
        assert_eq!(
            quote_candidate(
                &two_gp_terms,
                "sigrun",
                "marek",
                &jev_turn(
                    "ask_price",
                    "How much for the travelers clothes?",
                    "firm_price",
                    reply
                ),
            )
            .unwrap()
            .price_cp,
            200,
            "{reply}"
        );
    }
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "I can do 2 fine clothes for 1,200 copper pieces.",
            ),
        ),
        Err(QuoteRejection::AmbiguousAmount)
    );
    // The item is the caller's terms, so the words add no name check. A bare price judged firm
    // is a quote for the live item.
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "My price is 1,200 copper."
            ),
        )
        .unwrap()
        .price_cp,
        1_200
    );
    // "plus a favor" is a condition Jev judges as conditional_or_future, so no quote is made.
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "conditional_or_future",
                "I can do the fine clothes for 1,200 copper plus a favor.",
            ),
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    // A question back to the hero is judged hedges_or_asks, so it makes no quote.
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "hedges_or_asks",
                "I can do the fine clothes for 1,200 copper?",
            ),
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "No, I can do the fine clothes for 1,200 copper.",
            ),
        )
        .unwrap()
        .price_cp,
        1_200
    );
}

#[test]
fn amount_parser_uses_exact_integer_in_game_currency() {
    for (speech, expected) in [
        ("1,200 cp", 1_200),
        ("12 gp", 1_200),
        ("120 sp", 1_200),
        ("24 ep", 1_200),
        ("1 pp", 1_000),
        ("two gold pieces", 200),
        ("one gold and five silver", 150),
        ("a gold piece", 100),
        ("half a gold", 50),
        ("1.2 gp", 120),
        ("0.01 gp", 1),
    ] {
        assert_eq!(explicit_price_cp(speech), Ok(expected), "{speech}");
    }
    assert_eq!(
        explicit_price_cp("0.001 gp"),
        Err(QuoteRejection::InvalidAmount)
    );
    assert_eq!(
        explicit_price_cp("-5 cp"),
        Err(QuoteRejection::InvalidAmount)
    );
    assert_eq!(explicit_price_cp("1 gp and 5 sp"), Ok(150));
    assert_eq!(
        explicit_price_cp("some coins"),
        Err(QuoteRejection::MissingAmount)
    );
    // A qualifier such as "about" is not a parser rule. The parser reads the one exact amount.
    assert_eq!(explicit_price_cp("about 1,200 cp"), Ok(1_200));
    assert_eq!(
        explicit_price_cp("1,200 cp plus a favor"),
        Err(QuoteRejection::AmbiguousAmount)
    );
    // "next week" is a meaning, judged by Jev as conditional_or_future. The parser reads 12 gp.
    assert_eq!(explicit_price_cp("12 gp next week"), Ok(1_200));
    assert_eq!(
        merchant_commitment_from_judgement(Ok("conditional_or_future")).meaning,
        MerchantMeaning::ConditionalOrFuture
    );
    assert_eq!(
        explicit_price_cp("I'll buy it from you for 12 gp"),
        Ok(1_200)
    );
}

#[test]
fn only_jev_negotiation_labels_open_a_bargain() {
    for label in ["ask_price", "offer_price", "buy_now", "sell_to_merchant"] {
        let intent = HeroTradeIntent::from_label(label).unwrap();
        assert!(intent.bargains, "{label}");
    }
    for label in ["not_trade", "unclear"] {
        let intent = HeroTradeIntent::from_label(label).unwrap();
        assert!(!intent.bargains, "{label}");
    }
    assert!(
        !unjudged_line("How much for the travelers clothes?")
            .intent
            .bargains,
        "a question with no Jev label opens no negotiation"
    );
}

#[test]
fn direct_purchase_requires_a_judged_commitment_at_the_current_listing_price() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_line(
                "buy_now",
                "I buy the travelers clothes from Marek for 2 GP."
            ),
            Some(CLOTHES_ITEM),
        ),
        Some((CLOTHES_ITEM.into(), 200))
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_line("offer_price", "I offer 2 GP for those travelers clothes."),
            Some(CLOTHES_ITEM),
        ),
        None
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_line("unclear", "Could I buy the travelers clothes for 2 GP?"),
            Some(CLOTHES_ITEM),
        ),
        None
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_line("buy_now", "I buy the travelers clothes for 13 GP."),
            Some(CLOTHES_ITEM),
        ),
        None
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            unjudged_line("I buy the travelers clothes from Marek for 2 GP."),
            Some(CLOTHES_ITEM),
        ),
        None,
        "without Jev's buy_now label no purchase is committed"
    );
    let mut unavailable = buy_terms.clone();
    unavailable.available = false;
    unavailable.stock_units = 0;
    assert_eq!(
        fixed_price_purchase(
            &[unavailable],
            "sigrun",
            jev_line("buy_now", "I buy the travelers clothes for 2 GP."),
            Some(CLOTHES_ITEM),
        ),
        None
    );
}

#[test]
fn fixed_price_purchase_follows_jev_buy_now_alone() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    let words = "The travelers clothes are mine at 2 GP, I suppose.";
    let jev_buy_now = jev_line("buy_now", words);
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_buy_now,
            Some(CLOTHES_ITEM),
        ),
        Some((CLOTHES_ITEM.into(), 200))
    );
    let jev_offer = jev_line("offer_price", "I buy the travelers clothes for 2 GP.");
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            jev_offer,
            Some(CLOTHES_ITEM),
        ),
        None,
        "a bid is never a commitment, whatever its wording"
    );
}

#[test]
fn without_jev_no_words_settle_the_live_quote() {
    // Arrange: Jev gave no label, so each hero line is neutral against a live quote of 1,200 CP.
    for words in [
        "Deal.",
        "I buy the fine clothes for 12 gold.",
        "I buy the fine clothes for 1,300 cp instead.",
        "No, thank you.",
        "Maybe, I accept.",
    ] {
        let intent = unjudged_line(words).intent;
        // Act
        let decision = quote_decision(intent, words, 1_200);
        // Assert
        assert_eq!(decision, QuoteDecision::Unclear, "{words}");
    }
    assert_eq!(
        unjudged_line("Deal.").intent.judgement.reason_label(),
        "provider_unavailable"
    );
}

#[test]
fn typed_quote_decision_follows_jev_acceptance_and_the_stated_amount() {
    let accept = HeroTradeIntent::from_label("accept_quote").unwrap();
    let decline = HeroTradeIntent::from_label("decline_quote").unwrap();
    let counter = HeroTradeIntent::from_label("counter_offer").unwrap();
    let unclear = HeroTradeIntent::from_label("unclear").unwrap();
    assert_eq!(
        quote_decision(accept, "Deal.", 1_200),
        QuoteDecision::Accept
    );
    assert_eq!(
        quote_decision(accept, "I'll take 12 gold for it.", 1_200),
        QuoteDecision::Accept
    );
    assert_eq!(
        quote_decision(accept, "I'll take 1,300 cp for it.", 1_200),
        QuoteDecision::Counteroffer,
        "a stated different amount is a counteroffer, never an acceptance"
    );
    assert_eq!(
        quote_decision(decline, "No, thank you.", 1_200),
        QuoteDecision::Decline
    );
    assert_eq!(
        quote_decision(counter, "No, how about 1,300 cp?", 1_200),
        QuoteDecision::Counteroffer
    );
    assert_eq!(
        quote_decision(counter, "Could we talk about it?", 1_200),
        QuoteDecision::Unclear,
        "a counter without an amount is not yet a price"
    );
    assert_eq!(
        quote_decision(unclear, "Deal.", 1_200),
        QuoteDecision::Unclear,
        "Jev's unclear label keeps the quote open even when the words sound like agreement"
    );
}

#[test]
fn jev_labels_map_to_one_typed_meaning_each_and_back() {
    for (label, _) in HeroTradeMeaning::CHOICES {
        let meaning = HeroTradeIntent::from_label(label).unwrap().meaning;
        assert_eq!(meaning.label(), label);
    }
    for (label, _) in MerchantMeaning::CHOICES {
        let commitment = MerchantReplyCommitment::from_label(label).unwrap();
        assert_eq!(commitment.meaning.label(), label);
    }
    assert!(HeroTradeIntent::from_label("maybe").is_none());
    assert!(MerchantReplyCommitment::from_label("probably").is_none());
    let sell = HeroTradeIntent::from_label("sell_to_merchant").unwrap();
    assert_eq!(sell.direction, Some(TradeDirection::HeroSells));
    let ask = HeroTradeIntent::from_label("ask_price").unwrap();
    assert_eq!(
        ask.direction, None,
        "a price question takes the item's only direction"
    );
    assert_eq!(ask.judgement, TradeJudgement::Jev);
}

#[test]
fn accepted_offer_then_firm_price_reply_yields_the_exact_candidate() {
    let sale = travelers_clothes_sale();
    let turn = jev_turn(
        "offer_price",
        "I'll pay you 150 copper for the traveler's clothes.",
        "firm_price",
        "Fine, 150 copper it is.",
    );
    let candidate = quote_candidate_for_reply(
        std::slice::from_ref(&sale),
        "sigrun",
        "marek",
        &turn,
        Some(CLOTHES_ITEM),
    )
    .expect("a judged firm price in bounds is a quote");
    assert_eq!(candidate.item_id, CLOTHES_ITEM);
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.price_cp, 150);
}

#[test]
fn judged_merchant_meanings_decide_the_candidate() {
    let sale = travelers_clothes_sale();
    let request = "I'll pay you 150 copper for the traveler's clothes.";
    let reply = "I can do 150 copper for the traveler's clothes.";
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "hedges_or_asks", reply)
        ),
        Err(QuoteRejection::NoMerchantCommitment),
        "a hedged reply makes no quote even when its wording looks firm"
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "conditional_or_future", reply)
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "refuses", reply)
        ),
        Err(QuoteRejection::MerchantRefused)
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "not_trade", reply)
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("unclear", request, "firm_price", reply)
        ),
        Err(QuoteRejection::NotBargaining),
        "an unclear hero line opens no negotiation"
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("not_trade", request, "firm_price", reply)
        ),
        Err(QuoteRejection::NotBargaining)
    );
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "firm_price", reply)
        )
        .unwrap()
        .price_cp,
        150
    );
}

#[test]
fn judged_firm_price_out_of_range_and_sold_out_still_reject() {
    let sale = travelers_clothes_sale();
    let request = "I'll pay you 50 copper for the traveler's clothes.";
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn(
                "offer_price",
                request,
                "accepts_hero_price",
                "I can do 50 copper for the traveler's clothes."
            )
        ),
        Err(QuoteRejection::OutOfRange)
    );
    let mut sold = travelers_clothes_sale();
    sold.available = false;
    sold.stock_units = 0;
    assert_eq!(
        quote_candidate(
            &sold,
            "sigrun",
            "marek",
            &jev_turn(
                "buy_now",
                "I buy the traveler's clothes for 2 gold.",
                "firm_price",
                "I can do 200 copper for the traveler's clothes."
            )
        ),
        Err(QuoteRejection::Unavailable)
    );
}

#[test]
fn judgement_from_jev_or_no_label_is_recorded_on_the_typed_value() {
    let from_jev = hero_intent_from_judgement(Ok("ask_price"));
    assert_eq!(from_jev.judgement, TradeJudgement::Jev);
    assert_eq!(from_jev.meaning, HeroTradeMeaning::AskPrice);

    let unavailable = hero_intent_from_judgement(Err(JevUnavailable::ProviderUnavailable));
    assert_eq!(
        unavailable.meaning,
        HeroTradeMeaning::NotTrade,
        "no label makes the hero line neutral"
    );
    assert!(!unavailable.is_trade());
    assert_eq!(
        unavailable.judgement,
        TradeJudgement::Rules(JevUnavailable::ProviderUnavailable)
    );
    assert_eq!(unavailable.judgement.reason_label(), "provider_unavailable");

    let unknown_hero = hero_intent_from_judgement(Ok("maybe"));
    assert_eq!(unknown_hero.meaning, HeroTradeMeaning::NotTrade);
    assert_eq!(unknown_hero.judgement.reason_label(), "unknown_choice");

    let merchant = merchant_commitment_from_judgement(Err(JevUnavailable::Timeout));
    assert_eq!(merchant.meaning, MerchantMeaning::NotTrade);
    assert_eq!(merchant.judgement.source_label(), "rules");
    assert_eq!(merchant.judgement.reason_label(), "timeout");

    let unknown = merchant_commitment_from_judgement(Ok("definitely_a_price"));
    assert_eq!(unknown.judgement.reason_label(), "unknown_choice");
}

#[test]
fn mood_experience_and_relationship_are_narrative_context_not_price_authority() {
    let first = json!({
        "attitude_toward_hero": "friendly",
        "emotional_state": {"patience": "plenty", "fondness": "some"},
        "npc_reaction": "softens a little toward Sigrun",
        "role_archetype": {"experience": {"id": "veteran"}, "scope": "narrative only"},
    });
    let second = json!({
        "attitude_toward_hero": "hostile",
        "emotional_state": {"patience": "spent", "resentment": "some"},
        "npc_reaction": "grows wary of Sigrun",
        "role_archetype": {"experience": {"id": "new-to-the-work"}, "scope": "narrative only"},
    });
    let first_context = reply_context(
        &[terms(TradeDirection::HeroBuys)],
        "sigrun",
        "marek",
        jev_line("ask_price", "How much for the fine clothes?"),
        no_focus(),
        &first,
    )
    .unwrap();
    let second_context = reply_context(
        &[terms(TradeDirection::HeroBuys)],
        "sigrun",
        "marek",
        jev_line("ask_price", "How much for the fine clothes?"),
        no_focus(),
        &second,
    )
    .unwrap();
    assert_ne!(
        first_context["merchant_facts"],
        second_context["merchant_facts"]
    );
    assert_eq!(
        first_context["available_trades"],
        second_context["available_trades"]
    );
    assert_eq!(
        first_context["available_trades"][0]["listed_price_cp"],
        1_500
    );
    assert_eq!(first_context["available_trades"][0]["stock_units"], 1);
    assert!(
        first_context["available_trades"][0]
            .get("minimum_price_cp")
            .is_none()
    );
    assert!(
        first_context["available_trades"][0]
            .get("maximum_price_cp")
            .is_none()
    );
    let candidate = |facts: &serde_json::Value| {
        let _prompt_context = reply_context(
            &[terms(TradeDirection::HeroBuys)],
            "sigrun",
            "marek",
            jev_line("ask_price", "How much for the fine clothes?"),
            no_focus(),
            facts,
        )
        .unwrap();
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                "Can you offer a lower price on the fine clothes?",
                "firm_price",
                "I can do the fine clothes for 1,200 copper.",
            ),
        )
        .unwrap()
    };
    let first_quote = candidate(&first);
    let second_quote = candidate(&second);
    assert_eq!(first_quote, second_quote);
    assert_eq!(first_quote.price_cp, 1_200);
}

#[test]
fn live_counteroffer_context_preserves_the_exact_item_and_direction() {
    let quote = super::living_scene::SceneTradeQuote {
        quote_id: "quote-1".into(),
        direction: TradeDirection::HeroSells,
        place_id: "market".into(),
        item_id: CLOTHES_ITEM.into(),
        offer_id: None,
        seller_id: "sigrun".into(),
        buyer_id: "marek".into(),
        price_cp: 1_000,
        stock_revision: 2,
        purse_revision: 3,
        source_line_id: "source-1".into(),
        source_line_sequence: 4,
        source_revision: "revision-4".into(),
        source_actor_id: "marek".into(),
    };
    let context = reply_context_for_quote(
        &[
            terms(TradeDirection::HeroSells),
            terms(TradeDirection::HeroBuys),
        ],
        "sigrun",
        "marek",
        jev_line("counter_offer", "What about 11 GP?"),
        &quote,
        &json!({"attitude_toward_hero": "wary"}),
    )
    .unwrap();
    assert_eq!(context["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(context["available_trades"][0]["direction"], "merchant_buys");
    assert_eq!(context["live_quote"]["price_cp"], 1_000);
    assert_eq!(context["merchant_facts"]["attitude_toward_hero"], "wary");
}

#[test]
fn hero_judgement_state_names_the_party_and_the_merchant_and_the_rule_excludes_hand_overs() {
    let sale = terms(TradeDirection::HeroBuys);
    let words = "I hand the traveler's clothes to Sigrun.";
    let state = hero_judgement_state(words, None, &[sale]);
    assert_eq!(state["hero_line"], json!(words));
    assert_eq!(state["merchant_names"], json!(["Marek"]));
    assert_eq!(state["party_heroes"], json!(["Brakka", "Sigrun"]));
    assert_eq!(state["trade_items"][0]["item"], json!("fine clothes"));
    assert!(HERO_TRADE_INSTRUCTIONS.contains("party_heroes"));
    assert!(HERO_TRADE_INSTRUCTIONS.contains("merchant_names"));
    assert!(
        HERO_TRADE_INSTRUCTIONS.contains("is not_trade, even when the item is in trade_items"),
        "hand-overs to a companion must be classified as not_trade"
    );
    assert!(
        HERO_TRADE_INSTRUCTIONS.contains("only an exchange with the merchant for coin is trade")
    );
}

#[test]
fn prompt_context_keeps_only_listener_safe_facts_and_excludes_private_authority() {
    let facts = json!({
        "npc": {"name": "Marek", "role": "merchant"},
        "attitude_toward_hero": "indifferent",
        "role_archetype": {"experience": {"id": "veteran"}},
        "private_matter": "secret unrelated fact",
        "secret_bias": "hidden details",
    });
    let context = reply_context(
        &[terms(TradeDirection::HeroBuys)],
        "sigrun",
        "marek",
        jev_line("ask_price", "Could I buy the fine clothes?"),
        no_focus(),
        &facts,
    )
    .unwrap();
    assert!(context["merchant_facts"].get("role_archetype").is_some());
    assert!(context["merchant_facts"].get("private_matter").is_none());
    assert!(context["merchant_facts"].get("secret_bias").is_none());
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| {
                rule.as_str()
                    .is_some_and(|text| text.contains("ancestry or race"))
            })
    );
}

#[test]
fn reply_context_never_lists_unasked_hero_inventory_to_the_merchant() {
    let mut owned_item = terms(TradeDirection::HeroSells);
    owned_item.item_id = "item-silver-brooch".into();
    owned_item.item_name = "silver brooch".into();
    owned_item.aliases.clear();
    let facts = json!({"npc": {"name": "Marek", "role": "merchant"}});

    let buying = reply_context(
        &[terms(TradeDirection::HeroBuys), owned_item.clone()],
        "sigrun",
        "marek",
        jev_line("ask_price", "How much for the fine clothes?"),
        no_focus(),
        &facts,
    )
    .unwrap();
    assert_eq!(buying["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(buying["available_trades"][0]["direction"], "merchant_sells");
    assert_eq!(buying["available_trades"][0]["item"], "fine clothes");

    let selling = reply_context(
        &[terms(TradeDirection::HeroBuys), owned_item.clone()],
        "sigrun",
        "marek",
        jev_line(
            "sell_to_merchant",
            "What would you pay for my silver brooch?",
        ),
        focus_picked("item-silver-brooch"),
        &facts,
    )
    .unwrap();
    assert_eq!(selling["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(selling["available_trades"][0]["direction"], "merchant_buys");
    assert_eq!(selling["available_trades"][0]["item"], "silver brooch");

    let selling_by_how_much = reply_context(
        &[terms(TradeDirection::HeroBuys), owned_item],
        "sigrun",
        "marek",
        jev_line(
            "sell_to_merchant",
            "How much would you pay for my silver brooch?",
        ),
        focus_picked("item-silver-brooch"),
        &facts,
    )
    .unwrap();
    assert_eq!(
        selling_by_how_much["available_trades"][0]["direction"],
        "merchant_buys"
    );
    assert_eq!(
        selling_by_how_much["available_trades"][0]["item"],
        "silver brooch"
    );

    let other_hero_view = reply_context(
        &[
            terms(TradeDirection::HeroBuys),
            terms(TradeDirection::HeroSells),
        ],
        "another-hero",
        "marek",
        jev_line("sell_to_merchant", "What would you pay for my item?"),
        no_focus(),
        &facts,
    );
    assert!(other_hero_view.is_none());
}

#[test]
fn reply_context_shares_the_stockout_jev_picked_and_not_unpicked_unavailable_stock() {
    let mut sold_out = terms(TradeDirection::HeroBuys);
    sold_out.available = false;
    sold_out.stock_units = 0;
    let facts = json!({"npc": {"name": "Marek", "role": "merchant"}});

    let asked = reply_context(
        &[sold_out.clone()],
        "sigrun",
        "marek",
        jev_line("ask_price", "Do you still have the fine clothes?"),
        focus_picked(CLOTHES_ITEM),
        &facts,
    )
    .unwrap();
    assert_eq!(asked["available_trades"][0]["item"], "fine clothes");
    assert_eq!(asked["available_trades"][0]["available"], false);
    assert_eq!(asked["available_trades"][0]["stock_units"], 0);

    let unasked = reply_context(
        &[sold_out],
        "sigrun",
        "marek",
        jev_line("ask_price", "What are you selling?"),
        no_focus(),
        &facts,
    );
    assert!(unasked.is_none());
}

#[test]
fn stale_stock_wrong_buyer_and_out_of_range_prices_reject_before_quote_creation() {
    let request = "Could you do better on the fine clothes?";
    let mut changed_stock = terms(TradeDirection::HeroBuys);
    changed_stock.stock_units = 0;
    changed_stock.available = false;
    assert_eq!(
        quote_candidate(
            &changed_stock,
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "I can do the fine clothes for 1,200 copper."
            ),
        ),
        Err(QuoteRejection::Unavailable)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "someone-else",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "I can do the fine clothes for 1,200 copper."
            ),
        ),
        Err(QuoteRejection::InvalidTerms)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "I can do the fine clothes for 900 copper."
            ),
        ),
        Err(QuoteRejection::OutOfRange)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &jev_turn(
                "ask_price",
                request,
                "firm_price",
                "I can do the fine clothes for 1,600 copper."
            ),
        ),
        Err(QuoteRejection::OutOfRange)
    );
}

fn travelers_clothes_sale() -> TradeTerms {
    let mut sale = terms(TradeDirection::HeroBuys);
    sale.item_name = "traveler's clothes".into();
    sale.aliases = vec!["travelers clothes".into()];
    sale.minimum_price_cp = 100;
    sale.maximum_price_cp = 200;
    sale
}

#[test]
fn merchant_sells_context_gives_the_bounds_and_the_hero_named_amount() {
    let sale = travelers_clothes_sale();
    let facts = json!({"npc": {"name": "Marek", "role": "merchant"}});
    let offer = reply_context(
        std::slice::from_ref(&sale),
        "sigrun",
        "marek",
        jev_line(
            "offer_price",
            "I'll pay you 150 copper for the traveler's clothes.",
        ),
        focus_picked(CLOTHES_ITEM),
        &facts,
    )
    .unwrap();
    let entry = &offer["available_trades"][0];
    assert_eq!(entry["direction"], "merchant_sells");
    assert_eq!(entry["lowest_price_cp"], 100);
    assert_eq!(entry["listed_price_cp"], 200);
    assert_eq!(entry["hero_offer_cp"], 150);
    assert!(
        offer["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule
                .as_str()
                .is_some_and(|rule| rule.contains("hero_offer_cp")))
    );

    let question = reply_context(
        std::slice::from_ref(&sale),
        "sigrun",
        "marek",
        jev_line("ask_price", "How much for the traveler's clothes?"),
        focus_picked(CLOTHES_ITEM),
        &facts,
    )
    .unwrap();
    assert_eq!(question["available_trades"][0]["lowest_price_cp"], 100);
    assert!(
        question["available_trades"][0]
            .get("hero_offer_cp")
            .is_none()
    );
}

#[test]
fn plain_hero_offer_within_bounds_yields_an_exact_quote() {
    let sale = travelers_clothes_sale();
    let request = "I'll pay you 150 copper for the traveler's clothes.";
    let candidate = quote_candidate(
        &sale,
        "sigrun",
        "marek",
        &jev_turn(
            "offer_price",
            request,
            "firm_price",
            "I can do 150 copper for the traveler's clothes.",
        ),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.item_id, CLOTHES_ITEM);
    assert_eq!(candidate.price_cp, 150);
}

#[test]
fn sold_out_item_context_says_so_and_no_quote_is_created_for_it() {
    let mut sold = travelers_clothes_sale();
    sold.available = false;
    sold.stock_units = 0;
    let context = reply_context(
        std::slice::from_ref(&sold),
        "sigrun",
        "marek",
        jev_line("offer_price", "I buy the traveler's clothes for 2 gold."),
        focus_picked(CLOTHES_ITEM),
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .unwrap();
    let entry = &context["available_trades"][0];
    assert_eq!(entry["direction"], "merchant_sells");
    assert_eq!(entry["available"], false);
    assert_eq!(entry["stock_units"], 0);
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().is_some_and(|rule| rule.contains("sold out")))
    );

    let request = "I buy the traveler's clothes for 2 gold.";
    let reply = "I can do 200 copper for the traveler's clothes.";
    assert_eq!(
        quote_candidate(
            &sold,
            "sigrun",
            "marek",
            &jev_turn("offer_price", request, "firm_price", reply)
        ),
        Err(QuoteRejection::Unavailable)
    );
    assert_eq!(
        quote_candidate_for_item(
            &[sold],
            "sigrun",
            "marek",
            TradeDirection::HeroBuys,
            CLOTHES_ITEM,
            &jev_turn("offer_price", request, "firm_price", reply),
        ),
        Err(QuoteRejection::Unavailable)
    );
}

#[test]
fn buyback_hero_ask_is_passed_and_bounded_by_the_maximum_bid() {
    let mut buyback = terms(TradeDirection::HeroSells);
    buyback.item_name = "traveler's clothes".into();
    buyback.minimum_price_cp = 1;
    buyback.maximum_price_cp = 100;
    let request = "Marek, will you buy these traveler's clothes back from me for 150 copper? They are unworn.";
    let context = reply_context(
        std::slice::from_ref(&buyback),
        "sigrun",
        "marek",
        jev_line("sell_to_merchant", request),
        TradeFocus {
            tendered_item_id: Some(CLOTHES_ITEM),
            picked_item_id: Some(CLOTHES_ITEM),
        },
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .unwrap();
    let entry = &context["available_trades"][0];
    assert_eq!(entry["direction"], "merchant_buys");
    assert_eq!(entry["hero_offer_cp"], 150);
    assert_eq!(entry["maximum_bid_cp"], 100);

    let candidate = quote_candidate(
        &buyback,
        "sigrun",
        "marek",
        &jev_turn(
            "sell_to_merchant",
            request,
            "firm_price",
            "I can offer 90 copper for them.",
        ),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.price_cp, 90);
    assert_eq!(
        quote_candidate(
            &buyback,
            "sigrun",
            "marek",
            &jev_turn(
                "sell_to_merchant",
                request,
                "firm_price",
                "I can offer 150 copper for them.",
            ),
        ),
        Err(QuoteRejection::OutOfRange)
    );
}

#[test]
fn merchant_accepting_a_hero_offer_below_the_lowest_price_is_out_of_range() {
    let sale = travelers_clothes_sale();
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &jev_turn(
                "offer_price",
                "I'll pay you 50 copper for the traveler's clothes.",
                "accepts_hero_price",
                "I can do 50 copper for the traveler's clothes.",
            ),
        ),
        Err(QuoteRejection::OutOfRange)
    );
}

#[test]
fn merchant_asking_for_a_figure_or_name_produces_no_quote() {
    let sale = travelers_clothes_sale();
    let request = "I'll pay you 150 copper for the traveler's clothes.";
    for reply in [
        "I'm listening, but I need a figure before I can weigh it.",
        "Name the amount you're offering, and I'll consider it.",
    ] {
        assert_eq!(
            quote_candidate(
                &sale,
                "sigrun",
                "marek",
                &jev_turn("offer_price", request, "hedges_or_asks", reply),
            ),
            Err(QuoteRejection::NoMerchantCommitment),
            "{reply}"
        );
    }
}

#[test]
fn withdrawal_recheck_uses_the_judged_refusal_and_the_same_turn() {
    let sale = travelers_clothes_sale();
    let request = "I'll pay you 150 copper for the traveler's clothes.";
    let refusal = jev_turn(
        "offer_price",
        request,
        "refuses",
        "No, I won't do 150 copper.",
    );
    assert_eq!(
        quote_candidate(&sale, "sigrun", "marek", &refusal),
        Err(QuoteRejection::MerchantRefused)
    );
    let accepted = jev_turn(
        "offer_price",
        request,
        "firm_price",
        "I can do 150 copper for the traveler's clothes.",
    );
    let terms_slice = std::slice::from_ref(&sale);
    assert!(
        revalidate_quote(
            terms_slice,
            "sigrun",
            "marek",
            TradeDirection::HeroBuys,
            CLOTHES_ITEM,
            "I can do 150 copper for the traveler's clothes.",
        )
        .is_ok()
    );
    assert!(
        quote_candidate_for_item(
            terms_slice,
            "sigrun",
            "marek",
            TradeDirection::HeroBuys,
            CLOTHES_ITEM,
            &accepted,
        )
        .is_ok()
    );
    assert_eq!(
        revalidate_quote(
            terms_slice,
            "sigrun",
            "marek",
            TradeDirection::HeroBuys,
            CLOTHES_ITEM,
            "I can do 50 copper for the traveler's clothes.",
        ),
        Err(QuoteRejection::OutOfRange),
        "the deterministic bounds are checked again without repeating the judgement"
    );
}

/// The post-purchase terms after Marek bought the only traveler's clothes: the sale is sold out and
/// the buyback of the same instance is capped at half its 200 CP value.
fn sold_out_sale_and_buyback() -> [TradeTerms; 2] {
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let mut buyback = terms(TradeDirection::HeroSells);
    buyback.item_name = "traveler's clothes".into();
    buyback.aliases = vec!["travelers clothes".into()];
    buyback.minimum_price_cp = 1;
    buyback.maximum_price_cp = 100;
    [sold_out, buyback]
}

#[test]
fn live_buyback_line_after_purchase_gives_the_hero_figure_to_the_buyback_entry_only() {
    let line_words = "They're a poor fit after all. Marek, would you buy these traveler's clothes back from me for 1 gold?";
    let terms = sold_out_sale_and_buyback();
    let context = reply_context(
        &terms,
        "sigrun",
        "marek",
        jev_line("sell_to_merchant", line_words),
        TradeFocus {
            tendered_item_id: Some(CLOTHES_ITEM),
            picked_item_id: Some(CLOTHES_ITEM),
        },
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .expect("the buyback line has a trade context");

    let trades = context["available_trades"].as_array().unwrap();
    assert_eq!(
        trades.len(),
        1,
        "the sold-out sale is not offered in a buyback turn"
    );
    let buyback = &trades[0];
    assert_eq!(buyback["direction"], "merchant_buys");
    assert_eq!(buyback["maximum_bid_cp"], 100);
    assert_eq!(buyback["hero_offer_cp"], 100);
    let figures = trades
        .iter()
        .filter(|entry| entry.get("hero_offer_cp").is_some())
        .count();
    assert_eq!(figures, 1, "exactly one entry carries the hero's figure");
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().is_some_and(|rule| {
                rule.contains("you must answer that figure")
                    && rule.contains("Never ask the player what they propose")
            })),
        "the merchant must answer the stated figure rather than ask for one"
    );
}

#[test]
fn buy_line_gives_the_hero_figure_to_the_sale_entry_and_not_to_the_buyback() {
    let [sale, buyback] = sold_out_sale_and_buyback();
    let mut available_sale = sale;
    available_sale.available = true;
    available_sale.stock_units = 1;
    let context = reply_context(
        &[available_sale, buyback],
        "sigrun",
        "marek",
        jev_line("offer_price", "I'll buy the traveler's clothes for 1 gold."),
        focus_picked(CLOTHES_ITEM),
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .expect("the purchase line has a trade context");

    let trades = context["available_trades"].as_array().unwrap();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["direction"], "merchant_sells");
    assert_eq!(trades[0]["hero_offer_cp"], 100);
}

#[test]
fn hero_lines_state_their_figure_past_an_article_and_merchant_lines_stay_strict() {
    for wording in [
        "1 gold",
        "1 gold?",
        "a gold piece",
        "one gold",
        "100 copper",
        "1 gp",
    ] {
        let line = format!("They're a poor fit after all. Would you buy these back for {wording}?");
        assert_eq!(
            hero_stated_price_cp(&line),
            Ok(100),
            "hero wording: {wording}"
        );
    }
    assert_eq!(
        explicit_price_cp("They're a poor fit after all. I can do 1 gold for them."),
        Err(QuoteRejection::AmbiguousAmount),
        "a merchant commitment keeps the strict reading of an extra article"
    );
}

#[test]
fn sell_line_inside_the_buyback_terms_names_a_firm_figure_without_a_check() {
    let terms = sold_out_sale_and_buyback();
    let line = jev_line(
        "sell_to_merchant",
        "Marek, would you buy these traveler's clothes back from me for 1 gold?",
    );

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", line, Some(CLOTHES_ITEM)),
        Some((CLOTHES_ITEM.into(), 100)),
        "a named price inside the 1 to 100 CP buyback terms is a firm figure, not an Influence ask"
    );
}

#[test]
fn sell_line_naming_no_price_takes_the_buyback_ceiling_and_the_reply_must_name_it() {
    let terms = sold_out_sale_and_buyback();
    let line = jev_line(
        "sell_to_merchant",
        "You sold them to me a minute ago, Marek. They're untouched. What will you give me for them?",
    );

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", line, Some(CLOTHES_ITEM)),
        Some((CLOTHES_ITEM.into(), 100)),
        "a sell line that names no price is offered the terms price at once"
    );
    let context = reply_context(
        &terms,
        "sigrun",
        "marek",
        line,
        TradeFocus {
            tendered_item_id: Some(CLOTHES_ITEM),
            picked_item_id: Some(CLOTHES_ITEM),
        },
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .expect("the buyback line has a trade context");
    let buyback = context["available_trades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["direction"] == "merchant_buys")
        .expect("the buyback entry is in the context");
    assert_eq!(buyback["hero_offer_cp"], 100);
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().is_some_and(|rule| {
                rule.contains("never ask to inspect or judge the item's condition first")
                    && rule.contains("No inspection or condition check happens in this game")
            })),
        "the merchant must name the buyback price and never ask to inspect the item"
    );
}

#[test]
fn sell_line_naming_a_price_outside_the_buyback_terms_gets_no_firm_figure() {
    let terms = sold_out_sale_and_buyback();
    let outside = jev_line(
        "sell_to_merchant",
        "Marek, would you buy these traveler's clothes back from me for 5 gold?",
    );
    let ask_price = jev_line(
        "ask_price",
        "What are you asking for the traveler's clothes?",
    );

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", outside, Some(CLOTHES_ITEM)),
        None
    );
    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", ask_price, Some(CLOTHES_ITEM)),
        None
    );
}

#[test]
fn discount_ask_below_the_listing_offers_the_named_price_held_within_the_bounds() {
    let terms = [travelers_clothes_sale()];
    let line = jev_line(
        "offer_price",
        "Would you take 160 copper for the traveler's clothes?",
    );
    let ask = discount_ask(&terms, "sigrun", "marek", line, Some(CLOTHES_ITEM))
        .expect("a named price below the listing is a discount ask");

    assert_eq!(ask.item_id, CLOTHES_ITEM);
    assert_eq!(ask.named_cp, 160);
    assert_eq!(ask.listed_cp, 200);
    assert_eq!(ask.offer_cp(), 160);

    let below_bounds = jev_line(
        "offer_price",
        "Would you take 50 copper for the traveler's clothes?",
    );
    let floor = discount_ask(&terms, "sigrun", "marek", below_bounds, Some(CLOTHES_ITEM))
        .expect("a bid under the lowest price is still a discount ask");
    assert_eq!(
        floor.offer_cp(),
        100,
        "the merchant may not come down below the lowest price"
    );
}

#[test]
fn discount_ask_at_the_listing_or_on_a_sold_out_sale_is_not_a_discount_ask() {
    let terms = [travelers_clothes_sale()];
    let at_listing = jev_line(
        "offer_price",
        "Would you take 200 copper for the traveler's clothes?",
    );
    assert_eq!(
        discount_ask(&terms, "sigrun", "marek", at_listing, Some(CLOTHES_ITEM)),
        None
    );

    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let discount = jev_line(
        "offer_price",
        "Would you take 160 copper for the traveler's clothes?",
    );
    assert_eq!(
        discount_ask(&[sold_out], "sigrun", "marek", discount, Some(CLOTHES_ITEM)),
        None,
        "a sold-out item cannot be discounted"
    );
}

#[test]
fn sold_out_sale_item_names_no_price_and_no_hero_figure_in_the_reply_context() {
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let context = reply_context(
        &[sold_out],
        "sigrun",
        "marek",
        jev_line(
            "ask_price",
            "Marek, I'd like a set of those traveler's clothes too. Same price, two gold?",
        ),
        focus_picked(CLOTHES_ITEM),
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .expect("the sold-out item is named, so the merchant has a trade context");

    let entry = &context["available_trades"].as_array().unwrap()[0];
    assert_eq!(entry["direction"], "merchant_sells");
    assert_eq!(entry["sold_out"], true);
    assert_eq!(entry["available"], false);
    assert!(entry.get("listed_price_cp").is_none());
    assert!(entry.get("lowest_price_cp").is_none());
    assert!(entry.get("hero_offer_cp").is_none());
    assert!(
        context["negotiation_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().is_some_and(|rule| {
                rule.contains("sold_out") && rule.contains("accept no payment")
            })),
        "a sold-out item must be refused as sold out, never sold for coin"
    );
}

#[test]
fn purchase_of_a_sold_out_item_is_named_as_sold_out_and_in_stock_items_are_not() {
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let line = jev_line("buy_now", "Good, I'll take them. Here's two gold.");

    assert_eq!(
        sold_out_purchase_item(
            std::slice::from_ref(&sold_out),
            "sigrun",
            "marek",
            line,
            Some(CLOTHES_ITEM),
        ),
        Some("traveler's clothes".into()),
        "a purchase of the only sold-out item is refused as sold out"
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&sold_out),
            "sigrun",
            line,
            Some(CLOTHES_ITEM),
        ),
        None,
        "no purchase is committed for a sold-out item"
    );
    assert_eq!(
        sold_out_purchase_item(
            &[travelers_clothes_sale()],
            "sigrun",
            "marek",
            line,
            Some(CLOTHES_ITEM),
        ),
        None,
        "an item in stock is not reported as sold out"
    );
}

#[test]
fn a_bare_amount_counteroffer_is_recognised_so_a_refusal_cannot_withdraw_the_listed_quote() {
    let counter = quote_decision(
        HeroTradeIntent::from_label("ask_price").unwrap(),
        "Would you take 160?",
        200,
    );
    assert_eq!(
        counter,
        QuoteDecision::Counteroffer,
        "a bare amount against a live quote is a counteroffer, not an unclear line"
    );
    assert_eq!(
        listed_quote_action(counter, true),
        ListedQuoteAction::Keep,
        "the merchant refusing a counteroffer must not withdraw the listed quote"
    );
}

#[test]
fn a_refused_counteroffer_keeps_the_listed_quote_open_for_an_acceptance_at_its_price() {
    assert_eq!(
        listed_quote_action(QuoteDecision::Counteroffer, true),
        ListedQuoteAction::Keep
    );
    let accept = quote_decision(
        HeroTradeIntent::from_label("accept_quote").unwrap(),
        "Fine, I'll take it at two gold.",
        200,
    );
    assert_eq!(accept, QuoteDecision::Accept);
    assert_eq!(listed_quote_action(accept, false), ListedQuoteAction::Keep);
    assert_eq!(
        listed_quote_action(QuoteDecision::Decline, false),
        ListedQuoteAction::HeroDeclines,
        "only an explicit decline closes the listed quote as the hero's"
    );
}

#[test]
fn a_firm_buyback_figure_skips_the_influence_check_even_for_an_influence_attempt() {
    assert!(
        trade_influence_applies(true, false, false),
        "an ordinary influence attempt still rolls"
    );
    assert!(
        !trade_influence_applies(true, true, false),
        "a firm buyback figure is never an Influence check"
    );
    assert!(
        trade_influence_applies(false, false, true),
        "a discount ask rolls even when the approach is not an influence attempt"
    );
    assert!(!trade_influence_applies(false, false, false));
}

#[test]
fn a_discount_ask_is_hesitant_unless_the_merchant_is_already_unwilling() {
    assert_eq!(
        discount_willingness(Some(InfluenceWillingness::Willing)),
        InfluenceWillingness::Hesitant
    );
    assert_eq!(discount_willingness(None), InfluenceWillingness::Hesitant);
    assert_eq!(
        discount_willingness(Some(InfluenceWillingness::Unwilling)),
        InfluenceWillingness::Unwilling
    );
}

#[test]
fn a_successful_discount_check_offers_the_bounded_figure_and_a_failure_keeps_the_listed_price() {
    let terms = [travelers_clothes_sale()];
    let line = jev_line(
        "offer_price",
        "Would you take 160 copper for the traveler's clothes?",
    );
    let ask =
        discount_ask(&terms, "sigrun", "marek", line, Some(CLOTHES_ITEM)).expect("a discount ask");
    let context = || {
        json!({"available_trades": [{
            "item": "traveler's clothes",
            "direction": "merchant_sells",
            "listed_price_cp": 200,
            "hero_offer_cp": 160,
        }]})
    };

    let mut success = context();
    set_discount_offer(&mut success, &ask, true);
    assert_eq!(success["available_trades"][0]["hero_offer_cp"], 160);
    assert_eq!(success["available_trades"][0]["listed_price_cp"], 200);

    let mut failure = context();
    set_discount_offer(&mut failure, &ask, false);
    assert!(
        failure["available_trades"][0]
            .get("hero_offer_cp")
            .is_none(),
        "a failed check leaves no figure for the merchant to state"
    );
    assert_eq!(failure["available_trades"][0]["listed_price_cp"], 200);
}

#[test]
fn a_purchase_of_a_sold_out_item_gets_a_plain_sold_out_refusal_naming_no_price() {
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let line = jev_line("buy_now", "Good, I'll take them. Here's two gold.");
    let refusal = sold_out_refusal(&[sold_out], "sigrun", "marek", line, Some(CLOTHES_ITEM))
        .expect("a purchase of the sold-out item is refused");

    assert!(refusal.contains("sold out"), "{refusal}");
    assert!(
        !refusal.chars().any(|c| c.is_ascii_digit()),
        "the refusal names no price: {refusal}"
    );
}

#[test]
fn merchant_reply_facts_quote_each_service_at_its_exact_price() {
    let quotes = vec![crate::gameplay::journey::living_scene::ServiceQuote {
        name: "mending".into(),
        description: "Mending a small tear.".into(),
        price_cp: 50,
        done: false,
    }];
    let facts = crate::gameplay::journey::merchant_bargaining::service_facts(&quotes);
    assert_eq!(facts["services"][0]["service"], "mending");
    assert_eq!(facts["services"][0]["price_cp"], 50);
    assert_eq!(facts["services"][0]["done"], false);
    assert!(
        facts["services_rules"]
            .to_string()
            .contains("exact price_cp")
    );
}

/// The authored shop's nine offers as the merchant's sale terms for one hero, built as the live
/// terms are: the bounds run from half the listed price, rounded up, to the listed price.
fn authored_sale_terms(hero: &str) -> Vec<TradeTerms> {
    let mut model: crate::gameplay::adventure::scene_model::SceneModel = serde_json::from_slice(
        include_bytes!("../../../../../content/merchant-shop/scene-model.json"),
    )
    .unwrap();
    model.migrate_inventory_version().unwrap();
    model
        .offers
        .iter()
        .map(|offer| TradeTerms {
            direction: TradeDirection::HeroBuys,
            item_id: offer.canonical_item_id(),
            item_name: offer.name.clone(),
            aliases: offer.aliases.clone(),
            offer_id: Some(offer.id.clone()),
            seller_id: "merchant".into(),
            buyer_id: hero.into(),
            minimum_price_cp: offer.cost_cp / 2 + offer.cost_cp % 2,
            maximum_price_cp: offer.cost_cp,
            available: true,
            stock_units: 1,
            stock_revision: 1,
            purse_revision: 1,
            counterparty_name: "Marek".into(),
            party_hero_names: vec![hero.into()],
        })
        .collect()
}

#[test]
fn a_stock_question_lists_every_offer_and_service_at_its_exact_price() {
    let terms = authored_sale_terms("sigrun");
    assert_eq!(terms.len(), 9);
    let facts = json!({
        "npc": {"name": "Marek", "role": "merchant"},
        "services": [{"service": "mending", "price_cp": 50, "done": false}],
        "services_rules": ["When the hero asks what a listed service costs, quote it by name."],
    });
    // A question about the stock is judged ask_price; the listing itself needs no pick.
    let context = reply_context(
        &terms,
        "sigrun",
        "merchant",
        jev_line("ask_price", "Marek, what have you got for sale today?"),
        no_focus(),
        &facts,
    )
    .unwrap();
    let trades = context["available_trades"].as_array().unwrap();
    assert_eq!(trades.len(), 9, "{trades:?}");
    let wool_cloak = trades
        .iter()
        .find(|entry| entry["item"] == "warm wool cloak")
        .unwrap();
    assert_eq!(wool_cloak["listed_price_cp"], 300);
    assert_eq!(wool_cloak["lowest_price_cp"], 150);
    let gloves = trades
        .iter()
        .find(|entry| entry["item"] == "pair of gloves")
        .unwrap();
    assert_eq!(gloves["listed_price_cp"], 100);
    // The services reach the trade context and are not dropped by its fact whitelist.
    assert_eq!(context["merchant_facts"]["services"][0]["price_cp"], 50);
    assert!(context["merchant_facts"]["services_rules"].is_array());
    assert!(
        context["negotiation_rules"]
            .to_string()
            .contains("asks what you have for sale")
    );
}

#[test]
fn a_sold_out_offer_in_the_stock_question_names_no_price() {
    let mut terms = authored_sale_terms("sigrun");
    let gloves = terms
        .iter_mut()
        .find(|term| term.item_name == "pair of gloves")
        .unwrap();
    gloves.available = false;
    gloves.stock_units = 0;
    let facts = json!({"npc": {"name": "Marek", "role": "merchant"}});
    let context = reply_context(
        &terms,
        "sigrun",
        "merchant",
        jev_line("ask_price", "What are you selling today?"),
        no_focus(),
        &facts,
    )
    .unwrap();
    // An unpicked sold-out offer is not listed, so the stock question names no price for it.
    let trades = context["available_trades"].as_array().unwrap();
    assert_eq!(trades.len(), 8, "{trades:?}");
    assert!(trades.iter().all(|entry| entry["item"] != "pair of gloves"));
    // Picked by Jev, the same sold-out offer is shared as sold out, with no price.
    let asked = reply_context(
        &terms,
        "sigrun",
        "merchant",
        jev_line("ask_price", "Do you still have the gloves?"),
        focus_picked("item-gloves"),
        &facts,
    )
    .unwrap();
    let entry = asked["available_trades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"] == "pair of gloves")
        .unwrap();
    assert_eq!(entry["sold_out"], true);
    assert!(entry.get("listed_price_cp").is_none());
    assert!(entry.get("lowest_price_cp").is_none());
}

#[test]
fn a_described_cloak_price_question_is_picked_by_jev_and_quoted_as_that_offer() {
    let terms = authored_sale_terms("sigrun");
    let choices = OfferChoices::new(&terms, "sigrun", "merchant").unwrap();
    // Every in-stock offer is a numbered choice, and the last position names none of them.
    assert_eq!(choices.criteria().len(), 10);
    assert!(choices.criteria()[0].contains("traveler"));
    assert_eq!(choices.item_for(9), None);
    let wool = choices
        .criteria()
        .iter()
        .position(|c| c.starts_with("warm wool cloak"))
        .unwrap();
    assert_eq!(choices.item_for(wool), Some("item-warm-cloak-wool"));

    let turn = jev_turn(
        "ask_price",
        "How much for one of those warm cloaks?",
        "firm_price",
        "300 copper for the warm wool cloak, and that is my price.",
    );
    let quote = quote_candidate_for_item(
        &terms,
        "sigrun",
        "merchant",
        TradeDirection::HeroBuys,
        choices.item_for(wool).unwrap(),
        &turn,
    )
    .unwrap();
    assert_eq!(quote.item_id, "item-warm-cloak-wool");
    assert_eq!(quote.price_cp, 300);
    assert_eq!(quote.direction, TradeDirection::HeroBuys);
}

#[test]
fn a_generic_cloak_word_cannot_pick_the_fur_lined_cloak_without_jevs_pick() {
    // "cloak" is an alias of the warm wool cloak, so the words alone name two offers. With no
    // item picked by Jev the reading refuses; Jev's pick among the listed offers settles it.
    let terms = authored_sale_terms("sigrun");
    let turn = jev_turn(
        "ask_price",
        "How much for the fur-lined cloak?",
        "firm_price",
        "1000 copper for the fur-lined cloak.",
    );
    assert_eq!(
        quote_candidate_for_reply(&terms, "sigrun", "merchant", &turn, None),
        Err(QuoteRejection::ItemUnclear)
    );
    let choices = OfferChoices::new(&terms, "sigrun", "merchant").unwrap();
    let fur = choices
        .criteria()
        .iter()
        .position(|c| c.starts_with("fur-lined cloak"))
        .unwrap();
    let quote = quote_candidate_for_item(
        &terms,
        "sigrun",
        "merchant",
        TradeDirection::HeroBuys,
        choices.item_for(fur).unwrap(),
        &turn,
    )
    .unwrap();
    assert_eq!(quote.item_id, "item-warm-cloak-lined");
    assert_eq!(quote.price_cp, 1000);
}

#[test]
fn a_purchase_for_another_hero_quotes_the_named_offer_at_its_listed_price() {
    let terms = authored_sale_terms("brakka");
    let turn = jev_turn(
        "buy_now",
        "I'll buy the gloves for Sigrun.",
        "firm_price",
        "100 copper for the gloves, and they are yours.",
    );
    let quote = quote_candidate_for_reply(&terms, "brakka", "merchant", &turn, Some("item-gloves"))
        .unwrap();
    assert_eq!(quote.item_id, "item-gloves");
    assert_eq!(quote.price_cp, 100);
    assert_eq!(quote.buyer_id, "brakka");
}

#[test]
fn without_jev_an_accept_phrase_does_not_accept_a_live_quote_of_1200() {
    // Arrange: Jev gave no label, so "I'll take it." is a neutral line against a live quote of 1,200.
    let line = unjudged_line("I'll take it.");
    // Act
    let decision = quote_decision(line.intent, line.words, 1_200);
    // Assert
    assert_eq!(decision, QuoteDecision::Unclear);
    assert_eq!(
        line.intent.judgement,
        TradeJudgement::Rules(JevUnavailable::ProviderUnavailable)
    );
    assert_eq!(line.intent.judgement.reason_label(), "provider_unavailable");
}

#[test]
fn without_jev_a_hero_line_is_not_a_trade_and_selects_no_item() {
    // Arrange: a purchase at the listing that Jev never judged as buy_now.
    let words = "I buy the travelers clothes from Marek for 2 GP.";
    let line = unjudged_line(words);
    let sale = travelers_clothes_sale();
    let buyback = terms(TradeDirection::HeroSells);
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let trade_terms = [sale.clone(), buyback.clone()];
    let sold_out_terms = [sold_out, buyback];

    // Assert: a neutral line is not a trade, so no item is referenced, priced or refused.
    assert_eq!(line.intent.meaning, HeroTradeMeaning::NotTrade);
    assert!(!line.intent.is_trade());
    assert_eq!(
        referenced_item_id(&trade_terms, "sigrun", "marek", line, None),
        None
    );
    assert_eq!(
        fixed_price_purchase(&trade_terms, "sigrun", line, None),
        None
    );
    assert_eq!(
        discount_ask(&trade_terms, "sigrun", "marek", line, None),
        None
    );
    assert_eq!(
        firm_buyback_quote_cp(&trade_terms, "sigrun", "marek", line, None),
        None
    );
    assert_eq!(
        sold_out_purchase_item(&sold_out_terms, "sigrun", "marek", line, None),
        None
    );
    assert_eq!(
        sold_out_refusal(&sold_out_terms, "sigrun", "marek", line, None),
        None
    );
    // The merchant side is neutral too: a bargain that no Jev label opened makes no quote.
    assert_eq!(
        quote_candidate(
            &sale,
            "sigrun",
            "marek",
            &unjudged_turn(words, "I can do 150 copper for the traveler's clothes."),
        ),
        Err(QuoteRejection::NotBargaining)
    );
}

#[test]
fn without_jev_a_merchant_price_makes_no_candidate() {
    // Arrange: the hero's bid is judged offer_price, but the merchant's reply has no Jev label.
    let request = "I'll pay you 150 copper for the traveler's clothes.";
    let reply = "I can do 150 copper for the traveler's clothes.";
    let merchant = merchant_commitment_from_judgement(Err(JevUnavailable::ProviderUnavailable));
    let turn = TradeExchange {
        hero: jev_line("offer_price", request),
        merchant,
        merchant_reply: reply,
    };
    let sale = travelers_clothes_sale();

    // Assert
    assert_eq!(merchant.meaning, MerchantMeaning::NotTrade);
    assert_eq!(merchant.judgement.reason_label(), "provider_unavailable");
    assert_eq!(
        quote_candidate(&sale, "sigrun", "marek", &turn),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate_for_reply(
            std::slice::from_ref(&sale),
            "sigrun",
            "marek",
            &turn,
            Some(CLOTHES_ITEM),
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
}

#[tokio::test]
async fn judge_trade_item_without_a_provider_is_a_neutral_provider_unavailable() {
    // Arrange: the default approach model has no provider, so no network call can happen.
    let listed = [
        terms(TradeDirection::HeroBuys),
        terms(TradeDirection::HeroSells),
    ];
    let choices = TradeItemChoices::new(&listed, "sigrun", "marek").expect("two listed items");

    // Act
    let picked = judge_trade_item(
        &ApproachModel::default(),
        &choices,
        "How much for the fine clothes?",
        "I can do 150 copper for the traveler's clothes.",
    )
    .await;

    // Assert
    assert_eq!(picked, Err(JevUnavailable::ProviderUnavailable));
}

#[test]
fn trade_item_choices_need_a_listed_item_and_end_in_a_none_position() {
    let listed = [
        terms(TradeDirection::HeroBuys),
        terms(TradeDirection::HeroSells),
    ];
    assert!(
        TradeItemChoices::new(&[], "sigrun", "marek").is_none(),
        "no listed item, no choice"
    );
    assert!(
        TradeItemChoices::new(&listed, "another-hero", "marek").is_none(),
        "another hero's items are not this hero's choice"
    );

    let choices = TradeItemChoices::new(&listed, "sigrun", "marek").expect("two listed items");
    let criteria = choices.criteria();
    assert_eq!(criteria.len(), 3);
    assert!(criteria[0].contains("merchant sells"));
    assert!(criteria[1].contains("merchant buys"));
    assert!(criteria[2].starts_with("none:"));
    assert_eq!(choices.item_for(0), Some(CLOTHES_ITEM));
    assert_eq!(choices.item_for(2), None);
}

/// The two sources whose trade and scene code must hold no keyword tables for meaning.
const SCANNED_SOURCES: [(&str, &str); 2] = [
    (
        "merchant_bargaining.rs",
        include_str!("merchant_bargaining.rs"),
    ),
    ("living_scene.rs", include_str!("living_scene.rs")),
];

/// The only string-table constants the scan allows, by (file, name). Each is a closed Jev label set
/// of (label, description) pairs passed to `ApproachModel::choose_label`; Jev's chosen label is the
/// meaning, and these strings never decide a meaning by matching. Currency unit names are matched
/// in match arms, not in const arrays, so they need no exclusion.
const JEV_LABEL_SETS: [(&str, &str); 3] = [
    // HeroTradeMeaning and MerchantMeaning: the hero-line and merchant-reply label sets.
    ("merchant_bargaining.rs", "CHOICES"),
    // InteractionMeaning: the scene interaction label set.
    ("living_scene.rs", "CHOICES"),
    // The living scene's closed demand label set.
    ("living_scene.rs", "DEMAND_LABELS"),
];

/// The name of a `const` or `static` item on this line whose declared type is an array or slice
/// containing `str`: a table of string literals or string tuples.
fn string_table_name(line: &str) -> Option<&str> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let keyword = tokens
        .iter()
        .position(|token| matches!(*token, "const" | "static"))?;
    if tokens.get(keyword + 1) == Some(&"fn") {
        return None;
    }
    let name = tokens.get(keyword + 1).copied()?.trim_end_matches(':');
    let declaration = tokens[keyword + 1..].join(" ");
    let (_, declared) = declaration.split_once(':')?;
    let ty = declared.trim_start().trim_start_matches('&').trim_start();
    let ty = ty.strip_prefix("'static").unwrap_or(ty).trim_start();
    (ty.starts_with('[') && ty.contains("str")).then_some(name)
}

#[test]
fn trade_and_scene_code_holds_no_keyword_tables_for_meaning() {
    let mut violations = Vec::new();
    for (file, source) in SCANNED_SOURCES {
        for (index, line) in source.lines().enumerate() {
            let number = index + 1;
            if let Some(name) = string_table_name(line) {
                let exempt = JEV_LABEL_SETS
                    .iter()
                    .any(|(exempt_file, exempt_name)| *exempt_file == file && *exempt_name == name);
                if !exempt {
                    violations.push(format!("{file}:{number}: string table `{name}`"));
                }
            }
            if line.contains("contains_phrase(") || line.contains("starts_with(prefix)") {
                violations.push(format!("{file}:{number}: phrase matching"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "keyword tables or phrase matching must not decide meaning:\n{}",
        violations.join("\n")
    );
}

fn in_stock_listed_ids(terms: &[TradeTerms], count: usize) -> Vec<String> {
    terms
        .iter()
        .filter(|term| {
            term.direction == TradeDirection::HeroBuys && term.available && term.stock_units > 0
        })
        .take(count)
        .map(|term| term.item_id.clone())
        .collect()
}

#[test]
fn a_bulk_pick_keeps_only_listed_ids_once_and_text_that_is_not_an_array_picks_nothing() {
    let terms = authored_sale_terms("sigrun");
    let choices = TradeItemChoices::new(&terms, "sigrun", "merchant").unwrap();
    let ids = in_stock_listed_ids(&terms, 2);
    let text = json!([ids[0], "not-listed", ids[0], ids[1]]).to_string();
    assert_eq!(parse_bulk_picks(&text, &choices), ids);
    assert!(parse_bulk_picks("the cloak and the gloves", &choices).is_empty());
}

#[test]
fn a_bulk_ask_sums_the_picked_in_stock_offers_and_one_pick_is_no_bulk_ask() {
    let terms = authored_sale_terms("sigrun");
    let ids = in_stock_listed_ids(&terms, 2);
    let ask = bulk_ask_from_picks(&terms, "sigrun", "merchant", &ids).unwrap();
    assert_eq!(ask.items.len(), 2);
    let listed: u32 = ask.items.iter().map(|item| item.listed_cp).sum();
    assert_eq!(ask.listed_total_cp(), listed);
    assert!(bulk_ask_from_picks(&terms, "sigrun", "merchant", &ids[..1]).is_none());

    let mut context = json!({"negotiation_rules": []});
    attach_bulk_ask(&mut context, &ask);
    assert_eq!(context["bulk_ask"]["listed_total_cp"], json!(listed));
    assert!(
        context["negotiation_rules"]
            .to_string()
            .contains("sold separately")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn the_storyteller_bulk_pick_is_validated_against_the_listed_items() {
    let terms = authored_sale_terms("sigrun");
    let choices = TradeItemChoices::new(&terms, "sigrun", "merchant").unwrap();
    let ids = in_stock_listed_ids(&terms, 2);
    let cache = cache_directory("bulk-pick");
    let endpoint = FakeEndpoint::start(vec![Reply::Output(
        json!({"picks": json!([ids[0], "unlisted", ids[1]]).to_string()}),
    )]);
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(30), &cache);
    let picks = judged_bulk_picks(
        &storyteller,
        &choices,
        "How much for the cloak and the gloves?",
    )
    .await
    .unwrap();
    assert_eq!(picks, ids);
    assert_eq!(endpoint.bodies().len(), 1);
}

#[test]
fn haggling_over_two_cloaks_reads_the_bare_seven_gold_offer_against_the_fur_lined_floor() {
    let terms = authored_sale_terms("sigrun");
    let fur = terms
        .iter()
        .find(|term| term.item_name == "fur-lined cloak")
        .unwrap()
        .clone();
    assert_eq!(fur.maximum_price_cp, 1_000);
    assert_eq!(fur.minimum_price_cp, 500);
    let words = "Ten gold for the fur-lined cloak is steep, Marek. Would you take seven? We're buying two cloaks today.";
    let line = jev_line("offer_price", words);
    let ask = discount_ask(&terms, "sigrun", "merchant", line, Some(&fur.item_id))
        .expect("the bare seven inherits gold and is the haggle offer, not the two-cloak count");
    assert_eq!(ask.named_cp, 700);
    assert_eq!(ask.listed_cp, 1_000);
    assert_eq!(ask.lowest_cp, 500);
    assert_eq!(ask.offer_cp(), 700);
    assert_eq!(ask.counteroffer_cp(), 700);

    let context = reply_context(
        &terms,
        "sigrun",
        "merchant",
        line,
        focus_picked(&fur.item_id),
        &json!({"npc": {"name": "Marek", "role": "merchant"}}),
    )
    .unwrap();
    let entry = context["available_trades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"] == "fur-lined cloak")
        .unwrap();
    assert_eq!(entry["hero_offer_cp"], 700);
    assert_eq!(entry["lowest_price_cp"], 500);
    let rules = context["negotiation_rules"].to_string();
    assert!(rules.contains("counteroffer at lowest_price_cp"), "{rules}");
    assert!(rules.contains("exactly one"), "{rules}");

    let candidate = quote_candidate(
        &fur,
        "sigrun",
        "merchant",
        &jev_turn(
            "offer_price",
            words,
            "firm_price",
            "I can do 700 copper for the fur-lined cloak.",
        ),
    )
    .unwrap();
    assert_eq!(candidate.price_cp, 700);
}

#[test]
fn a_haggle_below_the_floor_counters_at_the_floor_not_at_the_listed_price() {
    let terms = authored_sale_terms("sigrun");
    let fur_id = terms
        .iter()
        .find(|term| term.item_name == "fur-lined cloak")
        .unwrap()
        .item_id
        .clone();
    let line = jev_line(
        "offer_price",
        "Would you take four gold for the fur-lined cloak?",
    );
    let ask = discount_ask(&terms, "sigrun", "merchant", line, Some(&fur_id)).unwrap();
    assert_eq!(ask.named_cp, 400);
    assert_eq!(
        ask.counteroffer_cp(),
        500,
        "the counteroffer is the floor, never below it and never the listed price"
    );
}

#[test]
fn a_multi_item_purchase_gets_a_bounded_bundle_discount_after_a_successful_check() {
    let terms = authored_sale_terms("sigrun");
    let ids: Vec<String> = ["warm wool cloak", "fur-lined cloak"]
        .iter()
        .map(|name| {
            terms
                .iter()
                .find(|term| term.item_name == *name)
                .unwrap()
                .item_id
                .clone()
        })
        .collect();
    let line = jev_line(
        "offer_price",
        "Would you take eleven gold for the warm wool cloak and the fur-lined cloak together?",
    );
    let ask = bundle_discount_ask(&terms, "sigrun", "merchant", line, &ids)
        .expect("a named total below the combined listing is a bundle discount ask");
    assert_eq!(ask.listed_total_cp(), 1_300);
    assert_eq!(ask.lowest_total_cp(), 650);
    assert_eq!(ask.offer_total_cp(), 1_100);
    let mut total = 0;
    for item in &ask.items {
        let offer = ask.item_offer_cp(&item.item_id).unwrap();
        assert!(
            (item.lowest_cp..=item.listed_cp).contains(&offer),
            "{item:?}"
        );
        total += offer;
    }
    assert_eq!(total, 1_100);

    let mut facts = json!({"available_trades": [
        {"item": "warm wool cloak", "direction": "merchant_sells", "listed_price_cp": 300},
        {"item": "fur-lined cloak", "direction": "merchant_sells", "listed_price_cp": 1000}
    ]});
    set_bundle_offer(&mut facts, &ask, true);
    let offered: u32 = facts["available_trades"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["hero_offer_cp"].as_u64().unwrap() as u32)
        .sum();
    assert_eq!(offered, 1_100);
    set_bundle_offer(&mut facts, &ask, false);
    assert!(
        facts["available_trades"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("hero_offer_cp").is_none())
    );

    let mut context = json!({"negotiation_rules": []});
    attach_bulk_ask(&mut context, &ask.ask);
    assert_eq!(context["bulk_ask"]["lowest_total_cp"], 650);
    let rules = context["negotiation_rules"].to_string();
    assert!(rules.contains("bundle"), "{rules}");
    assert!(!rules.contains("Do not promise a bundle price"), "{rules}");
}

#[test]
fn an_unlisted_service_must_be_quoted_at_one_price_or_declined_with_a_reason() {
    let facts = service_facts(&[]);
    let rules = facts["services_rules"].to_string();
    assert!(rules.contains("not listed"), "{rules}");
    assert!(rules.contains("one exact"), "{rules}");
    assert!(rules.contains("decline"), "{rules}");
    assert!(!SERVICE_DECISION_INSTRUCTIONS.contains("oilcloth"));

    let accepts = service_decision_from_judgement(Ok("accepts_at_price"));
    assert_eq!(
        unlisted_service_quote_cp(accepts, "I can wrap it in oilcloth for 20 copper."),
        Ok(20)
    );
    let declines = service_decision_from_judgement(Ok("declines_with_reason"));
    assert_eq!(
        unlisted_service_quote_cp(declines, "I cannot wrap that; I have no oilcloth to spare."),
        Err(QuoteRejection::MerchantRefused)
    );
    let hedges = service_decision_from_judgement(Ok("hedges_or_asks"));
    assert_eq!(
        unlisted_service_quote_cp(hedges, "Maybe I could wrap it, for some trouble."),
        Err(QuoteRejection::NoMerchantCommitment),
        "an evasive hedge is not a service quote"
    );
}

#[test]
fn only_the_selling_npc_may_quote_or_offer_stock() {
    let terms = authored_sale_terms("sigrun");
    let fur = terms
        .iter()
        .find(|term| term.item_name == "fur-lined cloak")
        .unwrap();
    let turn = jev_turn(
        "ask_price",
        "Jessa, how much for the fur-lined cloak?",
        "firm_price",
        "I can do 1000 copper for the fur-lined cloak.",
    );
    assert_eq!(
        quote_candidate(fur, "sigrun", "shop-soldier", &turn),
        Err(QuoteRejection::InvalidTerms),
        "a bystander is not the seller on the term, so her price makes no quote"
    );
    assert!(
        reply_context(
            &terms,
            "sigrun",
            "shop-soldier",
            jev_line("ask_price", "Jessa, how much for the fur-lined cloak?"),
            focus_picked(&fur.item_id),
            &json!({"npc": {"name": "Jessa", "role": "off-duty soldier"}}),
        )
        .is_none(),
        "a bystander receives no trade context to quote from"
    );
    assert!(HERO_TRADE_INSTRUCTIONS.contains("may quote or offer stock"));
}

#[tokio::test(flavor = "current_thread")]
async fn without_a_storyteller_the_bulk_pick_is_neutral() {
    let terms = authored_sale_terms("sigrun");
    let choices = TradeItemChoices::new(&terms, "sigrun", "merchant").unwrap();
    assert!(
        judged_bulk_picks(
            &Storyteller::disabled(),
            &choices,
            "How much for the cloak and the gloves?"
        )
        .await
        .is_err()
    );
}
