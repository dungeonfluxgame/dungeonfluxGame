//! The pure half of the browser glue: how one `Node` tree becomes the next, which element an event
//! meant, and which addresses the app may leave for. Nothing here touches `web-sys`, so all of it
//! is tested natively; `dom.rs` applies the `Op`s and `runtime.rs` owns the listeners.
//!
//! Reconciliation is positional. At the same child index, two elements with the same tag are
//! patched in place (attributes, then children, then properties); a different tag, or a text node
//! against an element, is replaced; children beyond the new length are removed (last first, so the
//! remaining indices stay valid); extra new children are appended. An element is therefore never
//! replaced while its tag is unchanged, which keeps focus, a caret and an unsent draft in a field.
//!
//! Operations address nodes by a child-index path from the root element (`[]` is the root).
//! `dom.rs` applies them in order, so a path always names the node as it is at that point.
//!
//! Properties differ from attributes: the `value` of a text field, the `checked` state of a box or
//! radio, the `selected` state of an option and the `disabled` state of a control live on the live
//! element, not in its markup. They become `SetProperty` operations; `dom.rs` writes `Value`,
//! `Checked` and `Selected` only when the live property differs, so the browser's own caret and
//! selection survive a render that repeats what the person just typed. A bound control carries its
//! property on every render so the model stays the truth even if it declined what was typed.
//!
//! Event intents (`on_click`, `on_submit`, `field`) are never written to the DOM as handlers. The
//! runtime keeps the current tree and listens once on the root. For an event, `dom.rs` computes the
//! child-index path from the root to the target, and `click_target`, `submit_target` and
//! `field_target` find the nearest ancestor-or-self element on that path that carries the wanted
//! intent.

use crate::{
    node::{Element, Node},
    state::{Field, Intent},
};

/// Child indices from the root element to a node.
pub type Path = Vec<usize>;

/// A live-element property, as opposed to an attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Property {
    Value(String),
    Checked(bool),
    Selected(bool),
    Disabled(bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
    /// Build `node` and put it where the node at `path` is (the root when `path` is empty).
    Replace {
        path: Path,
        node: Node,
    },
    /// Build `node` and add it as the last child of the element at `parent`.
    Append {
        parent: Path,
        node: Node,
    },
    Remove {
        path: Path,
    },
    SetAttribute {
        path: Path,
        name: &'static str,
        value: String,
    },
    RemoveAttribute {
        path: Path,
        name: &'static str,
    },
    SetText {
        path: Path,
        text: String,
    },
    SetProperty {
        path: Path,
        property: Property,
    },
}

/// The operations that turn the live copy of `old` into `new`.
pub fn diff(old: &Node, new: &Node) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut path = Path::new();
    diff_node(old, new, &mut path, &mut ops);
    ops
}

fn diff_node(old: &Node, new: &Node, path: &mut Path, ops: &mut Vec<Op>) {
    match (old, new) {
        (Node::Text(before), Node::Text(after)) => {
            if before != after {
                ops.push(Op::SetText {
                    path: path.clone(),
                    text: after.clone(),
                });
            }
        }
        (Node::Element(before), Node::Element(after)) if before.tag == after.tag => {
            diff_element(before, after, path, ops);
        }
        _ => ops.push(Op::Replace {
            path: path.clone(),
            node: new.clone(),
        }),
    }
}

fn diff_element(old: &Element, new: &Element, path: &mut Path, ops: &mut Vec<Op>) {
    diff_attributes(old, new, path, ops);
    let shared = old.children.len().min(new.children.len());
    for (index, (before, after)) in old.children.iter().zip(&new.children).enumerate() {
        path.push(index);
        diff_node(before, after, path, ops);
        path.pop();
    }
    for index in (shared..old.children.len()).rev() {
        ops.push(Op::Remove {
            path: child_path(path, index),
        });
    }
    for child in new.children.iter().skip(shared) {
        ops.push(Op::Append {
            parent: path.clone(),
            node: child.clone(),
        });
    }
    // After the children: a select's value only means something once its options are in place.
    diff_properties(old, new, path, ops);
}

fn child_path(parent: &[usize], index: usize) -> Path {
    let mut path = parent.to_vec();
    path.push(index);
    path
}

/// The effective value of an attribute: the last one written wins, as when the DOM is built by
/// `set_attribute` in order.
fn attribute_value<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .rev()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_str())
}

fn has_flag(element: &Element, name: &str) -> bool {
    attribute_value(element, name).is_some()
}

fn is_check_input(element: &Element) -> bool {
    element.tag == "input" && matches!(attribute_value(element, "type"), Some("checkbox" | "radio"))
}

/// Whether `name` on `element` is a live property rather than an attribute.
fn is_property(element: &Element, name: &str) -> bool {
    match name {
        // A radio's or checkbox's `value` is a constant the form submits, not typed text.
        "value" => {
            matches!(element.tag, "input" | "textarea" | "select") && !is_check_input(element)
        }
        "checked" => is_check_input(element),
        "selected" => element.tag == "option",
        "disabled" => matches!(
            element.tag,
            "input" | "select" | "option" | "textarea" | "button"
        ),
        _ => false,
    }
}

/// The attributes `dom.rs` writes with `set_attribute` when it builds `element`.
pub fn plain_attributes(element: &Element) -> Vec<(&'static str, &str)> {
    let mut written: Vec<(&'static str, &str)> = Vec::new();
    for (name, _) in &element.attributes {
        if is_property(element, name) || written.iter().any(|(seen, _)| seen == name) {
            continue;
        }
        if let Some(value) = attribute_value(element, name) {
            written.push((name, value));
        }
    }
    written
}

/// The properties `dom.rs` sets on a freshly built `element`, after its children exist.
pub fn creation_properties(element: &Element) -> Vec<Property> {
    let mut properties = Vec::new();
    if is_property(element, "value")
        && let Some(value) = attribute_value(element, "value")
    {
        properties.push(Property::Value(value.to_owned()));
    }
    if is_property(element, "checked") && has_flag(element, "checked") {
        properties.push(Property::Checked(true));
    }
    if is_property(element, "selected") && has_flag(element, "selected") {
        properties.push(Property::Selected(true));
    }
    if is_property(element, "disabled") && has_flag(element, "disabled") {
        properties.push(Property::Disabled(true));
    }
    properties
}

fn diff_attributes(old: &Element, new: &Element, path: &[usize], ops: &mut Vec<Op>) {
    let mut seen: Vec<&'static str> = Vec::new();
    for (name, _) in &new.attributes {
        if is_property(new, name) || seen.contains(name) {
            continue;
        }
        seen.push(name);
        if let Some(value) = attribute_value(new, name)
            && attribute_value(old, name) != Some(value)
        {
            ops.push(Op::SetAttribute {
                path: path.to_vec(),
                name,
                value: value.to_owned(),
            });
        }
    }
    let mut removed: Vec<&'static str> = Vec::new();
    for (name, _) in &old.attributes {
        if is_property(new, name) || has_flag(new, name) || removed.contains(name) {
            continue;
        }
        removed.push(name);
        ops.push(Op::RemoveAttribute {
            path: path.to_vec(),
            name,
        });
    }
}

fn diff_properties(old: &Element, new: &Element, path: &[usize], ops: &mut Vec<Op>) {
    let mut push = |property: Property| {
        ops.push(Op::SetProperty {
            path: path.to_vec(),
            property,
        });
    };
    if is_property(new, "value") {
        match (attribute_value(new, "value"), attribute_value(old, "value")) {
            (Some(value), _) => push(Property::Value(value.to_owned())),
            (None, Some(_)) => push(Property::Value(String::new())),
            (None, None) => {}
        }
    }
    if is_property(new, "checked") {
        let is_checked = has_flag(new, "checked");
        if new.field.is_some() || is_checked != has_flag(old, "checked") {
            push(Property::Checked(is_checked));
        }
    }
    if is_property(new, "selected") {
        let is_selected = has_flag(new, "selected");
        // A deselected option must be told so; an option that was never selected says nothing.
        if is_selected || has_flag(old, "selected") {
            push(Property::Selected(is_selected));
        }
    }
    if is_property(new, "disabled") {
        let is_disabled = has_flag(new, "disabled");
        if is_disabled != has_flag(old, "disabled") {
            push(Property::Disabled(is_disabled));
        }
    }
}

/// An element found by walking up from an event target.
#[derive(Debug)]
pub struct Hit<'a, T> {
    pub value: T,
    pub element: &'a Element,
    /// The path of `element` itself (a prefix of the target's path).
    pub path: Path,
}

/// The nearest ancestor-or-self element of the node at `path` for which `pick` finds something.
fn nearest<'a, T>(
    root: &'a Node,
    path: &[usize],
    pick: impl Fn(&'a Element) -> Option<T>,
) -> Option<Hit<'a, T>> {
    let mut chain: Vec<&'a Element> = Vec::with_capacity(path.len() + 1);
    let mut current = root;
    for depth in 0..=path.len() {
        match current {
            // The target itself is text: the elements pushed so far are its ancestors.
            Node::Text(_) if depth == path.len() => break,
            // A path that runs through text does not describe this tree.
            Node::Text(_) => return None,
            Node::Element(element) => {
                chain.push(element);
                if let Some(index) = path.get(depth) {
                    current = element.children.get(*index)?;
                }
            }
        }
    }
    chain
        .iter()
        .copied()
        .enumerate()
        .rev()
        .find_map(|(depth, element)| {
            pick(element).map(|value| Hit {
                value,
                element,
                path: path.iter().take(depth).copied().collect(),
            })
        })
}

pub fn click_target<'a>(root: &'a Node, path: &[usize]) -> Option<Hit<'a, Intent>> {
    nearest(root, path, |element| element.on_click.clone())
}

pub fn submit_target<'a>(root: &'a Node, path: &[usize]) -> Option<Hit<'a, Intent>> {
    nearest(root, path, |element| element.on_submit.clone())
}

pub fn field_target<'a>(root: &'a Node, path: &[usize]) -> Option<Hit<'a, Field>> {
    nearest(root, path, |element| element.field)
}

/// How a bound control's value is read from the live element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    /// A text-like input or textarea: its text.
    Text,
    Checkbox,
    /// Reports its value only while it is the checked one of its group.
    Radio,
    Select,
}

pub fn control(element: &Element) -> Control {
    match (element.tag, attribute_value(element, "type")) {
        ("select", _) => Control::Select,
        ("input", Some("checkbox")) => Control::Checkbox,
        ("input", Some("radio")) => Control::Radio,
        _ => Control::Text,
    }
}

/// The checkout page of the payment provider; the only address outside the app the person is sent
/// to.
const CHECKOUT_PREFIX: &str = "https://checkout.stripe.com/";

/// Whether the app may navigate the whole page to `url`: a path inside this site or the hosted
/// checkout page. A second slash, a backslash or any control character could turn a path into
/// another site (`//host`, `/\host`, a tab inside `//`), so they are refused.
pub fn is_allowed_destination(url: &str) -> bool {
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let is_site_path = url.starts_with('/') && !url.starts_with("//");
    is_site_path || (url.starts_with(CHECKOUT_PREFIX) && url.len() > CHECKOUT_PREFIX.len())
}

#[cfg(test)]
mod patch_tests;
