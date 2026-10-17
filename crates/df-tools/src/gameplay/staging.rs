//! Where the floor of a place is for the stage's figures, declared from the place's own
//! setting kind (content), never read from an art file name. The display draws what this says.

use df_protocol::common as rpc;

use super::adventure::setting::{SceneSetting, SettingKind};

/// The default floor: figures stand low in the frame, lit from the left on a damp floor.
const DEFAULT: (u32, u32, bool, bool) = (88, 75, true, true);

/// The stage floor of a setting kind: `(feet_mid, feet_far, key_from_left, wet)`. A quay is lit
/// from the right over wet planks; trails, passes and farmyards are dry and lit from the left.
fn floor_of(kind: SettingKind) -> (u32, u32, bool, bool) {
    match kind {
        SettingKind::Quay => (90, 78, false, true),
        SettingKind::ForestTrail | SettingKind::MountainPass => (90, 76, true, false),
        SettingKind::Farmyard => (90, 77, true, false),
        _ => DEFAULT,
    }
}

/// Whether the scene is indoors and its stage floor. A scene without a setting (the authored
/// harbor story) is the open quay.
pub(super) fn declared(setting: Option<&SceneSetting>) -> (bool, rpc::StageFloor) {
    let (interior, kind) = match setting {
        Some(setting) => (!setting.exterior, setting.kind),
        None => (false, SettingKind::Quay),
    };
    let (feet_mid, feet_far, key_from_left, wet) = floor_of(kind);
    (
        interior,
        rpc::StageFloor {
            feet_mid,
            feet_far,
            key_from_left,
            wet,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(kind: SettingKind, exterior: bool) -> SceneSetting {
        SceneSetting {
            place_id: "p".to_owned(),
            name: "P".to_owned(),
            short: "p".to_owned(),
            kind,
            exterior,
            art_key: "k".to_owned(),
            model: None,
        }
    }

    #[test]
    fn the_floor_follows_the_declared_setting_kind_and_never_a_name() {
        let (interior, quay) = declared(Some(&setting(SettingKind::Quay, true)));
        assert!(!interior);
        assert_eq!(
            (quay.feet_mid, quay.feet_far, quay.key_from_left, quay.wet),
            (90, 78, false, true)
        );
        let (interior, shop) = declared(Some(&setting(SettingKind::Shop, false)));
        assert!(interior);
        assert_eq!((shop.feet_mid, shop.feet_far), (88, 75));
        let (_, trail) = declared(Some(&setting(SettingKind::ForestTrail, true)));
        assert!(!trail.wet && trail.key_from_left);
        for kind in [SettingKind::Quay, SettingKind::Shop, SettingKind::Farmyard] {
            let (_, floor) = declared(Some(&setting(kind, true)));
            assert!(floor.feet_far < floor.feet_mid && floor.feet_mid <= 100);
        }
    }

    #[test]
    fn a_scene_with_no_setting_is_the_open_quay() {
        let (interior, floor) = declared(None);
        assert!(!interior);
        assert_eq!(floor.feet_mid, 90);
    }
}
