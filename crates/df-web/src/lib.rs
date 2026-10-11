//! Persistent browser composition over authorized role presentation views.

#[cfg(target_arch = "wasm32")]
mod role_shell;

#[cfg(target_arch = "wasm32")]
pub use role_shell::{DisplayPhase, PlayerPhase, RoleInput, RolePhase, RoleShell, RoleShellError};

#[cfg(target_arch = "wasm32")]
mod campaign_panel;
#[cfg(target_arch = "wasm32")]
pub use campaign_panel::{CampaignPanelError, CampaignPanelShell, PanelLease};
