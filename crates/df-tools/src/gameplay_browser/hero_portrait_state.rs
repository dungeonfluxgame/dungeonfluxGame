//! View policy for the creator's pending image, independently of provider completion.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Visibility {
    pub(super) shadow: bool,
    pub(super) image: bool,
    pub(super) animate: bool,
}
pub(super) fn visibility(pending: bool, has_previous: bool) -> Visibility {
    Visibility {
        shadow: pending || !has_previous,
        image: !pending && has_previous,
        animate: pending,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generation_animates_shadow_and_retains_previous_image_for_restore() {
        assert_eq!(
            visibility(true, false),
            Visibility {
                shadow: true,
                image: false,
                animate: true
            }
        );
        assert_eq!(
            visibility(true, true),
            Visibility {
                shadow: true,
                image: false,
                animate: true
            }
        );
        assert_eq!(
            visibility(false, true),
            Visibility {
                shadow: false,
                image: true,
                animate: false
            }
        );
        assert_eq!(
            visibility(false, false),
            Visibility {
                shadow: true,
                image: false,
                animate: false
            }
        );
    }
}
