//! Where the floor of a scene painting is, and which art a figure uses at which depth.
//!
//! The stage places every figure on a depth axis (near, mid, far) over the painting. Near
//! figures are the heroes in the foreground, knees-up and cropped by the bottom edge of the
//! screen; mid and far figures stand on the floor band of the painting with their feet shown, so
//! they use the full-body art where it exists. The floor band differs a little per painting:
//! [`staging_for`] holds a default and a small table of overrides to extend when a painting's
//! floor sits higher or lower. Presentation only: nothing here touches game state.

/// The floor of one painting and the light that falls on it.
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

const DEFAULT: Staging = Staging {
    feet_mid: 88,
    feet_far: 75,
    key_from_left: true,
    wet: true,
};

/// Overrides by a word of the painting's file name. The first match wins. Extend this table
/// when a painting needs its floor moved; a painting with no usable floor stays on the default.
const OVERRIDES: [(&str, Staging); 6] = [
    (
        "quay",
        Staging {
            feet_mid: 90,
            feet_far: 78,
            key_from_left: false,
            wet: true,
        },
    ),
    (
        "wharf",
        Staging {
            feet_mid: 90,
            feet_far: 78,
            key_from_left: false,
            wet: true,
        },
    ),
    (
        "trail",
        Staging {
            feet_mid: 90,
            feet_far: 76,
            key_from_left: true,
            wet: false,
        },
    ),
    (
        "pass",
        Staging {
            feet_mid: 90,
            feet_far: 76,
            key_from_left: true,
            wet: false,
        },
    ),
    (
        "farm",
        Staging {
            feet_mid: 90,
            feet_far: 77,
            key_from_left: true,
            wet: false,
        },
    ),
    (
        "courier-road",
        Staging {
            feet_mid: 90,
            feet_far: 76,
            key_from_left: false,
            wet: true,
        },
    ),
];

/// The staging for a scene painting named by its served path.
pub(crate) fn staging_for(scene_asset: &str) -> Staging {
    let name = scene_asset.rsplit('/').next().unwrap_or(scene_asset);
    OVERRIDES
        .iter()
        .find(|(word, _)| name.contains(word))
        .map_or(DEFAULT, |(_, staging)| *staging)
}

/// The full-body twin of a character cut-out stem, for the characters who have one: the
/// heroes, every antagonist of the roster and the bandit. Patrons, contacts and villains have
/// the knees-up cut-out only.
pub(crate) fn full_body_stem(sprite_stem: &str) -> Option<String> {
    let name = sprite_stem.strip_prefix("sprite-")?;
    let has_full_body = name.starts_with("hero-") || name.starts_with("ant-") || name == "bandit";
    has_full_body.then(|| format!("figure-{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heroes_antagonists_and_the_bandit_have_a_full_body_twin() {
        assert_eq!(
            full_body_stem("sprite-hero-2").as_deref(),
            Some("figure-hero-2")
        );
        assert_eq!(
            full_body_stem("sprite-ant-goblin-warrior").as_deref(),
            Some("figure-ant-goblin-warrior")
        );
        assert_eq!(
            full_body_stem("sprite-bandit").as_deref(),
            Some("figure-bandit")
        );
    }

    #[test]
    fn patrons_contacts_villains_and_other_stems_have_none() {
        for stem in [
            "sprite-patron-merchant",
            "sprite-contact-miller",
            "sprite-villain-lich",
            "sprite-courier",
            "portrait-hero-1",
            "figure-hero-1",
            "",
        ] {
            assert_eq!(full_body_stem(stem), None, "{stem}");
        }
    }

    #[test]
    fn a_painting_gets_its_override_or_the_default_and_far_stands_higher_than_mid() {
        assert_eq!(
            staging_for("/assets/ui/generated/scene-place-inn-taproom"),
            DEFAULT
        );
        let quay = staging_for("assets/ui/generated/scene-salt-quay-arrival.webp");
        assert!(!quay.key_from_left);
        for staging in [DEFAULT, quay, staging_for("scene-pinewood-trail-arrival")] {
            assert!(staging.feet_far < staging.feet_mid);
            assert!(staging.feet_mid <= 100);
        }
    }
}
