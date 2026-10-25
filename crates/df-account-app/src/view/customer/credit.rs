//! The customer add-credit and payment-return screens.

use super::submit_control;
use crate::view::{
    ButtonKind, InputKind, TextField, button, error_summary, failed, form, link, page, remote,
    text_field,
};
use crate::{
    copy, money,
    node::{Element, Node, el},
    route::Route,
    state::{AmountChoice, CreditForm, CreditReturn, Field, Form, Intent, Remote},
    wire::{CheckoutStatus, CheckoutView, CreditOffers},
};

pub(in crate::view) fn add_credit(screen: &CreditForm) -> Node {
    page(
        "Add credit",
        Some(
            "Credit pays for table time, narration and pictures when you play. You pay on a secure payment page, then come back here.",
        ),
    )
    .child(remote(&screen.offers, "Loading amounts", |offers| {
        credit_form(screen, offers)
    }))
    .into()
}

/// The cents the person has chosen, when the choice is a usable amount.
fn chosen_cents(screen: &CreditForm, offers: &CreditOffers) -> Option<u32> {
    match screen.choice? {
        AmountChoice::Preset(cents) => Some(cents),
        AmountChoice::Custom => money::parse_dollars(&screen.custom).ok().filter(|cents| {
            (offers.custom_minimum_cents..=offers.custom_maximum_cents).contains(cents)
        }),
    }
}

fn credit_form(screen: &CreditForm, offers: &CreditOffers) -> Node {
    let chosen = chosen_cents(screen, offers);
    let range = format!(
        "Between {} and {}.",
        money::price(offers.custom_minimum_cents),
        money::price(offers.custom_maximum_cents)
    );
    let is_custom = screen.choice == Some(AmountChoice::Custom);
    let mut body = el("div").class("credit-form");
    if offers.is_test_mode {
        body = body.child(
            el("p").child(
                el("span")
                    .class("pill pill-warn")
                    .text("Test mode: no real card is charged"),
            ),
        );
    }
    let mut amounts = el("fieldset")
        .class("amounts")
        .child(el("legend").text("Choose an amount"));
    for cents in &offers.preset_amounts_cents {
        amounts = amounts.child(amount_button(
            *cents,
            screen.choice == Some(AmountChoice::Preset(*cents)),
        ));
    }
    let mut custom = el("div")
        .class(if is_custom {
            "custom-amount is-chosen"
        } else {
            "custom-amount"
        })
        .child(text_field(
            TextField {
                field: Field::CustomAmount,
                label: "Other amount in dollars",
                value: &screen.custom,
                kind: InputKind::Dollars,
                autocomplete: "off",
                hint: Some(&range),
            },
            screen.submit.error(),
        ));
    if is_custom {
        custom = custom.child(el("p").class("amount-state").text("Selected"));
    }
    let receive = match (chosen, screen.choice) {
        (Some(cents), _) => format!(
            "You receive {} of DungeonFlux service credit",
            money::cents(u64::from(cents))
        ),
        (None, Some(AmountChoice::Custom)) => format!(
            "Enter an amount from {} to {} to see the credit you receive.",
            money::price(offers.custom_minimum_cents),
            money::price(offers.custom_maximum_cents)
        ),
        (None, _) => "Choose an amount to see the credit you receive.".to_owned(),
    };
    body.child(
        form(Form::Credit, "Add credit")
            .child(error_summary(screen.submit.error()))
            .child(amounts)
            .child(custom)
            .child(
                el("p")
                    .class("receive")
                    .attr("role", "status")
                    .attr("aria-live", "polite")
                    .text(receive),
            )
            // TODO(B): the exact tax and total come with the processor port; until then the
            // checkout page shows them.
            .child(el("p").class("fine").text("Tax, if any, is shown at checkout."))
            .child(el("p").class("fine").text(
                "Credit is for your own use and cannot be transferred to another person. Eligible unused credit can be refunded to the card you paid with.",
            ))
            .child(submit_control(
                &screen.operation,
                "Continue to payment",
                "Opening the payment page",
                screen.submit.is_sending(),
            )),
    )
    .into()
}

fn amount_button(cents: u32, is_selected: bool) -> Node {
    let mut control = el("button")
        .class(if is_selected {
            "btn btn-outline amount is-selected"
        } else {
            "btn btn-outline amount"
        })
        .attr("type", "button")
        .attr("aria-pressed", if is_selected { "true" } else { "false" })
        .on_click(Intent::ChooseAmount(cents))
        .text(money::price(cents));
    if is_selected {
        control = control.child(el("span").class("amount-state").text("Selected"));
    }
    control.into()
}

// ------------------------------------------------------------------------------ payment return

pub(in crate::view) fn credit_return(screen: &CreditReturn) -> Node {
    let body = if screen.checkout_id.is_none() {
        missing_payment()
    } else {
        match &screen.status {
            Remote::Failed(error) => failed(*error),
            Remote::Loading => {
                payment_live(el("p").class("loading").text("Checking your payment..."))
            }
            Remote::Ready(view) => payment_live(payment_result(view, screen.has_stopped_waiting())),
        }
    };
    page("Payment status", None).child(body).into()
}

/// The persistent polite live region: a change of payment state is announced, not shouted.
fn payment_live(content: Element) -> Node {
    el("div")
        .class("payment-status")
        .attr("role", "status")
        .attr("aria-live", "polite")
        .child(content)
        .into()
}

fn missing_payment() -> Node {
    el("div")
        .class("payment-status")
        .child(
            el("p").text(
                "We could not find a payment to show. No payment was started from this link.",
            ),
        )
        .child(
            el("p")
                .class("actions")
                .child(link("Add credit", Route::AddCredit, "btn btn-gilt"))
                .child(link(
                    "Go to your dashboard",
                    Route::Dashboard,
                    "btn btn-outline",
                )),
        )
        .into()
}

fn payment_result(view: &CheckoutView, has_stopped_waiting: bool) -> Element {
    let sentences = copy::checkout_status(view.status);
    let (pill_class, pill_words) = match view.status {
        CheckoutStatus::Pending => ("pill pill-wait", "Waiting for confirmation"),
        CheckoutStatus::Paid => ("pill pill-ok", "Paid"),
        CheckoutStatus::Failed => ("pill pill-bad", "Not paid"),
        CheckoutStatus::Cancelled => ("pill pill-warn", "Cancelled"),
        CheckoutStatus::Expired => ("pill pill-warn", "Timed out"),
    };
    let mut result = el("div")
        .class("payment-result")
        .child(el("p").child(el("span").class(pill_class).text(pill_words)))
        .child(el("p").class("payment-what").text(sentences.what));
    if !sentences.fix.is_empty() {
        result = result.child(el("p").text(sentences.fix));
    }
    result = result.child(el("p").class("fine").text(format!(
        "Credit amount: {}",
        money::cents(u64::from(view.amount_cents))
    )));
    let mut actions = el("p").class("actions");
    match view.status {
        CheckoutStatus::Pending => {
            if has_stopped_waiting {
                result = result.child(el("p").text(
                    "This is taking longer than usual. Nothing is lost: the credit appears once, as soon as the payment is confirmed.",
                ));
                actions = actions.child(button(
                    "Check again",
                    Some(Intent::Retry),
                    ButtonKind::Primary,
                    false,
                ));
            }
            actions = actions.child(link(
                "Go to your dashboard",
                Route::Dashboard,
                "btn btn-outline",
            ));
        }
        CheckoutStatus::Paid => {
            actions = actions
                .child(link(
                    "Go to your dashboard",
                    Route::Dashboard,
                    "btn btn-gilt",
                ))
                .child(link("Add more credit", Route::AddCredit, "btn btn-outline"));
        }
        CheckoutStatus::Failed | CheckoutStatus::Cancelled | CheckoutStatus::Expired => {
            actions = actions
                .child(link("Try again", Route::AddCredit, "btn btn-gilt"))
                .child(link(
                    "Go to your dashboard",
                    Route::Dashboard,
                    "btn btn-outline",
                ));
        }
    }
    result.child(actions)
}
