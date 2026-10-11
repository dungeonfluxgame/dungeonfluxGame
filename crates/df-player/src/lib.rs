//! Player-role presentation mounts for already permitted views.
mod gameplay_view;
pub use gameplay_view::{GameplayViewError, PlayerGameplayProjection};
mod character_creation;
pub use character_creation::PlayerCharacterConnection;
#[cfg(target_arch = "wasm32")]
pub use character_creation::PlayerCharacterScreen;

mod character_sheet;
pub use character_sheet::PlayerSheetConnection;
#[cfg(target_arch = "wasm32")]
pub use character_sheet::PlayerSheetScreen;

mod exploration;
#[cfg(target_arch = "wasm32")]
pub use exploration::{ExplorationMountError, PlayerExploration};

mod combat;
#[cfg(target_arch = "wasm32")]
pub use combat::{CombatMountError, PlayerCombat};

mod session_overlays;
#[cfg(target_arch = "wasm32")]
pub use session_overlays::{PlayerSessionOverlays, SessionOverlayMountError};

mod join;
pub use join::{PlayerJoinConnection, PlayerJoinInput};
#[cfg(target_arch = "wasm32")]
pub use join::{PlayerJoinError, PlayerJoinScreen};

mod action_draft;
pub use action_draft::{
    ActionDraft, ActionDraftError, ActionDraftStatus, ActionLookupError, ActionReceiptError,
    MAX_RETAINED_ACTION_BYTES,
};
#[cfg(target_arch = "wasm32")]
pub use action_draft::{ActionDraftLabels, ActionDraftMountError, MountedActionDraft};
