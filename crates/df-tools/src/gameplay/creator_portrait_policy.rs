//! Operation idempotence for explicit portrait requests. No development spend quota.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Admission {
    Replay,
    New,
    Conflict,
    Unknown,
}
pub(super) fn operation_valid(bytes: Option<&[u8]>) -> bool {
    bytes.is_some_and(|bytes| bytes.len() == 16 && bytes.iter().any(|byte| *byte != 0))
}
pub(super) fn admit(existing: Option<&[u8]>, proposed: &[u8], dispatch: bool) -> Admission {
    match existing {
        Some(existing) if existing == proposed => Admission::Replay,
        Some(_) => Admission::Conflict,
        None if !dispatch => Admission::Unknown,
        None => Admission::New,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_operations_are_exact_nonzero_128_bits() {
        assert!(!operation_valid(None));
        assert!(!operation_valid(Some(&[0; 16])));
        assert!(!operation_valid(Some(&[1; 15])));
        assert!(!operation_valid(Some(&[1; 17])));
        assert!(operation_valid(Some(&[1; 16])));
    }
    #[test]
    fn duplicate_poll_and_conflict_never_dispatch() {
        assert_eq!(admit(Some(b"first"), b"first", true), Admission::Replay);
        assert_eq!(admit(Some(b"first"), b"first", false), Admission::Replay);
        assert_eq!(admit(Some(b"first"), b"changed", true), Admission::Conflict);
        assert_eq!(admit(None, b"unknown", false), Admission::Unknown);
    }
    #[test]
    fn fresh_explicit_regenerations_have_no_session_quota() {
        for _ in 0..10000 {
            assert_eq!(admit(None, b"fresh", true), Admission::New);
        }
    }
}

/// Empty committed/uncertain metadata is meaningful: later art must not change an old input.
pub(super) fn frozen_metadata<T>(uncertain: Option<T>, committed: Option<T>, fresh: T) -> T {
    uncertain.or(committed).unwrap_or(fresh)
}
#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn uncertain_or_committed_metadata_wins_even_when_empty_or_art_changes() {
        for fresh in [vec![], vec!["later-ready"], vec!["different-image"]] {
            assert_eq!(
                frozen_metadata(Some(vec!["original"]), None, fresh.clone()),
                vec!["original"]
            );
            assert!(frozen_metadata(Some(Vec::<&str>::new()), None, fresh.clone()).is_empty());
            assert_eq!(
                frozen_metadata(None, Some(vec!["committed"]), fresh.clone()),
                vec!["committed"]
            );
            assert!(frozen_metadata(None, Some(Vec::<&str>::new()), fresh).is_empty());
        }
    }
}
