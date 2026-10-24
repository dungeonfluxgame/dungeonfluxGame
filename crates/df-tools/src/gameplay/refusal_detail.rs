//! Which step refused the command currently being processed, so the `REFUSAL` and
//! `UNAVAILABLE` log lines can name it instead of only the shared error kind.
//!
//! Scoping: the session owner (`actor::Actor` as a `Reducer`) runs on one dedicated
//! thread and processes one call at a time, start to finish, on that thread. The detail
//! therefore lives in thread-local storage for that thread only. It is cleared at the
//! start of every call the owner reduces (`take` and discard in `Actor::reduce`), and
//! printing consumes it (`take` in the owner's `REFUSAL`/`UNAVAILABLE` recorders), so a
//! detail noted by one command can never be printed for another: a later command either
//! starts from the cleared slot or the earlier print already emptied it. Code running on
//! any other thread has its own empty slot and neither sees nor disturbs the owner's.
//!
//! The first `note` of a command wins: the innermost step that refused is the specific
//! one, and outer wrappers noting their own stage must not overwrite it. Steps are a
//! closed vocabulary of short kebab-case `&'static str` literals chosen at each call
//! site; player text and other runtime strings are never accepted here.

use std::cell::Cell;

thread_local! {
    static REFUSAL_STEP: Cell<Option<&'static str>> = const { Cell::new(None) };
    static REFUSAL_RECORDED: Cell<bool> = const { Cell::new(false) };
}

/// Each submitted action gets one terminal refusal record, including pre-repository failures.
pub(super) fn begin_action() {
    REFUSAL_RECORDED.with(|slot| slot.set(false));
    let _step = take();
    let _cause = df_session::submission::take_refusal_diagnostic();
}
pub(super) fn recorded() -> bool {
    REFUSAL_RECORDED.with(Cell::get)
}
pub(super) fn mark_recorded() {
    REFUSAL_RECORDED.with(|slot| slot.set(true));
}

/// Records `step` as the refusing step for the command being processed on this thread,
/// unless an earlier (more specific) step was already noted for it.
pub(super) fn note(step: &'static str) {
    REFUSAL_STEP.with(|slot| {
        if slot.get().is_none() {
            slot.set(Some(step));
        }
    });
}

/// Attach a typed safe cause to the existing step, without replacing a deeper diagnostic.
pub(super) fn cause(step: &'static str, cause: String) {
    note(step);
    let step = REFUSAL_STEP.with(Cell::get).unwrap_or(step);
    df_session::submission::note_refusal_diagnostic(step, cause);
}

/// Returns the noted step for the current command and clears the slot, so the detail is
/// printed at most once. Returns `None` when nothing was noted.
pub(super) fn take() -> Option<&'static str> {
    REFUSAL_STEP.with(|slot| slot.take())
}

/// The step as printed after `step=` on a refusal line: the noted step, or `unknown`
/// when the refusing site noted nothing. Consumes the detail like `take`.
pub(super) fn step_or_unknown() -> &'static str {
    take().unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::{note, step_or_unknown, take};

    #[test]
    fn noted_detail_is_printed_once_then_cleared() {
        let _discarded = take();
        note("combat-admission");
        assert_eq!(step_or_unknown(), "combat-admission");
        assert_eq!(take(), None, "the detail must be printed once, then gone");
        assert_eq!(step_or_unknown(), "unknown");
    }

    #[test]
    fn unnoted_refusal_prints_unknown() {
        let _discarded = take();
        assert_eq!(step_or_unknown(), "unknown");
    }

    #[test]
    fn first_note_wins_over_outer_wrappers() {
        let _discarded = take();
        note("combat-admission");
        note("stage-using");
        assert_eq!(take(), Some("combat-admission"));
    }

    #[test]
    fn detail_does_not_cross_threads() {
        let _discarded = take();
        note("combat-admission");
        let other = std::thread::spawn(take).join().expect("thread panicked");
        assert_eq!(other, None);
        assert_eq!(take(), Some("combat-admission"));
    }
}
