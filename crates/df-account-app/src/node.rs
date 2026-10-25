//! The closed element tree every screen is described as, natively, before the browser sees it.
//!
//! Text is always a text node, never markup, so server or person-supplied text cannot become HTML.
//! An element carries at most one click intent, one submit intent and one bound field; the wasm
//! patcher turns them into delegated listeners and updates the previous tree in place.

use crate::state::{Field, Intent};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Element {
    pub tag: &'static str,
    pub attributes: Vec<(&'static str, String)>,
    pub children: Vec<Node>,
    pub on_click: Option<Intent>,
    pub on_submit: Option<Intent>,
    /// The form field whose value this input, select or checkbox edits.
    pub field: Option<Field>,
}

pub fn el(tag: &'static str) -> Element {
    Element {
        tag,
        attributes: Vec::new(),
        children: Vec::new(),
        on_click: None,
        on_submit: None,
        field: None,
    }
}

pub fn text(value: impl Into<String>) -> Node {
    Node::Text(value.into())
}

impl From<Element> for Node {
    fn from(element: Element) -> Self {
        Self::Element(element)
    }
}

impl Element {
    pub fn attr(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.attributes.push((name, value.into()));
        self
    }

    /// A boolean attribute, present when `is_on`.
    pub fn flag(self, name: &'static str, is_on: bool) -> Self {
        if is_on { self.attr(name, "") } else { self }
    }

    pub fn class(self, value: &str) -> Self {
        self.attr("class", value)
    }

    pub fn child(mut self, node: impl Into<Node>) -> Self {
        self.children.push(node.into());
        self
    }

    pub fn children(mut self, nodes: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(nodes);
        self
    }

    pub fn text(self, value: impl Into<String>) -> Self {
        self.child(text(value))
    }

    pub fn on_click(mut self, intent: Intent) -> Self {
        self.on_click = Some(intent);
        self
    }

    pub fn on_submit(mut self, intent: Intent) -> Self {
        self.on_submit = Some(intent);
        self
    }

    pub fn bind(mut self, field: Field) -> Self {
        self.field = Some(field);
        self
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

impl Node {
    /// All text below this node, joined by spaces (for tests and accessible names).
    pub fn text_content(&self) -> String {
        let mut collected = String::new();
        self.collect_text(&mut collected);
        collected.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn collect_text(&self, collected: &mut String) {
        match self {
            Self::Text(value) => {
                collected.push(' ');
                collected.push_str(value);
            }
            Self::Element(element) => {
                for child in &element.children {
                    child.collect_text(collected);
                }
            }
        }
    }

    /// Every element in document order, this one first.
    pub fn elements(&self) -> Vec<&Element> {
        let mut found = Vec::new();
        self.collect_elements(&mut found);
        found
    }

    fn collect_elements<'a>(&'a self, found: &mut Vec<&'a Element>) {
        if let Self::Element(element) = self {
            found.push(element);
            for child in &element.children {
                child.collect_elements(found);
            }
        }
    }
}
