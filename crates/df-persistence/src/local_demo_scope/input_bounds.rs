//! Bounded canonical expansion of a validated public character description.
pub(super) const CREATION_BYTES: usize = 32 * 1024;
pub(super) const ORDINARY_BYTES: usize = 8192;

pub(super) const fn maximum(creation: bool) -> usize {
    if creation {
        CREATION_BYTES
    } else {
        ORDINARY_BYTES
    }
}

pub(super) fn admits(creation: bool, bytes: Option<usize>) -> bool {
    bytes.is_some_and(|bytes| bytes <= maximum(creation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_expansion_does_not_raise_ordinary_input_limits() {
        assert!(admits(false, Some(8192)));
        assert!(!admits(false, Some(8193)));
        assert!(admits(true, Some(32768)));
        assert!(!admits(true, Some(32769)));
        assert!(!admits(true, None));
        assert!(!admits(false, None));
    }
}
