//! Joining a table at `/app/join`: an invitation link (its token lives in the screen's state and
//! is never printed) or a room code that ends in the host's approval (plan 9.6; S1 8.2, 8.3).

use super::{actions, is_busy, panel};
use crate::{
    copy, money,
    node::{Node, el},
    route::Route,
    state::{Field, Form, Intent, JoinScreen, Model, Remote},
    view::{
        ButtonKind, InputKind, TextField, button, error_summary, fact, facts, failed, form, link,
        loading, page, status_line, submit_button, text_field,
    },
    wire::ApiError,
    wire_campaign::InvitePreview,
};

pub(in crate::view) fn join(screen: &JoinScreen, model: &Model) -> Node {
    let shell = page("Join a table", None);
    if let Some(requested) = &screen.requested {
        return shell
            .child(status_line(format!(
                "Request sent to the host of {}. You will see the table on your dashboard once \
                 they approve.",
                requested.campaign_name
            )))
            .child(actions(vec![link(
                "Go to your dashboard",
                Route::Dashboard,
                "btn btn-gilt",
            )]))
            .into();
    }
    let is_signed_in = model.account().is_some();
    let offset_east_minutes = model.offset_east_minutes;
    match &screen.preview {
        Some(Remote::Loading) => shell.child(loading("Checking your invitation")).into(),
        Some(Remote::Failed(error)) => shell
            .child(refusal(*error))
            .child(actions(vec![leave_link(is_signed_in)]))
            .into(),
        Some(Remote::Ready(preview)) => shell
            .child(invitation(screen, preview, model, offset_east_minutes))
            .into(),
        None if !is_signed_in => shell.child(sign_in_for_code()).into(),
        None => shell
            .child(room_code(
                screen,
                offset_east_minutes,
                is_busy(screen.submit, &screen.operation),
            ))
            .into(),
    }
}

/// A refused invitation tells its holder why and nothing about the table.
fn refusal(error: ApiError) -> Node {
    if error.reason.is_none() {
        return failed(error);
    }
    let problem = copy::problem(error);
    el("div")
        .class("problem")
        .attr("role", "alert")
        .child(el("p").class("problem-what").text(problem.what))
        .child(el("p").text(problem.fix))
        .into()
}

fn leave_link(is_signed_in: bool) -> Node {
    if is_signed_in {
        link("Go to your dashboard", Route::Dashboard, "btn btn-outline")
    } else {
        link("Go to the start", Route::Landing, "btn btn-outline")
    }
}

fn preview_panel(preview: &InvitePreview, offset_east_minutes: i32) -> Node {
    let mut rows = vec![
        fact("Table", preview.campaign_name.clone()),
        fact("Host", preview.host_name.clone()),
    ];
    if let Some(at_ms) = preview.scheduled_at_ms {
        rows.push(fact(
            "Planned for",
            money::date_time(at_ms, offset_east_minutes),
        ));
    }
    rows.push(fact(
        "Content rating",
        copy::rating_name(preview.content_rating),
    ));
    panel(&format!("You are invited to {}", preview.campaign_name))
        .child(facts(rows))
        .into()
}

fn invitation(
    screen: &JoinScreen,
    preview: &InvitePreview,
    model: &Model,
    offset_east_minutes: i32,
) -> Node {
    let error = screen.submit.error();
    let mut body = el("div")
        .class("stack")
        .child(preview_panel(preview, offset_east_minutes))
        .child(error_summary(error));
    let Some(account) = model.account() else {
        let is_sending = screen.submit.is_sending();
        return body
            .child(el("p").text("We remember this invitation while you sign in."))
            .child(actions(vec![
                button(
                    "Sign in to join",
                    Some(Intent::HoldInvitation {
                        then: Route::SignIn,
                    }),
                    ButtonKind::Primary,
                    is_sending,
                ),
                button(
                    "Create a free account to join",
                    Some(Intent::HoldInvitation {
                        then: Route::SignUp,
                    }),
                    ButtonKind::Secondary,
                    is_sending,
                ),
            ]))
            .into();
    };
    let name = account.display_name.as_ref().unwrap_or(&account.email);
    body = body
        .child(el("p").text(format!("You are signed in as {name}.")))
        .child(actions(vec![button(
            "Join this table",
            Some(Intent::AcceptInvitation),
            ButtonKind::Primary,
            is_busy(screen.submit, &screen.operation),
        )]));
    body.into()
}

fn sign_in_for_code() -> Node {
    el("div")
        .class("stack")
        .child(el("h2").text("Sign in to use a room code"))
        .child(el("p").text(
            "A room code asks the host to let you in, so the host needs to know who you are. \
             Signing in or creating an account is free.",
        ))
        .child(actions(vec![
            link("Sign in", Route::SignIn, "btn btn-gilt"),
            link("Create a free account", Route::SignUp, "btn btn-outline"),
        ]))
        .into()
}

fn room_code(screen: &JoinScreen, offset_east_minutes: i32, is_busy: bool) -> Node {
    let error = screen.submit.error();
    let Some(preview) = &screen.code_preview else {
        return form(Form::CodeLookup, "Look up a room code")
            .child(error_summary(error))
            .child(text_field(
                TextField {
                    field: Field::RoomCode,
                    label: "Room code",
                    value: &screen.code,
                    kind: InputKind::Text,
                    autocomplete: "off",
                    hint: Some("Ask the host for the code. The host approves each person."),
                },
                error,
            ))
            .child(submit_button(
                "Look up code",
                "Looking up...",
                screen.submit.is_sending(),
            ))
            .into();
    };
    el("div")
        .class("stack")
        .child(preview_panel(preview, offset_east_minutes))
        .child(error_summary(error))
        .child(el("p").text("The host approves each person before they join."))
        .child(actions(vec![
            button(
                "Ask to join",
                Some(Intent::AskToJoin),
                ButtonKind::Primary,
                is_busy,
            ),
            button(
                "Use another code",
                Some(Intent::ClearCodeLookup),
                ButtonKind::Secondary,
                screen.submit.is_sending(),
            ),
        ]))
        .into()
}
