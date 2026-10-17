//! Where the floor of a scene is for the stage's figures.
//!
//! The stage places every figure on a depth axis (near, mid, far) over the scene picture. Near
//! figures are the heroes in the foreground, knees-up and cropped by the bottom edge of the
//! screen; mid and far figures stand on the floor band of the picture with their feet shown.
//! The floor band and light of a place are declared by the server from the place's setting
//! (`GameplayScene.floor`); nothing here reads an art or file name. Presentation only: nothing
//! here touches game state.

use df_protocol::common as rpc;

/// The floor of one place and the light that falls on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Staging {
    /// Screen height, in percent from the top, where the feet of a mid-depth figure stand.
    pub(crate) feet_mid: u8,
    /// The same for a far figure: higher on the screen, toward the horizon.
    pub(crate) feet_far: u8,
    /// The key light comes from the left of the screen (`true`) or the right.
    pub(crate) key_from_left: bool,
    /// The floor is wet or polished, so figures show a faint reflection.
    pub(crate) wet: bool,
}

/// The staging when the server declares none.
pub(crate) const DEFAULT: Staging = Staging {
    feet_mid: 88,
    feet_far: 75,
    key_from_left: true,
    wet: true,
};

/// The staging the server declared for the scene, bounded to the screen; the default when the
/// view carries none.
pub(crate) fn staging_of(floor: Option<&rpc::StageFloor>) -> Staging {
    let Some(floor) = floor else {
        return DEFAULT;
    };
    let percent = |value: u32| u8::try_from(value.min(100)).unwrap_or(100);
    let feet_mid = percent(floor.feet_mid);
    Staging {
        feet_mid,
        feet_far: percent(floor.feet_far).min(feet_mid),
        key_from_left: floor.key_from_left,
        wet: floor.wet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_floor_is_used_as_sent_and_a_missing_one_is_the_default() {
        assert_eq!(staging_of(None), DEFAULT);
        let quay = staging_of(Some(&rpc::StageFloor {
            feet_mid: 90,
            feet_far: 78,
            key_from_left: false,
            wet: true,
        }));
        assert_eq!(
            (quay.feet_mid, quay.feet_far, quay.key_from_left),
            (90, 78, false)
        );
    }

    #[test]
    fn a_floor_outside_the_screen_is_bounded() {
        let bounded = staging_of(Some(&rpc::StageFloor {
            feet_mid: 400,
            feet_far: 900,
            key_from_left: true,
            wet: false,
        }));
        assert_eq!((bounded.feet_mid, bounded.feet_far), (100, 100));
        let inverted = staging_of(Some(&rpc::StageFloor {
            feet_mid: 60,
            feet_far: 80,
            key_from_left: true,
            wet: false,
        }));
        assert!(inverted.feet_far <= inverted.feet_mid);
    }
}
