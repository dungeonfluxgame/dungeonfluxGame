//! The TV's pairing screen (`/app/display`; S1 8.5): ask for a code, show it, poll its status, and
//! redeem it once the host approved it in the lobby. The screen holds no secret: the device secret
//! and the ticket live in `HttpOnly` cookies the browser sends by itself.

use super::{Effect, FocusTarget, Intent, Model, Screen};
use crate::{
    api::{ApiCall, Reply},
    route::is_identifier,
    wire::{ApiError, ErrorCode},
    wire_campaign::DisplayState,
};

/// How many times a redemption is repeated when its answer never arrived (S1 8.5 step 4).
pub const DISPLAY_REDEEM_ATTEMPTS: u8 = 3;
/// How often the status is read until the challenge says otherwise.
const FALLBACK_POLL_MS: u32 = 2_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayEnd {
    Expired,
    Cancelled,
    Refused(ApiError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayPhase {
    /// Nothing has been asked for yet.
    Idle,
    /// Reading a fresh pre-session token, then asking for a code.
    Requesting,
    Showing {
        user_code: String,
        expires_at_ms: u64,
    },
    Redeeming {
        attempts: u8,
    },
    /// Redeemed: the browser is going to the runtime's display page.
    Opening,
    Ended(DisplayEnd),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayScreen {
    pub phase: DisplayPhase,
    pub poll_ms: u32,
}

impl Model {
    pub(super) fn enter_display(&mut self) -> Vec<Effect> {
        self.screen = Screen::Display(DisplayScreen {
            phase: DisplayPhase::Idle,
            poll_ms: FALLBACK_POLL_MS,
        });
        Vec::new()
    }

    pub(super) fn display_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let Screen::Display(screen) = &mut self.screen else {
            return Vec::new();
        };
        match (intent, &screen.phase) {
            (Intent::ShowPairingCode, DisplayPhase::Idle | DisplayPhase::Ended(_)) => {
                screen.phase = DisplayPhase::Requesting;
                // A TV that sat idle has an expired pre-session token: read a fresh one first.
                vec![self.api(ApiCall::Session)]
            }
            _ => Vec::new(),
        }
    }

    /// The session was read again while a code was being requested.
    pub(super) fn display_session_refreshed(&mut self) -> Option<Vec<Effect>> {
        let Screen::Display(screen) = &self.screen else {
            return None;
        };
        (screen.phase == DisplayPhase::Requesting)
            .then(|| vec![self.api(ApiCall::DisplayChallenge)])
    }

    pub(super) fn display_session_failed(&mut self, error: ApiError) -> bool {
        let Screen::Display(screen) = &mut self.screen else {
            return false;
        };
        if screen.phase != DisplayPhase::Requesting {
            return false;
        }
        screen.phase = DisplayPhase::Ended(DisplayEnd::Refused(error));
        true
    }

    pub(super) fn display_timer(&self) -> Vec<Effect> {
        match &self.screen {
            Screen::Display(DisplayScreen {
                phase: DisplayPhase::Showing { .. },
                ..
            }) => vec![self.api(ApiCall::DisplayStatus)],
            _ => Vec::new(),
        }
    }

    pub(super) fn display_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Display(screen) = &mut self.screen else {
            return Vec::new();
        };
        match (reply, &screen.phase) {
            (Reply::DisplayChallenge(Ok(challenge)), DisplayPhase::Requesting) => {
                screen.poll_ms = challenge.poll_seconds.saturating_mul(1_000).max(1_000);
                screen.phase = DisplayPhase::Showing {
                    user_code: challenge.user_code,
                    expires_at_ms: challenge.expires_at_ms,
                };
                vec![
                    Effect::After {
                        generation,
                        delay_ms: screen.poll_ms,
                    },
                    Effect::Focus(FocusTarget::Heading),
                ]
            }
            (Reply::DisplayChallenge(Err(error)), DisplayPhase::Requesting) => {
                screen.phase = DisplayPhase::Ended(DisplayEnd::Refused(error));
                vec![Effect::Focus(FocusTarget::ErrorSummary)]
            }
            (Reply::DisplayStatus(Ok(status)), DisplayPhase::Showing { .. }) => {
                match status.state {
                    DisplayState::Pending => vec![Effect::After {
                        generation,
                        delay_ms: screen.poll_ms,
                    }],
                    DisplayState::Approved => {
                        screen.phase = DisplayPhase::Redeeming { attempts: 1 };
                        vec![Effect::Api {
                            generation,
                            call: ApiCall::DisplayRedeem,
                        }]
                    }
                    DisplayState::Expired => end(screen, DisplayEnd::Expired),
                    DisplayState::Cancelled => end(screen, DisplayEnd::Cancelled),
                }
            }
            (Reply::DisplayStatus(Err(error)), DisplayPhase::Showing { .. }) => {
                if matches!(
                    error.code,
                    ErrorCode::Unavailable | ErrorCode::Busy | ErrorCode::Throttled
                ) {
                    vec![Effect::After {
                        generation,
                        delay_ms: screen.poll_ms,
                    }]
                } else {
                    end(screen, DisplayEnd::Refused(error))
                }
            }
            (Reply::DisplayRedeemed(Ok(redeemed)), DisplayPhase::Redeeming { .. }) => {
                if !is_identifier(&redeemed.session_id) {
                    return end(
                        screen,
                        DisplayEnd::Refused(ApiError::of(ErrorCode::Unavailable)),
                    );
                }
                screen.phase = DisplayPhase::Opening;
                vec![Effect::Leave(format!(
                    "/sessions/{}/display",
                    redeemed.session_id
                ))]
            }
            (Reply::DisplayRedeemed(Err(error)), DisplayPhase::Redeeming { attempts }) => {
                if error.code == ErrorCode::UnknownOutcome && *attempts < DISPLAY_REDEEM_ATTEMPTS {
                    screen.phase = DisplayPhase::Redeeming {
                        attempts: attempts + 1,
                    };
                    return vec![Effect::Api {
                        generation,
                        call: ApiCall::DisplayRedeem,
                    }];
                }
                end(screen, DisplayEnd::Refused(error))
            }
            _ => Vec::new(),
        }
    }
}

fn end(screen: &mut DisplayScreen, reason: DisplayEnd) -> Vec<Effect> {
    screen.phase = DisplayPhase::Ended(reason);
    vec![Effect::Focus(FocusTarget::Heading)]
}
