use super::*;
use crate::{
    diagnostics::{self, Event},
    state::Model,
};

#[derive(Debug, Eq, PartialEq)]
enum Call {
    ReplaceState(String),
    InstallPanicHook,
    StartRouter,
}

struct Recorder {
    location: Location,
    calls: Vec<Call>,
    started: Option<Msg>,
}

impl Recorder {
    fn at(path: &str, query: &str, fragment: &str) -> Self {
        Self {
            location: Location {
                path: path.to_owned(),
                query: query.to_owned(),
                fragment: fragment.to_owned(),
            },
            calls: Vec::new(),
            started: None,
        }
    }
}

impl Host for Recorder {
    fn location(&self) -> Location {
        self.location.clone()
    }

    fn offset_east_minutes(&self) -> i32 {
        -240
    }

    fn replace_state(&mut self, path: &str) {
        self.calls.push(Call::ReplaceState(path.to_owned()));
    }

    fn install_panic_hook(&mut self) {
        self.calls.push(Call::InstallPanicHook);
    }

    fn start_router(&mut self, started: Msg) {
        self.calls.push(Call::StartRouter);
        self.started = Some(started);
    }
}

#[test]
fn the_invitation_fragment_is_removed_before_the_panic_hook_and_the_router_start() {
    let mut host = Recorder::at("/app/join", "", "token=0a1b2c3d");

    boot(&mut host);

    assert_eq!(
        host.calls,
        vec![
            Call::ReplaceState("/app/join".to_owned()),
            Call::InstallPanicHook,
            Call::StartRouter,
        ]
    );
}

#[test]
fn the_token_still_reaches_the_router_in_memory_so_the_page_can_use_it() {
    let mut host = Recorder::at("/app/join", "", "token=0a1b2c3d");

    boot(&mut host);

    assert_eq!(
        host.started,
        Some(Msg::Started {
            route: Route::Join {
                token: Some("0a1b2c3d".to_owned())
            },
            offset_east_minutes: -240,
        })
    );
}

#[test]
fn a_verification_token_in_the_query_of_an_older_link_is_removed_too() {
    let mut host = Recorder::at("/app/verify-email", "token=ab12", "");

    boot(&mut host);

    assert_eq!(
        host.calls.first(),
        Some(&Call::ReplaceState("/app/verify-email".to_owned()))
    );
}

#[test]
fn a_plain_address_is_left_alone() {
    let mut host = Recorder::at("/app/dashboard", "", "");

    boot(&mut host);

    assert_eq!(host.calls, vec![Call::InstallPanicHook, Call::StartRouter]);
}

#[test]
fn an_unknown_address_with_a_fragment_keeps_the_path_it_was_typed_with() {
    let mut host = Recorder::at("/app/nowhere", "", "token=ab");

    boot(&mut host);

    assert_eq!(
        host.calls.first(),
        Some(&Call::ReplaceState("/app/nowhere".to_owned()))
    );
}

#[test]
fn no_log_line_the_app_can_write_contains_a_token_marker() {
    let fragment_text = "token=0a1b2c3d";
    let hostile = [
        format!("failed at /app/join#{fragment_text}"),
        format!("panicked at https://example.test/app/reset-password?x=1&{fragment_text}&y=2"),
        "TOKEN=ABCDEF and Token=abcdef".to_owned(),
        "encoded token%3Dabcdef".to_owned(),
    ];

    for message in &hostile {
        let line = diagnostics::line(Event::Panicked { message });
        assert!(!line.to_ascii_lowercase().contains("token="), "{line}");
        assert!(!line.to_ascii_lowercase().contains("token%3d"), "{line}");
    }
    for line in [
        diagnostics::line(Event::Booted {
            route: Route::Join {
                token: Some("a".to_owned()),
            }
            .name(),
        }),
        diagnostics::line(Event::RequestFailed {
            method: crate::api::Method::Post,
            status: 500,
        }),
        diagnostics::line(Event::BrowserRefused { what: "clipboard" }),
    ] {
        assert!(!line.contains("token="), "{line}");
    }
}

#[test]
fn the_model_never_formats_a_token_into_a_log_line_when_a_page_starts_with_one() {
    let mut model = Model::default();
    let route = Route::Join {
        token: Some("0a1b2c3d".to_owned()),
    };

    let effects = model.update(Msg::Started {
        route: route.clone(),
        offset_east_minutes: 0,
    });

    // The start asks for the session and nothing else; no effect carries the address.
    assert_eq!(effects.len(), 1);
    assert!(
        !diagnostics::line(Event::Booted {
            route: route.name()
        })
        .contains("0a1b2c3d")
    );
}
