//! The live DOM: applies `patch::Op`s, finds the path of an event target and moves focus.
//!
//! Text is always a text node or `set_text_content`, attributes always `set_attribute`; nothing here
//! builds markup from a string. The mount element holds exactly one child, the root of the tree
//! (`[]` in a path). Properties are read and written through `Reflect` by name, because all of
//! them (`value`, `checked`, `selected`, `disabled`) are plain JS properties shared by several
//! element types; a property is written only when the live one differs, so a caret, a selection
//! and an in-progress composition are not disturbed by a render that repeats what is already there.

use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::{
    node::Node,
    patch::{self, Op, Path, Property},
    state::FocusTarget,
};

#[derive(Clone)]
pub struct Dom {
    document: Document,
    mount: Element,
}

fn missing(what: &str) -> JsValue {
    JsValue::from_str(&format!("the DOM does not match the tree: no {what}"))
}

impl Dom {
    pub fn new(document: Document, mount: Element) -> Self {
        Self { document, mount }
    }

    pub fn mount(&self) -> &Element {
        &self.mount
    }

    pub fn set_title(&self, title: &str) {
        self.document.set_title(title);
    }

    /// Replaces everything in the mount with a fresh copy of `tree` (first render, and recovery
    /// after an operation could not be applied).
    pub fn rebuild(&self, tree: &Node) -> Result<(), JsValue> {
        let fresh = self.create(tree)?;
        self.mount.set_text_content(None);
        self.mount.append_child(&fresh)?;
        Ok(())
    }

    pub fn apply(&self, ops: &[Op]) -> Result<(), JsValue> {
        ops.iter().try_for_each(|op| self.apply_one(op))
    }

    fn apply_one(&self, op: &Op) -> Result<(), JsValue> {
        match op {
            Op::Replace { path, node } => {
                let fresh = self.create(node)?;
                match self.node_at(path) {
                    Some(old) => {
                        let parent = old.parent_node().ok_or_else(|| missing("parent"))?;
                        parent.replace_child(&fresh, &old)?;
                    }
                    None if path.is_empty() => {
                        self.mount.append_child(&fresh)?;
                    }
                    None => return Err(missing("node to replace")),
                }
            }
            Op::Append { parent, node } => {
                let parent = self
                    .node_at(parent)
                    .ok_or_else(|| missing("parent to extend"))?;
                parent.append_child(&self.create(node)?)?;
            }
            Op::Remove { path } => {
                let old = self
                    .node_at(path)
                    .ok_or_else(|| missing("node to remove"))?;
                let parent = old.parent_node().ok_or_else(|| missing("parent"))?;
                parent.remove_child(&old)?;
            }
            Op::SetAttribute { path, name, value } => {
                self.element_at(path)?.set_attribute(name, value)?;
            }
            Op::RemoveAttribute { path, name } => {
                self.element_at(path)?.remove_attribute(name)?;
            }
            Op::SetText { path, text } => {
                let node = self.node_at(path).ok_or_else(|| missing("text node"))?;
                node.set_text_content(Some(text));
            }
            Op::SetProperty { path, property } => {
                write_property(&self.element_at(path)?, property)?;
            }
        }
        Ok(())
    }

    fn create(&self, node: &Node) -> Result<web_sys::Node, JsValue> {
        match node {
            Node::Text(value) => Ok(self.document.create_text_node(value).into()),
            Node::Element(source) => {
                let element = self.document.create_element(source.tag)?;
                for (name, value) in patch::plain_attributes(source) {
                    element.set_attribute(name, value)?;
                }
                for child in &source.children {
                    element.append_child(&self.create(child)?)?;
                }
                // Properties last: a select's value needs its options, a radio's `checked` its type.
                for property in patch::creation_properties(source) {
                    write_property(&element, &property)?;
                }
                Ok(element.into())
            }
        }
    }

    /// The live node at a path, `[]` being the root of the tree.
    pub fn node_at(&self, path: &[usize]) -> Option<web_sys::Node> {
        let mut current = self.mount.first_child()?;
        for index in path {
            current = current.child_nodes().item(u32::try_from(*index).ok()?)?;
        }
        Some(current)
    }

    fn element_at(&self, path: &[usize]) -> Result<Element, JsValue> {
        self.node_at(path)
            .ok_or_else(|| missing("node"))?
            .dyn_into::<Element>()
            .map_err(|_| missing("element"))
    }

    /// The child-index path from the root of the tree to `target`, or `None` when the target is
    /// outside the tree (a removed node, a browser extension's element).
    pub fn path_of(&self, target: &web_sys::Node) -> Option<Path> {
        let mut path = Path::new();
        let mut current = target.clone();
        loop {
            let parent = current.parent_node()?;
            if parent.is_same_node(Some(self.mount.as_ref())) {
                break;
            }
            path.push(index_among_siblings(&current));
            current = parent;
        }
        path.reverse();
        Some(path)
    }

    /// The live `value` of the control at `path`.
    pub fn value_at(&self, path: &[usize]) -> Option<String> {
        let element = self.element_at(path).ok()?;
        Reflect::get(&element, &JsValue::from_str("value"))
            .ok()?
            .as_string()
    }

    /// The live `checked` state of the control at `path`.
    pub fn is_checked_at(&self, path: &[usize]) -> Option<bool> {
        let element = self.element_at(path).ok()?;
        Reflect::get(&element, &JsValue::from_str("checked"))
            .ok()?
            .as_bool()
    }

    /// Moves focus to the screen's heading or error summary. A screen without one is fine: the
    /// person keeps their place.
    pub fn focus(&self, target: FocusTarget) -> Result<(), JsValue> {
        let selector = match target {
            FocusTarget::Heading => "[data-df-focus=\"heading\"]",
            FocusTarget::ErrorSummary => "[data-df-focus=\"error-summary\"]",
            FocusTarget::Notice => "[data-df-focus=\"notice\"]",
        };
        let Some(found) = self.mount.query_selector(selector)? else {
            return Ok(());
        };
        let element = found
            .dyn_into::<web_sys::HtmlElement>()
            .map_err(|_| missing("focusable element"))?;
        if target == FocusTarget::Heading {
            // A new screen starts at its top; focus then moves without scrolling the heading to
            // the middle of a tall page (the landing picture would hide the top bar).
            if let Some(window) = web_sys::window() {
                window.scroll_to_with_x_and_y(0.0, 0.0);
            }
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            return element.focus_with_options(&options);
        }
        element.focus()
    }
}

fn index_among_siblings(node: &web_sys::Node) -> usize {
    let mut index = 0;
    let mut sibling = node.previous_sibling();
    while let Some(previous) = sibling {
        index += 1;
        sibling = previous.previous_sibling();
    }
    index
}

fn write_property(element: &Element, property: &Property) -> Result<(), JsValue> {
    match property {
        Property::Value(value) => set_property(element, "value", JsValue::from_str(value)),
        Property::Checked(is_checked) => {
            set_property(element, "checked", JsValue::from_bool(*is_checked))
        }
        Property::Selected(is_selected) => {
            set_property(element, "selected", JsValue::from_bool(*is_selected))
        }
        Property::Disabled(is_disabled) => {
            set_property(element, "disabled", JsValue::from_bool(*is_disabled))
        }
    }
}

/// Writes `name` only when the live property differs (`JsValue` equality is JS `===`).
fn set_property(element: &Element, name: &str, wanted: JsValue) -> Result<(), JsValue> {
    let key = JsValue::from_str(name);
    if Reflect::get(element, &key)? == wanted {
        return Ok(());
    }
    Reflect::set(element, &key, &wanted)?;
    Ok(())
}
