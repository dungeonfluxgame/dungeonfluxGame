#[cfg(any(target_arch = "wasm32", test))]
use std::rc::Rc;

/// Already localized, audience-safe text. An error retains an explicit unknown
/// outcome alongside its safe error message; delivery never supplies success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackView<'a> {
    Pending(&'a str),
    Uncertain(&'a str),
    Refused(&'a str),
    Error {
        uncertainty: &'a str,
        message: &'a str,
    },
}

impl<'a> FeedbackView<'a> {
    /// Maps the existing receipt owner's observation using caller-localized labels.
    /// Absence and namespace retirement do not prove rejection or permit replay.
    /// A committed receipt requires its owner's accepted/rejected and pending-work
    /// presentation; returning None deliberately supplies no success label.
    pub fn for_resolution<Receipt>(
        resolution: &df_client::receipts::OperationResolution<Receipt>,
        pending: &'a str,
        uncertainty: &'a str,
    ) -> Option<Self> {
        use df_client::receipts::OperationResolution;

        match resolution {
            OperationResolution::InProgress => Some(Self::Pending(pending)),
            OperationResolution::Uncertain
            | OperationResolution::NotRecorded
            | OperationResolution::ExpiredOrIndeterminate
            | OperationResolution::NamespaceRetired { .. } => Some(Self::Uncertain(uncertainty)),
            OperationResolution::Committed { .. } => None,
        }
    }
}

/// Identity of one presentation owner, not a server operation or permission.
/// Capture this token before waiting; replacement rejects its late completions.
#[derive(Clone)]
#[cfg(any(target_arch = "wasm32", test))]
pub struct FeedbackScope(Rc<()>);

#[cfg(any(target_arch = "wasm32", test))]
struct FeedbackOwner {
    scope: Option<FeedbackScope>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl FeedbackOwner {
    fn create() -> (Self, FeedbackScope) {
        let scope = FeedbackScope(Rc::new(()));
        (
            Self {
                scope: Some(scope.clone()),
            },
            scope,
        )
    }

    fn require(&self, supplied: &FeedbackScope) -> Result<(), crate::DraftError> {
        let current = self.scope.as_ref().ok_or(crate::DraftError::Disposed)?;
        if !Rc::ptr_eq(&current.0, &supplied.0) {
            return Err(crate::DraftError::Unavailable);
        }
        Ok(())
    }

    fn replace(&mut self) -> Result<FeedbackScope, crate::DraftError> {
        if self.scope.is_none() {
            return Err(crate::DraftError::Disposed);
        }
        let scope = FeedbackScope(Rc::new(()));
        self.scope = Some(scope.clone());
        Ok(scope)
    }

    fn dispose(&mut self) {
        self.scope = None;
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::RefCell;

    use web_sys::{Document, Element};

    use super::{FeedbackOwner, FeedbackScope, FeedbackView};
    use crate::{ControlError, UiError};

    /// Stable status and alert nodes with no requests, timers, callbacks or retry
    /// action. The caller alone validates server receipts and recovery policy.
    pub struct OperationFeedback {
        root: Element,
        status: Element,
        alert: Element,
        owner: RefCell<FeedbackOwner>,
    }

    impl OperationFeedback {
        pub fn create(
            document: &Document,
            view: FeedbackView<'_>,
        ) -> Result<(Self, FeedbackScope), ControlError> {
            validate(view)?;
            let root = document.create_element("div")?;
            root.set_class_name("df-ui-panel");
            let status = document.create_element("p")?;
            status.set_class_name("df-ui-feedback");
            status.set_attribute("role", "status")?;
            status.set_attribute("aria-live", "polite")?;
            status.set_attribute("aria-atomic", "true")?;
            let alert = document.create_element("p")?;
            alert.set_class_name("df-ui-feedback");
            alert.set_attribute("role", "alert")?;
            alert.set_attribute("aria-live", "assertive")?;
            alert.set_attribute("aria-atomic", "true")?;
            root.append_child(&status)?;
            root.append_child(&alert)?;
            let (owner, scope) = FeedbackOwner::create();
            let widget = Self {
                root,
                status,
                alert,
                owner: RefCell::new(owner),
            };
            widget.update(&scope, view)?;
            Ok((widget, scope))
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Reconciles the same nodes. Stale scopes and disposed handles cannot
        /// replace current text or release the current presentation's busy state.
        pub fn update(
            &self,
            scope: &FeedbackScope,
            view: FeedbackView<'_>,
        ) -> Result<(), ControlError> {
            self.owner.borrow().require(scope)?;
            validate(view)?;
            self.render(view)
        }

        /// Begins a different owner on the mounted nodes. Invalid labels leave
        /// the prior owner intact. Old tokens never become fresh again.
        pub fn replace(&self, view: FeedbackView<'_>) -> Result<FeedbackScope, ControlError> {
            validate(view)?;
            let scope = self.owner.borrow_mut().replace()?;
            self.render(view)?;
            Ok(scope)
        }

        fn render(&self, view: FeedbackView<'_>) -> Result<(), ControlError> {
            let (kind, status, alert) = match view {
                FeedbackView::Pending(message) => ("pending", Some(message), None),
                FeedbackView::Uncertain(message) => ("uncertain", Some(message), None),
                FeedbackView::Refused(message) => ("refused", None, Some(message)),
                FeedbackView::Error {
                    uncertainty,
                    message,
                } => ("unresolved-error", Some(uncertainty), Some(message)),
            };
            self.root.set_attribute("data-df-feedback", kind)?;
            self.root.set_attribute(
                "aria-busy",
                if matches!(view, FeedbackView::Pending(_)) {
                    "true"
                } else {
                    "false"
                },
            )?;
            self.status.set_text_content(status);
            self.alert.set_text_content(alert);
            Ok(())
        }

        /// Terminal and idempotent. Clears retained text, fences every captured
        /// completion and removes only this widget. Explicit disposal reports DOM errors.
        pub fn dispose(&self) -> Result<(), ControlError> {
            self.owner.borrow_mut().dispose();
            self.status.set_text_content(None);
            self.alert.set_text_content(None);
            if let Some(parent) = self.root.parent_node() {
                parent.remove_child(&self.root)?;
            }
            Ok(())
        }
    }

    impl Drop for OperationFeedback {
        fn drop(&mut self) {
            self.owner.get_mut().dispose();
            self.status.set_text_content(None);
            self.alert.set_text_content(None);
            self.root.remove();
        }
    }

    fn validate(view: FeedbackView<'_>) -> Result<(), ControlError> {
        let (message, extra) = match view {
            FeedbackView::Pending(message)
            | FeedbackView::Uncertain(message)
            | FeedbackView::Refused(message) => (message, None),
            FeedbackView::Error {
                uncertainty,
                message,
            } => (uncertainty, Some(message)),
        };
        if message.trim().is_empty() || extra.is_some_and(|value| value.trim().is_empty()) {
            return Err(UiError::EmptyLabel.into());
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::OperationFeedback;

#[cfg(test)]
mod tests {
    use super::FeedbackOwner;
    use crate::DraftError;

    #[test]
    fn old_and_other_widget_completions_cannot_release_current_owner() {
        let (mut owner, old) = FeedbackOwner::create();
        let (_, foreign) = FeedbackOwner::create();
        let current = owner.replace().expect("live owner");
        assert_eq!(owner.require(&old), Err(DraftError::Unavailable));
        assert_eq!(owner.require(&foreign), Err(DraftError::Unavailable));
        assert_eq!(owner.require(&current), Ok(()));
    }

    #[test]
    fn disposal_rejects_completion_and_cannot_be_replaced() {
        let (mut owner, scope) = FeedbackOwner::create();
        owner.dispose();
        owner.dispose();
        assert_eq!(owner.require(&scope), Err(DraftError::Disposed));
        assert!(matches!(owner.replace(), Err(DraftError::Disposed)));
    }
}
