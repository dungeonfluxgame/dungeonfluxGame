use serde_json::json;

use super::merchant_bargaining::{
    HERO_TRADE_INSTRUCTIONS, HeroLine, HeroTradeIntent, HeroTradeMeaning, ListedQuoteAction,
    MerchantBuybackMemory, MerchantMeaning, MerchantReplyCommitment, QuoteDecision, QuoteRejection,
    TradeDirection, TradeExchange, TradeJudgement, TradeTerms, attach_buyback_memory,
    constrain_buyback_ceiling, discount_ask, discount_willingness, explicit_price_cp,
    firm_buyback_quote_cp, fixed_price_purchase, hero_intent_from_judgement, hero_judgement_state,
    hero_stated_price_cp, is_trade_dialogue_request, listed_quote_action,
    merchant_commitment_from_judgement, quote_candidate, quote_candidate_for_item,
    quote_candidate_for_reply, quote_decision, referenced_item_id, reply_context,
    reply_context_for_quote, revalidate_quote, rules_hero_intent, rules_merchant_commitment,
    rules_quote_decision, set_discount_offer, sold_out_purchase_item, sold_out_refusal,
    trade_influence_applies,
};
use crate::gameplay::approach_model::JevUnavailable;
use crate::gameplay::social::InfluenceWillingness;

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
        item_id: "item-clothes-travelers".into(),
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

/// The phrase fallback reading of one hero line. Used only where Jev is unavailable, which is the
/// behaviour these tests pin down.
fn rules_line(words: &str) -> HeroLine<'_> {
    HeroLine {
        intent: rules_hero_intent(words, None, JevUnavailable::ProviderUnavailable),
        words,
    }
}

/// The phrase fallback reading of one negotiation turn.
fn rules_turn<'a>(player_words: &'a str, merchant_reply: &'a str) -> TradeExchange<'a> {
    TradeExchange {
        hero: rules_line(player_words),
        merchant: rules_merchant_commitment(merchant_reply, JevUnavailable::ProviderUnavailable),
        merchant_reply,
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
        hero: HeroLine {
            intent: HeroTradeIntent::from_label(hero_label).expect("hero label is in the Jev set"),
            words: player_words,
        },
        merchant: MerchantReplyCommitment::from_label(merchant_label)
            .expect("merchant label is in the Jev set"),
        merchant_reply,
    }
}

#[test]
fn seller_quotes_are_exact_bounded_and_directional() {
    let purchase = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &rules_turn(
            "Can you do better on the fine clothes?",
            "I can do the fine clothes for 1,200 copper pieces.",
        ),
    )
    .unwrap();
    assert_eq!(purchase.item_id, "item-clothes-travelers");
    assert_eq!(purchase.offer_id.as_deref(), Some("clothes-travelers"));
    assert_eq!(purchase.seller_id, "marek");
    assert_eq!(purchase.buyer_id, "sigrun");
    assert_eq!(purchase.price_cp, 1_200);

    let buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &rules_turn(
            "Would you take this fine clothes item back?",
            "I can buy the fine clothes for 12 gold.",
        ),
    )
    .unwrap();
    assert_eq!(buyback.item_id, "item-clothes-travelers");
    assert_eq!(buyback.seller_id, "sigrun");
    assert_eq!(buyback.buyer_id, "marek");
    assert_eq!(buyback.price_cp, 1_200);

    let requested_buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &rules_turn(
            "What's your offer for the fine clothes?",
            "I'll pay 12 gold for the fine clothes.",
        ),
    )
    .unwrap();
    assert_eq!(requested_buyback, buyback);

    let contextual = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &rules_turn(
            "Could you do better for this?",
            "I can do it for 1,200 copper.",
        ),
    )
    .unwrap();
    assert_eq!(contextual.item_id, "item-clothes-travelers");
    assert_eq!(contextual.price_cp, 1_200);

    let natural_buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        &rules_turn(
            "What would you pay for the fine clothes?",
            "I offer 12 gold for the fine clothes.",
        ),
    )
    .unwrap();
    assert_eq!(natural_buyback.price_cp, 1_200);

    let natural_buy = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        &rules_turn(
            "Can you do better on the fine clothes?",
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
    assert!(is_trade_dialogue_request(request));
    assert_eq!(
        referenced_item_id(
            &[sale_terms.clone()],
            "sigrun",
            "marek",
            rules_line(request)
        )
        .as_deref(),
        Some("item-clothes-travelers")
    );
    let candidate = quote_candidate_for_reply(
        &[sale_terms],
        "sigrun",
        "marek",
        &rules_turn(request, "I can pay 1 GP for the travelers clothes."),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.item_id, "item-clothes-travelers");
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

    assert!(is_trade_dialogue_request(request));
    assert_eq!(
        referenced_item_id(&trade_terms, "sigrun", "marek", rules_line(request)).as_deref(),
        Some("item-clothes-travelers")
    );
    assert!(constrain_buyback_ceiling(
        &mut trade_terms,
        "sigrun",
        "marek",
        "item-clothes-travelers",
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
        &rules_turn(request, "I can pay 199 copper for the travelers clothes."),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.price_cp, 199);
    assert_eq!(
        quote_candidate_for_reply(
            &trade_terms,
            "sigrun",
            "marek",
            &rules_turn(request, "I can pay 2 gold for the travelers clothes."),
        ),
        Err(QuoteRejection::OutOfRange)
    );

    let mut context = reply_context(
        &trade_terms,
        "sigrun",
        "marek",
        rules_line(request),
        Some("item-clothes-travelers"),
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
        referenced_item_id(&[sale], "sigrun", "marek", rules_line(request)).as_deref(),
        Some("item-clothes-travelers")
    );
}

#[test]
fn buyer_bid_is_not_misread_as_the_hero_selling_the_merchant_item() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    let trade_terms = [buy_terms, terms(TradeDirection::HeroSells)];
    let request = "I offer 1 GP for those travelers clothes.";
    assert!(is_trade_dialogue_request(request));
    assert_eq!(
        referenced_item_id(&trade_terms, "sigrun", "marek", rules_line(request)),
        None
    );
    let candidate = quote_candidate_for_reply(
        &trade_terms,
        "sigrun",
        "marek",
        &rules_turn(request, "I can sell the travelers clothes for 2 GP."),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.price_cp, 200);
}

#[test]
fn counteroffer_keeps_the_live_quote_item_and_trade_direction() {
    let request = "What about 11 GP?";
    assert!(is_trade_dialogue_request(request));
    let candidate = quote_candidate_for_item(
        &[terms(TradeDirection::HeroSells)],
        "sigrun",
        "marek",
        TradeDirection::HeroSells,
        "item-clothes-travelers",
        &rules_turn(
            request,
            "I can meet you at 12 GP for the travelers clothes.",
        ),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.item_id, "item-clothes-travelers");
    assert_eq!(candidate.price_cp, 1_200);
    assert!(is_trade_dialogue_request(
        "Could you lower the price on this?"
    ));
}

#[test]
fn quote_needs_a_bargaining_request_a_merchant_commitment_and_one_price() {
    let request = "Could you lower the price on the fine clothes?";
    for (reply, expected) in [
        (
            "Maybe, what did you have in mind?",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "Maybe I can do the fine clothes for 1,200 copper.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "I think I can do the fine clothes for 1,200 copper.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "No, I can't do 900 copper.",
            QuoteRejection::MerchantRefused,
        ),
        (
            "I can do the fine clothes for 1,200 copper if you buy both.",
            QuoteRejection::NoMerchantCommitment,
        ),
        (
            "I can do 1,200 copper or 12 gold.",
            QuoteRejection::AmbiguousAmount,
        ),
        (
            "I can't agree to one gold.",
            QuoteRejection::MerchantRefused,
        ),
        (
            "I can't take less for them.",
            QuoteRejection::MerchantRefused,
        ),
        ("I could do one gold.", QuoteRejection::NoMerchantCommitment),
    ] {
        assert_eq!(
            quote_candidate(
                &terms(TradeDirection::HeroBuys),
                "sigrun",
                "marek",
                &rules_turn(request, reply),
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
            &rules_turn(
                "Tell me about the fine clothes.",
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
            &rules_turn(
                "How much for the fine clothes?",
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
            &rules_turn(
                "How much for the travelers clothes?",
                "The set costs two gold pieces. It’s practical traveler’s clothing in sturdy wool and linen.",
            ),
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
            &rules_turn(
                "How much for the travelers clothes?",
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
                &rules_turn("How much for the travelers clothes?", reply),
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
            &rules_turn(request, "I can do 2 fine clothes for 1,200 copper pieces.",),
        ),
        Err(QuoteRejection::AmbiguousAmount)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(request, "My price is 1,200 copper."),
        ),
        Err(QuoteRejection::ItemUnclear)
    );
    // The non-cash words in this reply are a judged meaning (conditional_or_future), so the
    // fallback now reports no merchant commitment; before, it reported an ambiguous amount. Both
    // reject the quote.
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(
                request,
                "I can do the fine clothes for 1,200 copper plus a favor.",
            ),
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(request, "I can do the fine clothes for 1,200 copper?",),
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(request, "No, I can do the fine clothes for 1,200 copper.",),
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
    assert_eq!(
        explicit_price_cp("about 1,200 cp"),
        Err(QuoteRejection::AmbiguousAmount)
    );
    assert_eq!(
        explicit_price_cp("1,200 cp plus a favor"),
        Err(QuoteRejection::AmbiguousAmount)
    );
    // The parser reads the exact amount only. "next week" is a meaning, judged by Jev as
    // conditional_or_future, and by the phrase fallback in rules_merchant_commitment.
    assert_eq!(explicit_price_cp("12 gp next week"), Ok(1_200));
    assert_eq!(
        rules_merchant_commitment(
            "I can do the fine clothes for 12 gp next week.",
            JevUnavailable::ProviderUnavailable,
        )
        .meaning,
        MerchantMeaning::ConditionalOrFuture
    );
    assert_eq!(
        explicit_price_cp("I'll buy it from you for 12 gp"),
        Ok(1_200)
    );
}

#[test]
fn natural_price_questions_enter_the_merchant_reply_path() {
    for words in [
        "How much for the travelers clothes?",
        "What do the travelers clothes cost?",
        "What is the price of those clothes?",
    ] {
        assert!(is_trade_dialogue_request(words), "{words}");
    }
}

#[test]
fn direct_purchase_requires_an_explicit_commitment_at_the_current_listing_price() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            rules_line("I buy the travelers clothes from Marek for 2 GP."),
        ),
        Some(("item-clothes-travelers".into(), 200))
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            rules_line("I offer 2 GP for those travelers clothes."),
        ),
        None
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            rules_line("Could I buy the travelers clothes for 2 GP?"),
        ),
        None
    );
    assert_eq!(
        fixed_price_purchase(
            std::slice::from_ref(&buy_terms),
            "sigrun",
            rules_line("I buy the travelers clothes for 13 GP."),
        ),
        None
    );
    let mut unavailable = buy_terms.clone();
    unavailable.available = false;
    unavailable.stock_units = 0;
    assert_eq!(
        fixed_price_purchase(
            &[unavailable],
            "sigrun",
            rules_line("I buy the travelers clothes for 2 GP."),
        ),
        None
    );
}

#[test]
fn fixed_price_purchase_follows_jev_buy_now_not_the_phrase_list() {
    let mut buy_terms = terms(TradeDirection::HeroBuys);
    buy_terms.minimum_price_cp = 100;
    buy_terms.maximum_price_cp = 200;
    let words = "The travelers clothes are mine at 2 GP, I suppose.";
    let jev_buy_now = HeroLine {
        intent: HeroTradeIntent::from_label("buy_now").unwrap(),
        words,
    };
    assert_eq!(
        fixed_price_purchase(std::slice::from_ref(&buy_terms), "sigrun", jev_buy_now),
        Some(("item-clothes-travelers".into(), 200))
    );
    let jev_offer = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "I buy the travelers clothes for 2 GP.",
    };
    assert_eq!(
        fixed_price_purchase(std::slice::from_ref(&buy_terms), "sigrun", jev_offer),
        None,
        "a bid is never a commitment, whatever its wording"
    );
}

#[test]
fn player_must_accept_the_live_quote_price_or_make_a_counteroffer() {
    assert_eq!(rules_quote_decision("Deal.", 1_200), QuoteDecision::Accept);
    assert_eq!(
        rules_quote_decision("I buy the fine clothes for 12 gold.", 1_200),
        QuoteDecision::Accept
    );
    assert_eq!(
        rules_quote_decision("I buy the fine clothes for 1,300 cp instead.", 1_200),
        QuoteDecision::Counteroffer
    );
    assert_eq!(
        rules_quote_decision("No, how about 1,300 cp?", 1_200),
        QuoteDecision::Counteroffer
    );
    assert_eq!(
        rules_quote_decision("Maybe, I accept.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        rules_quote_decision("Yes, but I need a moment.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        rules_quote_decision("I accept 12 gp next week.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        rules_quote_decision("No, thank you.", 1_200),
        QuoteDecision::Decline
    );
    assert_eq!(
        rules_quote_decision("Yes, but only if you include the belt.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        rules_quote_decision("1,200 cp?", 1_200),
        QuoteDecision::Unclear
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
    let firm = MerchantReplyCommitment::from_label("firm_price").unwrap();
    assert!(
        firm.binds_active_item,
        "a judged firm price binds the item in discussion"
    );
    let hedged = MerchantReplyCommitment::from_label("hedges_or_asks").unwrap();
    assert!(!hedged.binds_active_item);
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
    let candidate =
        quote_candidate_for_reply(std::slice::from_ref(&sale), "sigrun", "marek", &turn)
            .expect("a judged firm price in bounds is a quote");
    assert_eq!(candidate.item_id, "item-clothes-travelers");
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.price_cp, 150);
}

#[test]
fn judged_merchant_meanings_decide_the_candidate_before_any_phrase_is_read() {
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
fn judgement_from_jev_or_the_fallback_is_recorded_on_the_typed_value() {
    let words = "Could you lower the price on the fine clothes?";
    let from_jev = hero_intent_from_judgement(Ok("ask_price"), words, None);
    assert_eq!(from_jev.judgement, TradeJudgement::Jev);
    assert_eq!(from_jev.meaning, HeroTradeMeaning::AskPrice);

    let unavailable =
        hero_intent_from_judgement(Err(JevUnavailable::ProviderUnavailable), words, None);
    assert_eq!(
        unavailable,
        rules_hero_intent(words, None, JevUnavailable::ProviderUnavailable),
        "with Jev unavailable the phrase fallback reads the line as it always did"
    );
    assert_eq!(
        unavailable.judgement,
        TradeJudgement::Rules(JevUnavailable::ProviderUnavailable)
    );
    assert_eq!(unavailable.judgement.reason_label(), "provider_unavailable");

    let merchant = merchant_commitment_from_judgement(
        Err(JevUnavailable::Timeout),
        "I can do the fine clothes for 1,200 copper.",
    );
    assert_eq!(merchant.meaning, MerchantMeaning::FirmPrice);
    assert_eq!(merchant.judgement.source_label(), "rules");
    assert_eq!(merchant.judgement.reason_label(), "timeout");

    let unknown = merchant_commitment_from_judgement(
        Ok("definitely_a_price"),
        "I can do the fine clothes for 1,200 copper.",
    );
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
        rules_line("How much for the fine clothes?"),
        None,
        &first,
    )
    .unwrap();
    let second_context = reply_context(
        &[terms(TradeDirection::HeroBuys)],
        "sigrun",
        "marek",
        rules_line("How much for the fine clothes?"),
        None,
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
            rules_line("How much for the fine clothes?"),
            None,
            facts,
        )
        .unwrap();
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(
                "Can you offer a lower price on the fine clothes?",
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
        item_id: "item-clothes-travelers".into(),
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
        rules_line("What about 11 GP?"),
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
        rules_line("Could I buy the fine clothes?"),
        None,
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
        rules_line("How much for the fine clothes?"),
        None,
        &facts,
    )
    .unwrap();
    assert_eq!(buying["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(buying["available_trades"][0]["direction"], "merchant_sells");
    assert_eq!(buying["available_trades"][0]["item"], "fine clothes");

    let selling = reply_context(
        &[terms(TradeDirection::HeroBuys), owned_item],
        "sigrun",
        "marek",
        rules_line("What would you pay for my silver brooch?"),
        None,
        &facts,
    )
    .unwrap();
    assert_eq!(selling["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(selling["available_trades"][0]["direction"], "merchant_buys");
    assert_eq!(selling["available_trades"][0]["item"], "silver brooch");

    let mut silver_brooch = terms(TradeDirection::HeroSells);
    silver_brooch.item_id = "item-silver-brooch".into();
    silver_brooch.item_name = "silver brooch".into();
    silver_brooch.aliases.clear();
    let selling_by_how_much = reply_context(
        &[terms(TradeDirection::HeroBuys), silver_brooch],
        "sigrun",
        "marek",
        rules_line("How much would you pay for my silver brooch?"),
        None,
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
        rules_line("What would you pay for my item?"),
        None,
        &facts,
    );
    assert!(other_hero_view.is_none());
}

#[test]
fn reply_context_shares_named_stockout_and_not_unmentioned_unavailable_stock() {
    let mut sold_out = terms(TradeDirection::HeroBuys);
    sold_out.available = false;
    sold_out.stock_units = 0;
    let facts = json!({"npc": {"name": "Marek", "role": "merchant"}});

    let asked = reply_context(
        &[sold_out.clone()],
        "sigrun",
        "marek",
        rules_line("Do you still have the fine clothes?"),
        None,
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
        rules_line("What are you selling?"),
        None,
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
            &rules_turn(request, "I can do the fine clothes for 1,200 copper."),
        ),
        Err(QuoteRejection::Unavailable)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "someone-else",
            "marek",
            &rules_turn(request, "I can do the fine clothes for 1,200 copper."),
        ),
        Err(QuoteRejection::InvalidTerms)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(request, "I can do the fine clothes for 900 copper."),
        ),
        Err(QuoteRejection::OutOfRange)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            &rules_turn(request, "I can do the fine clothes for 1,600 copper."),
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
        rules_line("I'll pay you 150 copper for the traveler's clothes."),
        None,
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
        rules_line("How much for the traveler's clothes?"),
        None,
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
        &rules_turn(request, "I can do 150 copper for the traveler's clothes."),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroBuys);
    assert_eq!(candidate.item_id, "item-clothes-travelers");
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
        rules_line("I buy the traveler's clothes for 2 gold."),
        None,
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
        quote_candidate(&sold, "sigrun", "marek", &rules_turn(request, reply)),
        Err(QuoteRejection::Unavailable)
    );
    assert_eq!(
        quote_candidate_for_item(
            &[sold],
            "sigrun",
            "marek",
            TradeDirection::HeroBuys,
            "item-clothes-travelers",
            &rules_turn(request, reply),
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
        rules_line(request),
        Some("item-clothes-travelers"),
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
        &rules_turn(request, "I can offer 90 copper for them."),
    )
    .unwrap();
    assert_eq!(candidate.direction, TradeDirection::HeroSells);
    assert_eq!(candidate.price_cp, 90);
    assert_eq!(
        quote_candidate(
            &buyback,
            "sigrun",
            "marek",
            &rules_turn(request, "I can offer 150 copper for them."),
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
            &rules_turn(
                "I'll pay you 50 copper for the traveler's clothes.",
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
            quote_candidate(&sale, "sigrun", "marek", &rules_turn(request, reply)),
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
            "item-clothes-travelers",
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
            "item-clothes-travelers",
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
            "item-clothes-travelers",
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
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("sell_to_merchant").unwrap(),
        words: line_words,
    };
    let context = reply_context(
        &terms,
        "sigrun",
        "marek",
        line,
        Some("item-clothes-travelers"),
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
    let line_words = "I'll buy the traveler's clothes for 1 gold.";
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: line_words,
    };
    let context = reply_context(
        &[available_sale, buyback],
        "sigrun",
        "marek",
        line,
        None,
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
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("sell_to_merchant").unwrap(),
        words: "Marek, would you buy these traveler's clothes back from me for 1 gold?",
    };

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", line),
        Some(("item-clothes-travelers".into(), 100)),
        "a named price inside the 1 to 100 CP buyback terms is a firm figure, not an Influence ask"
    );
}

#[test]
fn sell_line_naming_no_price_takes_the_buyback_ceiling_and_the_reply_must_name_it() {
    let terms = sold_out_sale_and_buyback();
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("sell_to_merchant").unwrap(),
        words: "You sold them to me a minute ago, Marek. They're untouched. What will you give me for them?",
    };

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", line),
        Some(("item-clothes-travelers".into(), 100)),
        "a sell line that names no price is offered the terms price at once"
    );
    let context = reply_context(
        &terms,
        "sigrun",
        "marek",
        line,
        Some("item-clothes-travelers"),
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
    let outside = HeroLine {
        intent: HeroTradeIntent::from_label("sell_to_merchant").unwrap(),
        words: "Marek, would you buy these traveler's clothes back from me for 5 gold?",
    };
    let ask_price = HeroLine {
        intent: HeroTradeIntent::from_label("ask_price").unwrap(),
        words: "What are you asking for the traveler's clothes?",
    };

    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", outside),
        None
    );
    assert_eq!(
        firm_buyback_quote_cp(&terms, "sigrun", "marek", ask_price),
        None
    );
}

#[test]
fn discount_ask_below_the_listing_offers_the_named_price_held_within_the_bounds() {
    let terms = [travelers_clothes_sale()];
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "Would you take 160 copper for the traveler's clothes?",
    };
    let ask = discount_ask(&terms, "sigrun", "marek", line)
        .expect("a named price below the listing is a discount ask");

    assert_eq!(ask.item_id, "item-clothes-travelers");
    assert_eq!(ask.named_cp, 160);
    assert_eq!(ask.listed_cp, 200);
    assert_eq!(ask.offer_cp(), 160);

    let below_bounds = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "Would you take 50 copper for the traveler's clothes?",
    };
    let floor = discount_ask(&terms, "sigrun", "marek", below_bounds)
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
    let at_listing = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "Would you take 200 copper for the traveler's clothes?",
    };
    assert_eq!(discount_ask(&terms, "sigrun", "marek", at_listing), None);

    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let discount = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "Would you take 160 copper for the traveler's clothes?",
    };
    assert_eq!(
        discount_ask(&[sold_out], "sigrun", "marek", discount),
        None,
        "a sold-out item cannot be discounted"
    );
}

#[test]
fn sold_out_sale_item_names_no_price_and_no_hero_figure_in_the_reply_context() {
    let mut sold_out = travelers_clothes_sale();
    sold_out.available = false;
    sold_out.stock_units = 0;
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("ask_price").unwrap(),
        words: "Marek, I'd like a set of those traveler's clothes too. Same price, two gold?",
    };
    let context = reply_context(
        &[sold_out],
        "sigrun",
        "marek",
        line,
        None,
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
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("buy_now").unwrap(),
        words: "Good, I'll take them. Here's two gold.",
    };

    assert_eq!(
        sold_out_purchase_item(std::slice::from_ref(&sold_out), "sigrun", "marek", line),
        Some("traveler's clothes".into()),
        "a purchase of the only sold-out item is refused as sold out"
    );
    assert_eq!(
        fixed_price_purchase(std::slice::from_ref(&sold_out), "sigrun", line),
        None,
        "no purchase is committed for a sold-out item"
    );
    assert_eq!(
        sold_out_purchase_item(&[travelers_clothes_sale()], "sigrun", "marek", line),
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
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("offer_price").unwrap(),
        words: "Would you take 160 copper for the traveler's clothes?",
    };
    let ask = discount_ask(&terms, "sigrun", "marek", line).expect("a discount ask");
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
    let line = HeroLine {
        intent: HeroTradeIntent::from_label("accept_quote").unwrap(),
        words: "Good, I'll take them. Here's two gold.",
    };
    let refusal = sold_out_refusal(&[sold_out], "sigrun", "marek", line)
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
