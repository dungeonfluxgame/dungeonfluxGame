#![cfg(target_arch = "wasm32")]
//! The browser half of the machine round trip (plan O.16 W1): a Rust/WASM client that opens one
//! binary WebSocket to the bridge, watches a machine and submits one tap and one spoken line
//! through the generated `SessionServiceClient` and `ActionServiceClient`, and writes each step
//! into the page. It is the same exchange as `tests/machine_round_trip.rs`, over the browser's
//! `WebSocket` and `BrowserChannel` instead of a native client.
//!
//! To run it, from the repository root:
//!
//! ```text
//! cargo build -p df-rpc-bridge --example machine_browser --target wasm32-unknown-unknown
//! wasm-bindgen --target web --no-typescript --out-dir <directory> \
//!     <target>/wasm32-unknown-unknown/debug/examples/machine_browser.wasm
//! cargo run -p df-rpc-bridge --example machine_bridge_server -- <directory>
//! ```
//!
//! then open the address the server prints. The page shows one line per step and sets
//! `data-result` on its root element to `pass` or `fail`, and asks the server for `/report/pass`
//! or `/report/fail`, which the server prints (`development/machine-browser.sh` reads that line).

use df_api::machine::{check_machine_view, sample};
use df_protocol::common as wire;
use df_protocol::common::{
    action_service_client::ActionServiceClient, session_service_client::SessionServiceClient,
};
use df_rpc_bridge::{BrowserConnection, RPC_MESSAGE_BYTES};
use wasm_bindgen::prelude::*;

fn document() -> Result<web_sys::Document, String> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "the page has no document".to_owned())
}

fn show(lines: &[String]) {
    if let Some(element) = document()
        .ok()
        .and_then(|document| document.get_element_by_id("results"))
    {
        element.set_text_content(Some(&lines.join("\n")));
    }
}

fn finish(outcome: &Result<Vec<String>, (Vec<String>, String)>) {
    let (mut lines, result) = match outcome {
        Ok(lines) => (lines.clone(), "pass"),
        Err((lines, message)) => {
            let mut lines = lines.clone();
            lines.push(format!("FAIL: {message}"));
            (lines, "fail")
        }
    };
    if result == "pass" {
        lines.push("PASS: one view and one intent crossed the bridge".to_owned());
    }
    show(&lines);
    // Tell the server the verdict, so a script without a browser console can read it.
    if let Some(window) = web_sys::window() {
        drop(window.fetch_with_str(&format!("/report/{result}")));
    }
    if let Some(root) = document()
        .ok()
        .and_then(|document| document.document_element())
    {
        drop(root.set_attribute("data-result", result));
    }
}

fn note(lines: &mut Vec<String>, text: &str) {
    lines.push(text.to_owned());
    show(lines);
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn tunnel_url() -> Result<String, String> {
    let host = web_sys::window()
        .ok_or("no window")?
        .location()
        .host()
        .map_err(|_| "the page has no host")?;
    Ok(format!("ws://{host}/tunnel"))
}

fn watch_request() -> wire::WatchViewRequest {
    wire::WatchViewRequest {
        session_id: Some(wire::SessionId {
            value: Some(sample::SESSION_BYTES.to_vec()),
        }),
        run_id: Some(wire::RunId {
            value: Some(sample::RUN_BYTES.to_vec()),
        }),
        client_binding_id: None,
        after_revision: None,
    }
}

fn declined(response: wire::SubmitMachineIntentResponse) -> Result<wire::MachineReceipt, String> {
    match response.outcome {
        Some(wire::submit_machine_intent_response::Outcome::Receipt(receipt)) => Ok(receipt),
        _ => Err("the answer is not a receipt".to_owned()),
    }
}

async fn run(lines: &mut Vec<String>) -> Result<(), String> {
    note(lines, "CONNECTING: binary WebSocket carrying native HTTP/2");
    let (connection, channel) = BrowserConnection::connect(&tunnel_url()?)
        .await
        .map_err(|error| error.to_string())?;
    let mut sessions = SessionServiceClient::new(channel.clone())
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES);
    let mut actions = ActionServiceClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES);

    note(lines, "STEP 1: WatchMachine (server stream), first view");
    let mut views = sessions
        .watch_machine(watch_request())
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    let first = views
        .message()
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the stream ended before the first view")?;
    check_machine_view(&first).map_err(|error| format!("first view refused: {error:?}"))?;
    require(
        first == sample::seat_view(),
        "the first view is not the sample",
    )?;
    note(
        lines,
        "PASS: first view equals the sample and passes the guard",
    );

    note(
        lines,
        "STEP 2: SubmitMachineIntent (unary), tap on the offer",
    );
    let receipt = declined(
        actions
            .submit_machine_intent(sample::tap_request())
            .await
            .map_err(|error| error.to_string())?
            .into_inner(),
    )?;
    require(
        receipt.result
            == Some(wire::machine_receipt::Result::Applied(
                wire::MachineApplied {},
            )),
        "the tap was not applied",
    )?;
    note(lines, "PASS: tap applied");

    note(lines, "STEP 3: the next view arrives on the same stream");
    let next = views
        .message()
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the stream ended before the next view")?;
    check_machine_view(&next).map_err(|error| format!("next view refused: {error:?}"))?;
    let sequence = next.revision.and_then(|revision| revision.sequence);
    require(
        sequence == Some(sample::SEQUENCE + 1),
        "the next view is not one revision on",
    )?;
    require(
        matches!(&next.audience, Some(wire::machine_view::Audience::Seat(seat))
            if seat.narration.last().map(String::as_str) == Some(sample::TAP_NARRATION)),
        "the next view lacks the new narration line",
    )?;
    require(
        next.revision == receipt.revision,
        "the receipt and the view disagree on the revision",
    )?;
    note(lines, "PASS: revision 43 with the new line");

    note(
        lines,
        "STEP 4: a spoken line is declined by id, not by words",
    );
    let spoken = declined(
        actions
            .submit_machine_intent(sample::say_request())
            .await
            .map_err(|error| error.to_string())?
            .into_inner(),
    )?;
    require(
        spoken.utterance_id == Some(sample::UTTERANCE_ID),
        "the decline does not name the line",
    )?;
    let Some(wire::machine_receipt::Result::Declined(decline)) = spoken.result else {
        return Err("the line was not declined".to_owned());
    };
    require(
        wire::RejectionCode::try_from(decline.code) == Ok(wire::RejectionCode::WrongTurn),
        "the decline is not WrongTurn",
    )?;
    note(lines, "PASS: declined WrongTurn, utterance 7");

    note(lines, "STEP 5: a malformed intent is a call failure");
    let mut malformed = sample::tap_request();
    if let Some(intent) = malformed.intent.as_mut() {
        intent.machine_id.clear();
    }
    let status = actions
        .submit_machine_intent(malformed)
        .await
        .err()
        .ok_or("a blank machine id was accepted")?;
    require(
        status.code() == tonic::Code::InvalidArgument,
        "the refusal is not InvalidArgument",
    )?;
    note(lines, "PASS: InvalidArgument status, no receipt");

    drop(views);
    connection.close();
    Ok(())
}

#[wasm_bindgen(start)]
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        let mut lines = Vec::new();
        let outcome = match run(&mut lines).await {
            Ok(()) => Ok(lines),
            Err(message) => Err((lines, message)),
        };
        finish(&outcome);
    });
}
