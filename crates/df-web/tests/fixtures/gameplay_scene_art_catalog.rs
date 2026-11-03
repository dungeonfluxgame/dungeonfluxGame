#[derive(Clone, Copy)]
pub(super) enum GameplaySceneArt {
    HarborExploration,
    HarborEncounter,
    CampfireRest,
}

pub(super) struct SceneArtSelection {
    pub(super) asset_path: &'static str,
    pub(super) title: &'static str,
    pub(super) image_description: &'static str,
    pub(super) fallback_label: &'static str,
}

impl GameplaySceneArt {
    pub(super) const fn selection(self) -> SceneArtSelection {
        match self {
            Self::HarborExploration => SceneArtSelection {
                asset_path: "assets/ui/scenes/mara-harbor-v4.png",
                title: "The Moonlit Harbor",
                image_description: "Mara stands on a lantern-lit dock beside the moonlit harbor.",
                fallback_label: "Harbor illustration unavailable",
            },
            // Generic prepared artwork is atmosphere, not Mara's identity or an action outcome.
            Self::HarborEncounter => SceneArtSelection {
                asset_path: "assets/placeholder/combat-harbor.webp",
                title: "Trouble on the Docks",
                image_description: "A painted harbor skirmish beneath warm dock lanterns.",
                fallback_label: "Dockside illustration unavailable",
            },
            Self::CampfireRest => SceneArtSelection {
                asset_path: "assets/placeholder/scene-campfire.webp",
                title: "Beneath the Stars",
                image_description: "A campfire glows beneath a starry sky above a misty valley.",
                fallback_label: "Campfire illustration unavailable",
            },
        }
    }
}
