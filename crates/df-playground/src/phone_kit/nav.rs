//! Where a phone is in its pages: the page open, the pages it came from and whether the "more"
//! sheet is up. It is presentation state only (the demo's, and the live phone's tab bar): a tap
//! carries an action string, `NavAction` reads it, and `PhoneNav` applies it. No host is asked.

/// The prefix of an action that opens a page: `page:bag`.
const PAGE_PREFIX: &str = "page:";
/// The page name that stands for the page before this one.
const BACK_PAGE: &str = "back";
/// The action of the bar's "more" slot.
pub const MORE_ACTION: &str = "more";
/// The action of a tap outside the open "more" sheet.
pub const CLOSE_MORE_ACTION: &str = "more-close";

/// The action that opens page `key`.
pub fn page_action(key: &str) -> String {
    format!("{PAGE_PREFIX}{key}")
}

/// What a tap on the bar or in a page asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavAction {
    Go(String),
    Back,
    ToggleMore,
    CloseMore,
}

impl NavAction {
    /// The navigation an action string asks for, or `None` when it is something else's.
    pub fn parse(action: &str) -> Option<Self> {
        if action == MORE_ACTION {
            return Some(Self::ToggleMore);
        }
        if action == CLOSE_MORE_ACTION {
            return Some(Self::CloseMore);
        }
        match action.strip_prefix(PAGE_PREFIX)? {
            BACK_PAGE => Some(Self::Back),
            page if !page.is_empty() => Some(Self::Go(page.to_owned())),
            _ => None,
        }
    }
}

/// The page open and the way back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneNav {
    pub page: String,
    history: Vec<String>,
    pub more_open: bool,
}

impl PhoneNav {
    pub fn new(page: &str) -> Self {
        Self {
            page: page.to_owned(),
            history: Vec::new(),
            more_open: false,
        }
    }

    /// Applies one tap. Opening a page closes the "more" sheet; going back with nowhere to go
    /// back to stays where it is.
    pub fn apply(&mut self, action: NavAction) {
        match action {
            NavAction::Go(page) => {
                if page != self.page {
                    let before = std::mem::replace(&mut self.page, page);
                    self.history.push(before);
                }
                self.more_open = false;
            }
            NavAction::Back => {
                if let Some(before) = self.history.pop() {
                    self.page = before;
                }
                self.more_open = false;
            }
            NavAction::ToggleMore => self.more_open = !self.more_open,
            NavAction::CloseMore => self.more_open = false,
        }
    }
}
