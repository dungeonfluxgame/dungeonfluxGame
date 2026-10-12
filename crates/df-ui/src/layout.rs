use std::fmt;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlButtonElement, HtmlInputElement};

use crate::theme;

/// Selects responsive composition, never a permission or authorized client role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutRole {
    Player,
    SharedDisplay,
}

impl LayoutRole {
    const fn attribute(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::SharedDisplay => "shared-display",
        }
    }
}

/// DOM operation or binding conversion failure; no successful mount is fabricated.
#[derive(Debug)]
pub enum UiError {
    Browser(JsValue),
    WrongElementType,
    EmptyLabel,
}

impl fmt::Display for UiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browser(_) => formatter.write_str("browser DOM operation failed"),
            Self::WrongElementType => formatter.write_str("browser element type did not match"),
            Self::EmptyLabel => formatter.write_str("control or heading label is empty"),
        }
    }
}

impl std::error::Error for UiError {}

impl From<JsValue> for UiError {
    fn from(error: JsValue) -> Self {
        Self::Browser(error)
    }
}

/// Detached reusable layout with one scoped stylesheet and stable content root.
/// The caller mounts this root once, reconciles its children in place, and owns
/// listeners/resources on those children. Removing the root removes its stylesheet.
pub struct LayoutRoot {
    root: Element,
    content: Element,
}

impl LayoutRoot {
    pub fn create(document: &Document, role: LayoutRole) -> Result<Self, UiError> {
        let root = document.create_element("section")?;
        root.set_class_name("df-ui-root");
        root.set_attribute("data-df-layout", role.attribute())?;
        let styles = document.create_element("style")?;
        styles.set_text_content(Some(&theme::stylesheet()));
        let content = document.create_element("div")?;
        content.set_class_name("df-ui-content");
        root.append_child(&styles)?;
        root.append_child(&content)?;
        Ok(Self { root, content })
    }

    pub fn root(&self) -> &Element {
        &self.root
    }

    pub fn content(&self) -> &Element {
        &self.content
    }

    /// Changes composition in place; retains child nodes, focused inputs and drafts.
    pub fn set_role(&self, role: LayoutRole) -> Result<(), UiError> {
        self.root
            .set_attribute("data-df-layout", role.attribute())?;
        Ok(())
    }

    /// Applies the caller's presentation preference in place: false removes this explicit
    /// reduction. The device's own reduce-motion setting is never consulted.
    /// This changes no child nodes, native control behavior, focus, drafts or game state.
    pub fn set_reduced_motion(&self, reduce: bool) -> Result<(), UiError> {
        if reduce {
            self.root.set_attribute("data-df-reduced-motion", "true")?;
        } else {
            self.root.remove_attribute("data-df-reduced-motion")?;
        }
        Ok(())
    }

    /// Repeated calls succeed. Child owners dispose resources before removing a layout.
    pub fn unmount(&self) -> Result<(), UiError> {
        if let Some(parent) = self.root.parent_node() {
            parent.remove_child(&self.root)?;
        }
        Ok(())
    }
}

/// Creates a semantic section with an existing localized heading. Child content
/// is appended by the owning component; text is never interpreted as HTML.
pub fn panel(document: &Document, heading: &str) -> Result<Element, UiError> {
    validate_label(heading)?;
    let section = document.create_element("section")?;
    section.set_class_name("df-ui-panel");
    let title = document.create_element("h2")?;
    title.set_text_content(Some(heading));
    section.append_child(&title)?;
    Ok(section)
}

pub fn stack(document: &Document) -> Result<Element, UiError> {
    let element = document.create_element("div")?;
    element.set_class_name("df-ui-stack");
    Ok(element)
}

/// Native button supplies keyboard and touch semantics. The action owner attaches
/// its scoped listener and sends only a current server-advertised command.
pub fn action_button(
    document: &Document,
    label: &str,
    enabled: bool,
) -> Result<HtmlButtonElement, UiError> {
    validate_label(label)?;
    let button = document
        .create_element("button")?
        .dyn_into::<HtmlButtonElement>()
        .map_err(|_| UiError::WrongElementType)?;
    button.set_class_name("df-ui-action");
    button.set_type("button");
    button.set_tab_index(0);
    button.set_disabled(!enabled);
    button.set_text_content(Some(label));
    Ok(button)
}

/// The enclosing native label associates the input without shared ID collisions.
/// Callers retain and update the returned input, so ordinary render updates do
/// not overwrite a valid local draft. No listener or remote work is created.
pub fn text_input(
    document: &Document,
    label: &str,
) -> Result<(Element, HtmlInputElement), UiError> {
    validate_label(label)?;
    let field = document.create_element("label")?;
    field.set_class_name("df-ui-field");
    let caption = document.create_element("span")?;
    caption.set_text_content(Some(label));
    let input = document
        .create_element("input")?
        .dyn_into::<HtmlInputElement>()
        .map_err(|_| UiError::WrongElementType)?;
    input.set_class_name("df-ui-input");
    input.set_type("text");
    field.append_child(&caption)?;
    field.append_child(&input)?;
    Ok((field, input))
}

fn validate_label(label: &str) -> Result<(), UiError> {
    if label.trim().is_empty() {
        return Err(UiError::EmptyLabel);
    }
    Ok(())
}
