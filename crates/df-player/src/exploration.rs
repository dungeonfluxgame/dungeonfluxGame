//! Persistent player exploration mount; accepted presentation is not a wire contract.
//!
//! The caller must supply player-specific already audience-filtered snapshots. Binding IDs only
//! correlate updates; they never establish permission. In particular, this module
//! does not accept private checkpoints and redact them for another audience.

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{
        cell::{Cell, RefCell},
        fmt,
    };

    use df_client::cache::{CacheScope, FetchToken};
    use df_client::revisions::{ViewAcceptance, ViewStore};
    use df_types::SessionRevision;
    use df_ui::{
        CampaignSceneAssets, ExplorationError, ExplorationInput, ExplorationLimits,
        ExplorationPhase, ExplorationView, SceneImageLimits,
    };
    use web_sys::{Document, Element};

    #[derive(Debug)]
    pub enum ExplorationMountError {
        Surface(ExplorationError),
        Admission(ViewAcceptance),
        Revoked,
    }

    impl fmt::Display for ExplorationMountError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Surface(error) => fmt::Display::fmt(error, formatter),
                Self::Admission(admission) => write!(
                    formatter,
                    "exploration view was not accepted: {admission:?}"
                ),
                Self::Revoked => formatter.write_str("exploration mount is revoked"),
            }
        }
    }
    impl std::error::Error for ExplorationMountError {}
    impl From<ExplorationError> for ExplorationMountError {
        fn from(error: ExplorationError) -> Self {
            Self::Surface(error)
        }
    }

    /// One binding's persistent exploration surface. Reconnect suspends input until
    /// a newer accepted snapshot arrives. A binding replacement requires revoke
    /// followed by a new mount; retained private DOM is scrubbed by df-ui disposal.
    pub struct PlayerExploration {
        phase: ExplorationPhase,
        order: RefCell<ViewStore<()>>,
        limits: ExplorationLimits,
        is_revoked: Cell<bool>,
    }

    impl PlayerExploration {
        /// Mounts the existing df-ui composition and forwards its exact advertised
        /// inputs. The callback submits proposals; it must not resolve game outcomes.
        /// Display views must already exclude private story, NPC context and drafts.
        pub fn mount(
            document: &Document,
            parent: &Element,
            draft_identifier: &str,
            view: &ExplorationView<'_>,
            limits: ExplorationLimits,
            on_input: impl FnMut(ExplorationInput) + 'static,
        ) -> Result<Self, ExplorationMountError> {
            let phase = ExplorationPhase::create(document, draft_identifier, view, limits)?;
            phase.on_input(on_input)?;
            parent
                .append_child(phase.root())
                .map_err(ExplorationError::from)?;
            let mut order = ViewStore::new(view.binding);
            let admission = order.accept(view.binding, view.revision, ());
            if admission != ViewAcceptance::Applied {
                return Err(ExplorationMountError::Admission(admission));
            }
            Ok(Self {
                phase,
                order: RefCell::new(order),
                limits,
                is_revoked: Cell::new(false),
            })
        }

        pub fn root(&self) -> &Element {
            self.phase.root()
        }

        pub fn revision(&self) -> Option<SessionRevision> {
            self.order.borrow().current().map(|(revision, ())| revision)
        }

        /// Valid same-binding snapshots update in place. df-ui keeps keyed offers,
        /// valid focus and same-offer drafts; an epoch or draft-owner change clears
        /// obsolete input. Failed rendering keeps the attempted watermark and input
        /// suspended; recovery requires a newer complete permitted snapshot.
        pub fn update(&self, view: &ExplorationView<'_>) -> Result<(), ExplorationMountError> {
            if self.is_revoked.get() {
                return Err(ExplorationMountError::Revoked);
            }
            view.validate(self.limits).map_err(ExplorationError::from)?;
            let admission = self
                .order
                .borrow_mut()
                .accept(view.binding, view.revision, ());
            if admission != ViewAcceptance::Applied {
                return Err(ExplorationMountError::Admission(admission));
            }
            self.phase.set_visible(true)?;
            if let Err(error) = self.phase.update(view) {
                // Explicit cleanup failure remains visible to the owner. Both paths
                // fence input before another browser event can run.
                self.phase.set_visible(false)?;
                return Err(error.into());
            }
            Ok(())
        }

        pub fn enable_scene_assets(
            &self,
            scope: CacheScope,
            limits: SceneImageLimits,
        ) -> Result<(), ExplorationMountError> {
            if self.is_revoked.get() {
                return Err(ExplorationMountError::Revoked);
            }
            self.phase
                .enable_scene_assets(scope, limits)
                .map_err(Into::into)
        }
        pub fn update_with_scene_assets(
            &self,
            view: &ExplorationView<'_>,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<Option<FetchToken>, ExplorationMountError> {
            if self.is_revoked.get() {
                return Err(ExplorationMountError::Revoked);
            }
            view.validate(self.limits).map_err(ExplorationError::from)?;
            self.phase.validate_scene_assets(view, &assets)?;
            let admission = self
                .order
                .borrow_mut()
                .accept(view.binding, view.revision, ());
            if admission != ViewAcceptance::Applied {
                return Err(ExplorationMountError::Admission(admission));
            }
            self.phase.set_visible(true)?;
            match self.phase.update_with_scene_assets(view, assets) {
                Ok(fetch) => Ok(fetch),
                Err(error) => {
                    self.phase.set_visible(false)?;
                    Err(error.into())
                }
            }
        }
        pub fn complete_scene_asset(
            &self,
            token: &FetchToken,
            bytes: Vec<u8>,
        ) -> Result<(), ExplorationMountError> {
            if self.is_revoked.get() {
                return Err(ExplorationMountError::Revoked);
            }
            self.phase
                .complete_scene_asset(token, bytes)
                .map_err(Into::into)
        }

        /// Connection/presentation suspension preserves valid drafts and nodes but
        /// disables submissions. Only update with a newer accepted view resumes.
        pub fn suspend_input(&self) -> Result<(), ExplorationMountError> {
            if self.is_revoked.get() {
                return Err(ExplorationMountError::Revoked);
            }
            self.phase
                .set_visible(false)
                .map_err(ExplorationMountError::from)
        }

        /// Terminal, idempotent disposal. Caller-held raw Text, attributes and draft
        /// references are erased in place; callbacks can no longer submit input.
        pub fn revoke(&self) -> Result<(), ExplorationMountError> {
            self.is_revoked.set(true);
            self.phase.dispose().map_err(ExplorationMountError::from)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{ExplorationMountError, PlayerExploration};
