//! Shared Rust browser presentation primitives.
//!
//! Labels are already localized plain text. Role layout is presentation only;
//! callers retain server-authorized view, action, focus, and resource ownership.

mod campaign_surface;
mod scene_image;
pub use scene_image::{CampaignSceneAssets, SceneImageError, SceneImageLimits};
#[cfg(target_arch = "wasm32")]
mod campaign_theme;
#[cfg(target_arch = "wasm32")]
mod controls;
mod draft_state;
#[cfg(target_arch = "wasm32")]
mod layout;
mod phase_assets;
mod theme;

#[cfg(target_arch = "wasm32")]
pub use campaign_surface::{CampaignError, CampaignIllustration, CampaignSurface};
pub use campaign_surface::{
    CampaignLimits, CampaignMember, CampaignObjective, CampaignValidationError, CampaignView,
    ConceptScene, ObjectiveState,
};

#[cfg(target_arch = "wasm32")]
pub use controls::{
    ActionView, ControlError, ControlledAction, ControlledTextInput, DraftUpdate, InputFeedback,
    TextInputView,
};
pub use draft_state::{DraftError, MAX_DRAFT_UTF16_UNITS};
#[cfg(target_arch = "wasm32")]
pub use layout::{LayoutRole, LayoutRoot, UiError, action_button, panel, stack, text_input};
pub use theme::ThemeToken;

pub use phase_assets::{MAX_PHASE_ASSET_BYTES, PHASE_ASSETS, PhaseAsset, PhaseAssetFallback};

mod join_phase;
#[cfg(target_arch = "wasm32")]
mod join_phase_theme;
pub use join_phase::{
    JoinFeedback, JoinPhaseIntent, JoinPhaseView, JoinRoom, JoinStage, JoinStamp, JoinUpdate,
    JoinValidationError, LobbyOffer, LobbyParticipant, validate_join_drafts,
};
#[cfg(target_arch = "wasm32")]
pub use join_phase::{JoinPhaseError, JoinPhaseSurface};

mod character_phase;
#[cfg(target_arch = "wasm32")]
mod character_phase_theme;
pub use character_phase::{
    CharacterAction, CharacterActionKind, CharacterAppearanceDraft, CharacterChoice, CharacterFact,
    CharacterGroup, CharacterLimits, CharacterOption, CharacterPhaseView, CharacterPortrait,
    CharacterStatus, CharacterSubmission, CharacterValidationError,
};
#[cfg(target_arch = "wasm32")]
pub use character_phase::{CharacterPhaseError, CharacterPhaseSurface};

mod exploration_phase;
#[cfg(target_arch = "wasm32")]
mod exploration_phase_theme;

pub use exploration_phase::{
    ExplorationChoice, ExplorationInput, ExplorationLimits, ExplorationNpc, ExplorationPortrait,
    ExplorationValidationError, ExplorationView,
};
#[cfg(target_arch = "wasm32")]
pub use exploration_phase::{ExplorationError, ExplorationPhase};

mod combat_phase;
#[cfg(target_arch = "wasm32")]
mod combat_phase_theme;

pub use combat_phase::{
    CombatActor, CombatArt, CombatDraft, CombatFeedback, CombatHitPoints, CombatIntent,
    CombatLimits, CombatOffer, CombatOfferKind, CombatPhaseState, CombatPhaseView, CombatReaction,
    CombatResource, CombatRoll, CombatTone, CombatValidationError, CombatVerdict,
};
#[cfg(target_arch = "wasm32")]
pub use combat_phase::{CombatError, CombatPhaseSurface};

mod character_sheet_phase;
#[cfg(target_arch = "wasm32")]
mod character_sheet_phase_theme;

#[cfg(target_arch = "wasm32")]
pub use character_sheet_phase::{CharacterSheetSurface, SheetError};
pub use character_sheet_phase::{
    CharacterSheetView, SheetField, SheetLabels, SheetOffer, SheetOwnerGeneration, SheetRow,
    SheetSection, SheetSubmission, SheetTab, SheetValidationError,
};

mod feedback;
mod session_overlay_phase;
#[cfg(target_arch = "wasm32")]
mod session_overlay_phase_theme;

pub use feedback::FeedbackView;
#[cfg(target_arch = "wasm32")]
pub use feedback::{FeedbackScope, OperationFeedback};
pub use session_overlay_phase::{
    MAX_SESSION_HOST_OFFERS, SessionBookend, SessionBookendKind, SessionConnection,
    SessionOverlayOffer, SessionOverlaySelection, SessionOverlayValidationError,
    SessionOverlayView,
};
#[cfg(target_arch = "wasm32")]
pub use session_overlay_phase::{SessionOverlayError, SessionOverlayPhase};

mod character_display_phase;
#[cfg(target_arch = "wasm32")]
mod character_display_phase_theme;

#[cfg(target_arch = "wasm32")]
pub use character_display_phase::CharacterDisplaySurface;
pub use character_display_phase::{
    CharacterDisplayConnection, CharacterDisplayHostOffer, CharacterDisplayLimits,
    CharacterDisplaySubmission, CharacterDisplayView, CharacterPublicMember,
    CharacterPublicReadiness,
};

mod campfire_phase;
pub use campfire_phase::{
    CampfireAction, CampfireMember, CampfireOffer, CampfireSelection, CampfireValidationError,
    CampfireView,
};
#[cfg(target_arch = "wasm32")]
pub use campfire_phase::{CampfireError, CampfireSurface};

mod encounter_aftermath_phase;
#[cfg(target_arch = "wasm32")]
pub use encounter_aftermath_phase::{AftermathError, EncounterAftermathSurface};
pub use encounter_aftermath_phase::{
    AftermathNextScene, AftermathPartyMember, AftermathSelection, AftermathValidationError,
    EncounterAftermathView,
};

mod scene_transition_phase;
#[cfg(target_arch = "wasm32")]
pub use scene_transition_phase::{SceneTransitionError, SceneTransitionPhase};
pub use scene_transition_phase::{SceneTransitionValidationError, SceneTransitionView};
