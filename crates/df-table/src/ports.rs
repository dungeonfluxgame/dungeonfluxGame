//! The seams between this crate and its host.
//!
//! A port is owned here and implemented twice: by the host for real, and by `testkit` for tests.
//! More ports (rules owner, interpreter, narrator, broker, clock, dice, knowledge view) join this
//! file as the machines and the pipeline need them.

use crate::event::TableEvent;

/// Receives every typed event the table produces.
///
/// An implementation must not block and must not fail the caller: a sink that cannot record
/// drops the event and counts the loss itself.
pub trait EventSink {
    fn record(&self, event: &TableEvent);
}
