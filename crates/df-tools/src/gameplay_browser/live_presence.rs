//! The phone's speech-intent signal: while a player types in the dialogue box or holds to talk,
//! the shared display shows a loader bubble on that player's hero.
//!
//! Only an ephemeral state leaves the phone (typing, speaking, submitted, stopped) through the
//! generated `DialogueService.Compose` call; the draft text and the audio never leave before
//! submit. It is not game state: the server only keeps it in memory and expires it after about
//! five seconds, so the phone sends on a change, refreshes while the intent lasts at most every
//! [`REFRESH_MS`], and sends a stop when the player submits, blurs, clears or releases. Table
//! talk ("About the game", "Plan only") shows no bubble. A failed send is recorded on the
//! player's root as `data-presence="failed"`; it never blocks typing, a submit or the session.
use super::*;
use gloo_timers::future::TimeoutFuture;
use std::cell::{Cell, RefCell as StdRefCell};

/// The most often an unchanged intent is sent again, and the keep-alive period.
const REFRESH_MS: u32 = 2_000;
/// A presence call is best effort; it gets a short deadline of its own.
const SEND_DEADLINE_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Intent {
    Typing,
    Speaking,
    Submitted,
    Stopped,
}

impl Intent {
    fn kind(self) -> rpc::ComposeKind {
        match self {
            Self::Typing => rpc::ComposeKind::Typing,
            Self::Speaking => rpc::ComposeKind::Speaking,
            Self::Submitted => rpc::ComposeKind::Submitted,
            Self::Stopped => rpc::ComposeKind::Stopped,
        }
    }
}

struct Sent {
    intent: Intent,
    at_ms: f64,
}

thread_local! {
    static LAST: StdRefCell<Option<Sent>> = const { StdRefCell::new(None) };
    /// Which keep-alive loop is current; an older one ends itself.
    static LOOP: Cell<u64> = const { Cell::new(0) };
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// Whether the dialogue context is table talk: no bubble is shown for it.
fn table_talk(client: &Client) -> bool {
    client.dialogue.context_value().is_some_and(|value| {
        value == rpc::DialogueContext::Meta as i32 || value == rpc::DialogueContext::PlanOnly as i32
    })
}

/// Tells the table what this player is doing. Sends on a change, refreshes an unchanged intent
/// at most every [`REFRESH_MS`], and sends a stop only when something was shown.
pub(super) fn signal(client: &Rc<RefCell<Client>>, requested: Intent) {
    if client.borrow().role != "player" {
        return;
    }
    let intent = if table_talk(&client.borrow()) {
        Intent::Stopped
    } else {
        requested
    };
    let now = now_ms();
    let send = LAST.with(|last| {
        let mut last = last.borrow_mut();
        let fresh = match (&*last, intent) {
            (None, Intent::Stopped) => false,
            (Some(sent), Intent::Stopped) => sent.intent != Intent::Stopped,
            (Some(sent), _) if sent.intent == intent => now - sent.at_ms >= f64::from(REFRESH_MS),
            _ => true,
        };
        if fresh {
            *last = Some(Sent { intent, at_ms: now });
        }
        fresh
    });
    if !send {
        return;
    }
    dispatch(client, intent);
    match intent {
        Intent::Typing | Intent::Speaking => keep_alive(client, intent),
        Intent::Submitted | Intent::Stopped => {
            LOOP.with(|current| current.set(current.get() + 1));
        }
    }
}

/// Whether the intent is still true: the draft is non-empty and focused, or the talk control is
/// held.
fn still(client: &Rc<RefCell<Client>>, intent: Intent) -> bool {
    let state = client.borrow();
    if table_talk(&state) {
        return false;
    }
    match intent {
        Intent::Speaking => state.dialogue.talk.is_held(),
        Intent::Typing => state.dialogue.editor().is_some_and(|editor| {
            let element: &Element = editor.as_ref();
            !editor.value().trim().is_empty()
                && state.document.active_element().as_ref() == Some(element)
        }),
        Intent::Submitted | Intent::Stopped => false,
    }
}

/// Re-sends the intent while it lasts, so the server's expiry never ends a live one, and stops
/// it when it ends. One loop at a time owns the signal.
fn keep_alive(client: &Rc<RefCell<Client>>, intent: Intent) {
    let id = LOOP.with(|current| {
        current.set(current.get() + 1);
        current.get()
    });
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            TimeoutFuture::new(REFRESH_MS).await;
            if LOOP.with(Cell::get) != id {
                return;
            }
            let Some(client) = weak.upgrade() else {
                return;
            };
            if still(&client, intent) {
                signal(&client, intent);
                // `signal` starts a loop of its own only for a change; this one carries on.
                LOOP.with(|current| current.set(id));
            } else {
                signal(&client, Intent::Stopped);
                return;
            }
        }
    });
}

fn dispatch(client: &Rc<RefCell<Client>>, intent: Intent) {
    let (channel, credential) = {
        let state = client.borrow();
        let Some(channel) = state.dialogue.channel_clone() else {
            return;
        };
        (channel, state.credential.clone())
    };
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        let mut channel = channel;
        let outcome = match request(
            rpc::ComposeRequest {
                kind: intent.kind() as i32,
            },
            &credential,
        ) {
            Ok(mut call) => {
                call.set_timeout(std::time::Duration::from_millis(SEND_DEADLINE_MS));
                channel.compose(call).await.map(|_| ())
            }
            Err(_) => return,
        };
        if let Some(client) = weak.upgrade() {
            let root = client.borrow().root.clone();
            // A refused attribute write only loses the diagnostic.
            let _marked = root.set_attribute(
                "data-presence",
                if outcome.is_ok() { "sent" } else { "failed" },
            );
        }
    });
}
