//! Independent local panel ownership around the persistent campaign role mount.
//! Panel leases fence local presentation callbacks; they grant no account,
//! payment or game authority. The caller owns panel content and transport.
use crate::{RoleInput, RoleShell, RoleShellError};
use df_types::ClientBindingId;
use std::{cell::Cell, fmt, rc::Rc};
use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

#[derive(Debug)]
pub enum CampaignPanelError {
    Dom(JsValue),
    Campaign(RoleShellError),
    Disposed,
    StaleLease,
    InvalidPanelOwnership,
    ForeignDocument,
    GenerationExhausted,
    Cleanup {
        operation: Option<Box<CampaignPanelError>>,
        failure: JsValue,
    },
}
impl fmt::Display for CampaignPanelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dom(_) => formatter.write_str("panel DOM operation failed"),
            Self::Campaign(error) => fmt::Display::fmt(error, formatter),
            Self::Disposed => formatter.write_str("campaign panel shell is disposed"),
            Self::StaleLease => formatter.write_str("panel lease is no longer current"),
            Self::InvalidPanelOwnership => {
                formatter.write_str("panel root must be detached and caller owned")
            }
            Self::ForeignDocument => formatter.write_str("panel root belongs to another document"),
            Self::GenerationExhausted => formatter.write_str("panel generation exhausted"),
            Self::Cleanup { operation, .. } => {
                if let Some(operation) = operation {
                    write!(formatter, "{operation}; panel cleanup failed")
                } else {
                    formatter.write_str("panel cleanup failed")
                }
            }
        }
    }
}
impl std::error::Error for CampaignPanelError {}
impl From<JsValue> for CampaignPanelError {
    fn from(failure: JsValue) -> Self {
        Self::Dom(failure)
    }
}
impl From<RoleShellError> for CampaignPanelError {
    fn from(failure: RoleShellError) -> Self {
        Self::Campaign(failure)
    }
}

struct LeaseState {
    generation: Cell<u64>,
    active: Cell<Option<u64>>,
    disposed: Cell<bool>,
}

/// A bounded local presentation gate for one mounted panel. Retained leases
/// cannot update a replacement panel or apply an obsolete local completion.
#[derive(Clone)]
pub struct PanelLease {
    state: Rc<LeaseState>,
    generation: u64,
    root: Element,
    host: Element,
}
impl PanelLease {
    pub fn apply(
        &self,
        update: impl FnOnce(&Element) -> Result<(), JsValue>,
    ) -> Result<(), CampaignPanelError> {
        if self.state.disposed.get() {
            return Err(CampaignPanelError::Disposed);
        }
        if self.state.active.get() != Some(self.generation)
            || !self
                .root
                .parent_element()
                .is_some_and(|parent| parent.is_same_node(Some(&self.host)))
        {
            return Err(CampaignPanelError::StaleLease);
        }
        // No RefCell borrow or shell borrow is held across caller code.
        update(&self.root).map_err(CampaignPanelError::Dom)
    }
}
struct Panel {
    root: Element,
    cleanup: Box<dyn FnOnce() -> Result<(), JsValue>>,
}
impl Panel {
    fn retire(self) -> Result<(), CampaignPanelError> {
        self.root.remove();
        (self.cleanup)().map_err(|failure| CampaignPanelError::Cleanup {
            operation: None,
            failure,
        })
    }
    fn reject(self, operation: CampaignPanelError) -> CampaignPanelError {
        self.root.remove();
        match (self.cleanup)() {
            Ok(()) => operation,
            Err(failure) => CampaignPanelError::Cleanup {
                operation: Some(Box::new(operation)),
                failure,
            },
        }
    }
}

/// Owns one campaign RoleShell and at most one independent caller-mounted panel.
/// Opening, updating and closing a panel never present, suspend or dispose the
/// campaign, change its accepted revision, or initiate navigation.
pub struct CampaignPanelShell {
    document: Document,
    root: Element,
    panel_host: Element,
    campaign: RoleShell,
    panel: Option<Panel>,
    state: Rc<LeaseState>,
}
impl CampaignPanelShell {
    pub fn mount(
        document: &Document,
        parent: &Element,
        binding: ClientBindingId,
        input_identifier: &str,
        on_role_input: impl FnMut(RoleInput) + 'static,
    ) -> Result<Self, CampaignPanelError> {
        let root = document.create_element("div")?;
        root.set_attribute("data-campaign-panel-shell", "")?;
        let campaign_host = document.create_element("section")?;
        campaign_host.set_attribute("data-campaign-slot", "")?;
        let panel_host = document.create_element("aside")?;
        panel_host.set_attribute("data-customer-panel-slot", "")?;
        root.append_child(&campaign_host)?;
        root.append_child(&panel_host)?;
        let mut campaign = RoleShell::mount(
            document,
            &campaign_host,
            binding,
            input_identifier,
            on_role_input,
        )?;
        if let Err(failure) = parent.append_child(&root) {
            let operation = CampaignPanelError::Dom(failure);
            if let Err(cleanup) = campaign.dispose() {
                return Err(CampaignPanelError::Cleanup {
                    operation: Some(Box::new(operation)),
                    failure: JsValue::from_str(&cleanup.to_string()),
                });
            }
            return Err(operation);
        }
        Ok(Self {
            document: document.clone(),
            root,
            panel_host,
            campaign,
            panel: None,
            state: Rc::new(LeaseState {
                generation: Cell::new(0),
                active: Cell::new(None),
                disposed: Cell::new(false),
            }),
        })
    }
    pub fn root(&self) -> &Element {
        &self.root
    }
    pub fn campaign(&self) -> &RoleShell {
        &self.campaign
    }
    pub fn campaign_mut(&mut self) -> Result<&mut RoleShell, CampaignPanelError> {
        if self.state.disposed.get() {
            Err(CampaignPanelError::Disposed)
        } else {
            Ok(&mut self.campaign)
        }
    }
    /// Transfers one detached root and its exactly-once cleanup to this shell.
    /// Rejected transfers also run cleanup. The caller must remove its listeners
    /// and cancel its panel-owned work inside cleanup; no transport is invented.
    pub fn open_panel(
        &mut self,
        root: Element,
        on_dispose: impl FnOnce() -> Result<(), JsValue> + 'static,
    ) -> Result<PanelLease, CampaignPanelError> {
        let panel = Panel {
            root,
            cleanup: Box::new(on_dispose),
        };
        let admission = if self.state.disposed.get() {
            Err(CampaignPanelError::Disposed)
        } else if panel.root.parent_node().is_some()
            || panel.root.is_same_node(Some(&self.root))
            || panel.root.contains(Some(&self.root))
        {
            Err(CampaignPanelError::InvalidPanelOwnership)
        } else if !panel
            .root
            .owner_document()
            .is_some_and(|owner| self.document.is_same_node(Some(&owner)))
        {
            Err(CampaignPanelError::ForeignDocument)
        } else {
            self.state
                .generation
                .get()
                .checked_add(1)
                .ok_or(CampaignPanelError::GenerationExhausted)
        };
        let generation = match admission {
            Ok(generation) => generation,
            Err(operation) => {
                // Rejected connected/foreign roots remain where their caller put
                // them. Cleanup runs, but this shell never detaches foreign DOM.
                return match (panel.cleanup)() {
                    Ok(()) => Err(operation),
                    Err(failure) => Err(CampaignPanelError::Cleanup {
                        operation: Some(Box::new(operation)),
                        failure,
                    }),
                };
            }
        };
        if let Err(operation) = self.close_panel() {
            return Err(panel.reject(operation));
        }
        if let Err(failure) = self.panel_host.append_child(&panel.root) {
            return Err(panel.reject(CampaignPanelError::Dom(failure)));
        }
        self.state.generation.set(generation);
        self.state.active.set(Some(generation));
        let lease = PanelLease {
            state: Rc::clone(&self.state),
            generation,
            root: panel.root.clone(),
            host: self.panel_host.clone(),
        };
        self.panel = Some(panel);
        Ok(lease)
    }
    /// Fences the old lease before any fallible caller cleanup.
    pub fn close_panel(&mut self) -> Result<(), CampaignPanelError> {
        self.state.active.set(None);
        self.panel.take().map_or(Ok(()), |panel| panel.retire())
    }
    /// Terminal, idempotent whole-shell teardown. Both owners are attempted even
    /// if one cleanup fails; panel-close alone does not call this method.
    pub fn dispose(&mut self) -> Result<(), CampaignPanelError> {
        if self.state.disposed.replace(true) {
            return Ok(());
        }
        let panel = self.close_panel();
        let campaign = self
            .campaign
            .dispose()
            .map_err(CampaignPanelError::Campaign);
        self.root.remove();
        match (panel, campaign) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(operation), Err(failure)) => Err(CampaignPanelError::Cleanup {
                operation: Some(Box::new(operation)),
                failure: JsValue::from_str(&failure.to_string()),
            }),
        }
    }
}
impl Drop for CampaignPanelShell {
    fn drop(&mut self) {
        let _result = self.dispose();
        self.root.remove();
    }
}
