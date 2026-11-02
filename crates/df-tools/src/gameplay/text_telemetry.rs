//! The text service's logging on the gameplay diagnostic path. The mapping from text events to
//! `generation.*` records is shared with every other composition and lives in `df-ai`.

pub(super) use df_ai::text_telemetry::{TextTelemetry, logged_service as text_service};
