//! Every bound and timing value of the host's text surface, in one place.

use std::time::Duration;

/// How long one question to the text service (an identity question, a persona or a line) may
/// take. Generous for dev work: a slow model or a cold Codex start still answers; the text
/// service bounds it again (`df_ai::text::TextLimits`).
pub const TEXT_DEADLINE: Duration = Duration::from_secs(600);
/// The most distinct models a runtime serves besides its default. A request names a model, so the
/// cache of routes is bounded; more is a typed refusal.
pub const MAXIMUM_MODEL_ROUTES: usize = 1_000;
/// The most output kept from a Codex child's standard output or standard error, in bytes.
pub const MAXIMUM_CODEX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// The most tokens one Responses answer may use, reasoning included (the request's
/// `max_output_tokens`).
pub const MAXIMUM_RESPONSES_OUTPUT_TOKENS: u32 = 32_000;
