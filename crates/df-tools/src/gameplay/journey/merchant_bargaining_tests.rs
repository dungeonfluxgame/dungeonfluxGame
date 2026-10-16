use serde_json::json;

use super::merchant_bargaining::{
    QuoteDecision, QuoteRejection, TradeDirection, TradeTerms, explicit_price_cp, quote_candidate,
    quote_decision, reply_context,
};

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
        minimum_price_cp: 1_000,
        maximum_price_cp: 1_500,
        available: true,
        stock_units: 1,
        stock_revision: 1,
        purse_revision: 1,
    }
}

#[test]
fn seller_quotes_are_exact_bounded_and_directional() {
    let purchase = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        "Can you do better on the fine clothes?",
        "I can do the fine clothes for 1,200 copper pieces.",
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
        "Would you take this fine clothes item back?",
        "I can buy the fine clothes for 12 gold.",
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
        "What's your offer for the fine clothes?",
        "I'll pay 12 gold for the fine clothes.",
    )
    .unwrap();
    assert_eq!(requested_buyback, buyback);

    let contextual = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        "Could you do better for this?",
        "I can do it for 1,200 copper.",
    )
    .unwrap();
    assert_eq!(contextual.item_id, "item-clothes-travelers");
    assert_eq!(contextual.price_cp, 1_200);

    let natural_buyback = quote_candidate(
        &terms(TradeDirection::HeroSells),
        "sigrun",
        "marek",
        "What would you pay for the fine clothes?",
        "I offer 12 gold for the fine clothes.",
    )
    .unwrap();
    assert_eq!(natural_buyback.price_cp, 1_200);

    let natural_buy = quote_candidate(
        &terms(TradeDirection::HeroBuys),
        "sigrun",
        "marek",
        "Can you do better on the fine clothes?",
        "I'll take 1,200 copper for the fine clothes.",
    )
    .unwrap();
    assert_eq!(natural_buy.price_cp, 1_200);
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
    ] {
        assert_eq!(
            quote_candidate(
                &terms(TradeDirection::HeroBuys),
                "sigrun",
                "marek",
                request,
                reply
            ),
            Err(expected),
            "{reply}"
        );
    }
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            "Tell me about the fine clothes.",
            "I can do the fine clothes for 1,200 copper.",
        ),
        Err(QuoteRejection::NotBargaining)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            request,
            "I can do 2 fine clothes for 1,200 copper pieces.",
        ),
        Err(QuoteRejection::AmbiguousAmount)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            request,
            "My price is 1,200 copper.",
        ),
        Err(QuoteRejection::ItemUnclear)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            request,
            "I can do the fine clothes for 1,200 copper plus a favor.",
        ),
        Err(QuoteRejection::AmbiguousAmount)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            request,
            "I can do the fine clothes for 1,200 copper?",
        ),
        Err(QuoteRejection::NoMerchantCommitment)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            request,
            "No, I can do the fine clothes for 1,200 copper.",
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
    ] {
        assert_eq!(explicit_price_cp(speech), Ok(expected), "{speech}");
    }
    assert_eq!(
        explicit_price_cp("1.2 gp"),
        Err(QuoteRejection::InvalidAmount)
    );
    assert_eq!(
        explicit_price_cp("-5 cp"),
        Err(QuoteRejection::InvalidAmount)
    );
    assert_eq!(
        explicit_price_cp("1 gp and 5 sp"),
        Err(QuoteRejection::AmbiguousAmount)
    );
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
    assert_eq!(
        explicit_price_cp("12 gp next week"),
        Err(QuoteRejection::AmbiguousAmount)
    );
    assert_eq!(
        explicit_price_cp("I'll buy it from you for 12 gp"),
        Ok(1_200)
    );
}

#[test]
fn player_must_accept_the_live_quote_price_or_make_a_counteroffer() {
    assert_eq!(quote_decision("Deal.", 1_200), QuoteDecision::Accept);
    assert_eq!(
        quote_decision("I buy the fine clothes for 12 gold.", 1_200),
        QuoteDecision::Accept
    );
    assert_eq!(
        quote_decision("I buy the fine clothes for 1,300 cp instead.", 1_200),
        QuoteDecision::Counteroffer
    );
    assert_eq!(
        quote_decision("No, how about 1,300 cp?", 1_200),
        QuoteDecision::Counteroffer
    );
    assert_eq!(
        quote_decision("Maybe, I accept.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        quote_decision("Yes, but I need a moment.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        quote_decision("I accept 12 gp next week.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(
        quote_decision("No, thank you.", 1_200),
        QuoteDecision::Decline
    );
    assert_eq!(
        quote_decision("Yes, but only if you include the belt.", 1_200),
        QuoteDecision::Unclear
    );
    assert_eq!(quote_decision("1,200 cp?", 1_200), QuoteDecision::Unclear);
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
        "How much for the fine clothes?",
        None,
        &first,
    )
    .unwrap();
    let second_context = reply_context(
        &[terms(TradeDirection::HeroBuys)],
        "sigrun",
        "marek",
        "How much for the fine clothes?",
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
            "How much for the fine clothes?",
            None,
            facts,
        )
        .unwrap();
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            "Can you offer a lower price on the fine clothes?",
            "I can do the fine clothes for 1,200 copper.",
        )
        .unwrap()
    };
    let first_quote = candidate(&first);
    let second_quote = candidate(&second);
    assert_eq!(first_quote, second_quote);
    assert_eq!(first_quote.price_cp, 1_200);
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
        "Could I buy the fine clothes?",
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
        "How much for the fine clothes?",
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
        "What would you pay for my silver brooch?",
        None,
        &facts,
    )
    .unwrap();
    assert_eq!(selling["available_trades"].as_array().unwrap().len(), 1);
    assert_eq!(selling["available_trades"][0]["direction"], "merchant_buys");
    assert_eq!(selling["available_trades"][0]["item"], "silver brooch");

    let other_hero_view = reply_context(
        &[
            terms(TradeDirection::HeroBuys),
            terms(TradeDirection::HeroSells),
        ],
        "another-hero",
        "marek",
        "What would you pay for my item?",
        None,
        &facts,
    );
    assert!(other_hero_view.is_none());
}

#[test]
fn stale_stock_wrong_buyer_and_out_of_range_prices_reject_before_quote_creation() {
    let mut changed_stock = terms(TradeDirection::HeroBuys);
    changed_stock.stock_units = 0;
    changed_stock.available = false;
    assert_eq!(
        quote_candidate(
            &changed_stock,
            "sigrun",
            "marek",
            "Could you do better on the fine clothes?",
            "I can do the fine clothes for 1,200 copper.",
        ),
        Err(QuoteRejection::Unavailable)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "someone-else",
            "marek",
            "Could you do better on the fine clothes?",
            "I can do the fine clothes for 1,200 copper.",
        ),
        Err(QuoteRejection::InvalidTerms)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            "Could you do better on the fine clothes?",
            "I can do the fine clothes for 900 copper.",
        ),
        Err(QuoteRejection::OutOfRange)
    );
    assert_eq!(
        quote_candidate(
            &terms(TradeDirection::HeroBuys),
            "sigrun",
            "marek",
            "Could you do better on the fine clothes?",
            "I can do the fine clothes for 1,600 copper.",
        ),
        Err(QuoteRejection::OutOfRange)
    );
}
