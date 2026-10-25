//! The customer table pace screen.

use super::{plural, section, submit_control};
use crate::view::{
    ButtonKind, InputKind, RadioChoice, TextField, button, checkbox, error_summary, fact, facts,
    form, link, page_wide, radio_group, remote, status_line, text_field,
};
use crate::{
    copy, money,
    node::{Node, el},
    route::Route,
    state::{AllocateEdit, Field, Form, FundingScreen, Intent, PaceEdit},
    wire::{CampaignFunding, FundingView, FundsState, PacePreset, SessionFunding},
};

pub(in crate::view) fn funding(screen: &FundingScreen) -> Node {
    page_wide(
        "Table pace",
        Some(
            "The pace is the most a table may spend in an hour of play. You pay for what is used, never more than that.",
        ),
    )
    .child(remote(&screen.funding, "Loading table pace", |funding| {
        funding_content(screen, funding)
    }))
    .into()
}

fn funding_content(screen: &FundingScreen, funding: &FundingView) -> Node {
    let mut content = el("div").class("funding");
    if funding.is_proposed_pricing {
        content = content.child(
            el("p")
                .child(
                    el("span")
                        .class("pill pill-proposed")
                        .text("Proposed pilot pricing"),
                )
                .text(" These prices are proposed for the pilot and may change."),
        );
    }
    content = content.child(preset_list(funding));
    let mut campaigns = section("campaigns-pace-title", "Your campaigns");
    if funding.campaigns.is_empty() {
        campaigns = campaigns.child(el("p").text(
            "You are not in a campaign yet. Create or join one from your dashboard and its pace appears here.",
        ))
        .child(
            el("p")
                .class("actions")
                .child(link("Go to your dashboard", Route::Dashboard, "btn btn-outline")),
        );
    } else {
        campaigns = campaigns.child(
            el("div").class("stack").children(
                funding
                    .campaigns
                    .iter()
                    .map(|campaign| campaign_panel(screen, funding, campaign)),
            ),
        );
    }
    content.child(campaigns).into()
}

fn preset_price_words(offer_ceiling: Option<u32>) -> Option<String> {
    offer_ceiling.map(|ceiling| format!("{} an hour at most", money::price(ceiling)))
}

fn preset_list(funding: &FundingView) -> Node {
    let rows = funding.presets.iter().map(|offer| {
        let words = match preset_price_words(offer.ceiling_cents_per_hour) {
            Some(price) => format!("{price}. {}", copy::preset_description(offer.preset)),
            None => format!(
                "You choose, from {} to {} an hour. {}",
                money::price(funding.custom_minimum_cents_per_hour),
                money::price(funding.custom_maximum_cents_per_hour),
                copy::preset_description(offer.preset)
            ),
        };
        fact(copy::preset_name(offer.preset), words)
    });
    section("paces-title", "The paces")
        .child(facts(rows.collect()))
        .into()
}

fn campaign_panel(
    screen: &FundingScreen,
    funding: &FundingView,
    campaign: &CampaignFunding,
) -> Node {
    let title_id = format!("pace-{}", campaign.campaign_id);
    let mut panel = el("section")
        .class("panel campaign-pace")
        .attr("aria-labelledby", title_id.clone())
        .child(el("h3").attr("id", title_id).text(campaign.name.clone()))
        .child(facts(vec![
            fact("Current pace", copy::preset_name(campaign.preset)),
            fact(
                "Hourly maximum",
                format!(
                    "{} an hour at most",
                    money::price(campaign.ceiling_cents_per_hour)
                ),
            ),
        ]));
    if screen.saved_campaign.as_deref() == Some(campaign.campaign_id.as_str()) {
        panel = panel.child(status_line(format!(
            "Your change to {} is saved.",
            campaign.name
        )));
    }
    let editing = screen
        .edit
        .as_ref()
        .filter(|edit| edit.campaign_id == campaign.campaign_id);
    panel = match (campaign.can_choose_pace, editing) {
        (true, Some(edit)) => panel.child(pace_form(funding, campaign, edit)),
        (true, None) => panel.child(button(
            "Change pace",
            Some(Intent::EditPace {
                campaign_id: campaign.campaign_id.clone(),
            }),
            ButtonKind::Secondary,
            false,
        )),
        (false, _) => panel.child(
            el("p")
                .class("fine")
                .text("Only the host chooses the pace."),
        ),
    };
    if let Some(session) = &campaign.session {
        panel = panel.child(session_funding(screen, campaign, session));
    }
    panel.into()
}

fn pace_form(funding: &FundingView, campaign: &CampaignFunding, edit: &PaceEdit) -> Node {
    let error = edit.submit.error();
    let choices = funding
        .presets
        .iter()
        .map(|offer| RadioChoice {
            field: Field::PacePreset,
            value: copy::preset_value(offer.preset),
            title: copy::preset_name(offer.preset).to_owned(),
            detail: match preset_price_words(offer.ceiling_cents_per_hour) {
                Some(price) => format!("{price}. {}", copy::preset_description(offer.preset)),
                None => copy::preset_description(offer.preset).to_owned(),
            },
            is_checked: offer.preset == edit.preset,
        })
        .collect();
    let mut pace = form(Form::Pace, &format!("Change pace for {}", campaign.name))
        .child(error_summary(error))
        .child(radio_group("Pace", Field::PacePreset, choices, error));
    if edit.preset == PacePreset::Custom {
        let hint = format!(
            "Between {} and {} an hour.",
            money::price(funding.custom_minimum_cents_per_hour),
            money::price(funding.custom_maximum_cents_per_hour)
        );
        pace = pace.child(text_field(
            TextField {
                field: Field::CustomCeiling,
                label: "Hourly maximum in dollars",
                value: &edit.custom,
                kind: InputKind::Dollars,
                autocomplete: "off",
                hint: Some(&hint),
            },
            error,
        ));
    }
    pace.child(
        el("div")
            .class("actions")
            .child(submit_control(
                &edit.operation,
                "Save pace",
                "Saving pace",
                edit.submit.is_sending(),
            ))
            .child(button(
                "Cancel",
                Some(Intent::CancelEdit),
                ButtonKind::Quiet,
                false,
            )),
    )
    .into()
}

fn runway_text(minutes: u32) -> String {
    let (hours, rest) = (minutes / 60, minutes % 60);
    let words = match (hours, rest) {
        (0, minutes) => plural(minutes, "minute"),
        (hours, 0) => plural(hours, "hour"),
        (hours, minutes) => format!("{} {}", plural(hours, "hour"), plural(minutes, "minute")),
    };
    format!("About {words}")
}

fn funds_row(state: FundsState) -> Node {
    let (class, words) = match state {
        FundsState::Funded => ("pill pill-ok", "Funded"),
        FundsState::Low => ("pill pill-warn", "Low"),
        FundsState::FundingRequired => ("pill pill-bad", "Funding required"),
    };
    el("div")
        .class("fact")
        .child(el("dt").text("Funding"))
        .child(el("dd").child(el("span").class(class).text(words)))
        .into()
}

fn session_funding(
    screen: &FundingScreen,
    campaign: &CampaignFunding,
    session: &SessionFunding,
) -> Node {
    let mut rows = vec![
        funds_row(session.funds),
        fact(
            "Available in the pot",
            money::balance(session.pot_available_micro_dollars),
        ),
        fact(
            "Held for pending work",
            money::balance(session.open_holds_micro_dollars),
        ),
        fact("Spent so far", money::charge(session.spent_micro_dollars)),
        fact(
            "Your share of the pot",
            money::balance(session.own_allocation_micro_dollars),
        ),
        fact(
            "Left of your share",
            money::balance(session.own_remaining_micro_dollars),
        ),
    ];
    if let Some(minutes) = session.runway_at_ceiling_minutes {
        rows.push(fact("Lasts at the pace maximum", runway_text(minutes)));
    }
    if let Some(minutes) = session.runway_at_recent_rate_minutes {
        rows.push(fact("Lasts at the recent rate", runway_text(minutes)));
    }
    rows.push(fact(
        "Contributions",
        if session.contributions_visible {
            "Other players can see what you add."
        } else {
            "Other players cannot see what you add."
        },
    ));
    let heading_id = format!("table-funding-{}", session.session_id);
    let allocating = screen
        .allocation
        .as_ref()
        .filter(|edit| edit.session_id == session.session_id);
    let action = match allocating {
        Some(edit) => allocate_form(campaign, edit),
        None => button(
            "Add credit to this table",
            Some(Intent::EditAllocation {
                session_id: session.session_id.clone(),
            }),
            ButtonKind::Primary,
            false,
        ),
    };
    el("div")
        .class("session-funding")
        .child(el("h4").attr("id", heading_id).text("This table's funding"))
        .child(facts(rows))
        .child(action)
        .into()
}

fn allocate_form(campaign: &CampaignFunding, edit: &AllocateEdit) -> Node {
    let error = edit.submit.error();
    form(Form::Allocate, &format!("Add credit to {}", campaign.name))
        .child(error_summary(error))
        .child(text_field(
            TextField {
                field: Field::AllocateAmount,
                label: "Amount to add in dollars",
                value: &edit.amount,
                kind: InputKind::Dollars,
                autocomplete: "off",
                hint: Some("This comes from your available credit."),
            },
            error,
        ))
        .child(checkbox(
            Field::AllocateConsent,
            "I agree the host may spend up to this amount at this table's pace",
            edit.consents_to_host_spending,
            error,
        ))
        .child(
            el("div")
                .class("actions")
                .child(submit_control(
                    &edit.operation,
                    "Add to the table",
                    "Adding to the table",
                    edit.submit.is_sending(),
                ))
                .child(button(
                    "Cancel",
                    Some(Intent::CancelEdit),
                    ButtonKind::Quiet,
                    false,
                )),
        )
        .into()
}
