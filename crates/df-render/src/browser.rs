use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use web_sys::{Document, Element};

use crate::{ElementKey, FlatScene, RenderError, SceneOwner, SceneRenderer};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

#[derive(Debug)]
struct TokenNodes {
    group: Element,
    circle: Element,
    title: Element,
    label: Element,
}

#[derive(Debug)]
pub(crate) struct SvgMount {
    document: Document,
    root: Element,
    mount_root: Element,
    image_host: Element,
    camera_root: Element,
    layers_root: Element,
    tokens_root: Element,
    layers: BTreeMap<ElementKey, Element>,
    tokens: BTreeMap<ElementKey, TokenNodes>,
}

impl SceneRenderer {
    /// Mounts an owned flat SVG into an existing shell without replacing its children.
    /// Parent must be connected to its document. Errors contain no browser payloads.
    pub fn mount(parent: &Element, owner: SceneOwner) -> Result<Self, RenderError> {
        let document = parent
            .owner_document()
            .ok_or(RenderError::BrowserUnavailable)?;
        if !parent.is_connected() {
            return Err(RenderError::BrowserUnavailable);
        }
        let mount_root = document
            .create_element("div")
            .map_err(|_| RenderError::BrowserMutation)?;
        attribute(&mount_root, "data-scene-mount", "")?;
        let image_host = document
            .create_element("div")
            .map_err(|_| RenderError::BrowserMutation)?;
        attribute(&image_host, "data-scene-image-host", "")?;
        append(&mount_root, &image_host)?;
        let root = svg_element(&document, "svg")?;
        attribute(&root, "role", "img")?;
        attribute(&root, "width", "100%")?;
        attribute(&root, "height", "100%")?;
        attribute(&root, "preserveAspectRatio", "xMidYMid meet")?;
        attribute(
            &root,
            "style",
            "display:block;min-height:240px;background:#0f172a",
        )?;
        let layers_root = svg_element(&document, "g")?;
        let tokens_root = svg_element(&document, "g")?;
        let camera_root = svg_element(&document, "g")?;
        attribute(&camera_root, "data-scene-camera", "")?;
        attribute(&layers_root, "data-scene-layers", "")?;
        attribute(&tokens_root, "data-scene-tokens", "")?;
        append(&camera_root, &layers_root)?;
        append(&camera_root, &tokens_root)?;
        append(&root, &camera_root)?;
        append(&mount_root, &root)?;
        append(parent, &mount_root)?;
        let mut renderer = Self::empty(owner);
        renderer.browser = Some(SvgMount {
            document,
            root,
            mount_root,
            image_host,
            camera_root,
            layers_root,
            tokens_root,
            layers: BTreeMap::new(),
            tokens: BTreeMap::new(),
        });
        Ok(renderer)
    }

    /// Internal decoded-resource presentation slot, separate from world geometry.
    /// Resource owners release their handles before disposing this mounted scene.
    pub(crate) fn image_host(&self) -> Result<&Element, RenderError> {
        let mount = self
            .browser
            .as_ref()
            .ok_or(RenderError::BrowserUnavailable)?;
        if !mount.is_connected() {
            return Err(RenderError::BrowserUnavailable);
        }
        Ok(&mount.image_host)
    }

    /// Current mounted readiness, separate from declared implementation capabilities.
    pub fn is_mounted(&self) -> bool {
        self.browser.as_ref().is_some_and(SvgMount::is_connected)
    }
}

impl SvgMount {
    pub(crate) fn is_connected(&self) -> bool {
        self.mount_root.is_connected()
            && self.image_host.is_connected()
            && self.root.is_connected()
            && self.camera_root.is_connected()
            && self.layers_root.is_connected()
            && self.tokens_root.is_connected()
    }

    pub(crate) fn update(&mut self, scene: &FlatScene) -> Result<(), RenderError> {
        if !self.is_connected() {
            return Err(RenderError::BrowserUnavailable);
        }
        let viewport = scene.viewport;
        attribute(
            &self.root,
            "viewBox",
            &format!(
                "{} {} {} {}",
                viewport.origin.x, viewport.origin.y, viewport.width, viewport.height
            ),
        )?;
        attribute(&self.root, "aria-label", &scene.label)?;
        let layer_keys: BTreeSet<_> = scene.layers.iter().map(|layer| &layer.key).collect();
        let removed: Vec<_> = self
            .layers
            .keys()
            .filter(|key| !layer_keys.contains(*key))
            .cloned()
            .collect();
        for key in removed {
            if let Some(element) = self.layers.remove(&key) {
                retire_layer(&element)?;
            }
        }
        for layer in &scene.layers {
            if let Entry::Vacant(entry) = self.layers.entry(layer.key.clone()) {
                let element = svg_element(&self.document, "rect")?;
                attribute(&element, "data-layer-key", layer.key.as_str())?;
                entry.insert(element);
            }
            let element = self
                .layers
                .get(&layer.key)
                .ok_or(RenderError::BrowserMutation)?;
            attribute(element, "x", &layer.origin.x.to_string())?;
            attribute(element, "y", &layer.origin.y.to_string())?;
            attribute(element, "width", &layer.width.to_string())?;
            attribute(element, "height", &layer.height.to_string())?;
            attribute(element, "fill", layer.color.css())?;
            // Appending existing nodes reconciles z-order while retaining their identity.
            append(&self.layers_root, element)?;
        }
        let token_keys: BTreeSet<_> = scene.tokens.iter().map(|token| &token.key).collect();
        let removed: Vec<_> = self
            .tokens
            .keys()
            .filter(|key| !token_keys.contains(*key))
            .cloned()
            .collect();
        for key in removed {
            if let Some(nodes) = self.tokens.remove(&key) {
                nodes.retire()?;
            }
        }
        for token in &scene.tokens {
            if let Entry::Vacant(entry) = self.tokens.entry(token.key.clone()) {
                let group = svg_element(&self.document, "g")?;
                attribute(&group, "data-token-key", token.key.as_str())?;
                let circle = svg_element(&self.document, "circle")?;
                let title = svg_element(&self.document, "title")?;
                let label = svg_element(&self.document, "text")?;
                attribute(&circle, "stroke", "#ffffff")?;
                attribute(&circle, "stroke-width", "0.5")?;
                attribute(&label, "text-anchor", "middle")?;
                attribute(&label, "fill", "#ffffff")?;
                attribute(&label, "font-size", "4")?;
                append(&group, &title)?;
                append(&group, &circle)?;
                append(&group, &label)?;
                entry.insert(TokenNodes {
                    group,
                    circle,
                    title,
                    label,
                });
            }
            let nodes = self
                .tokens
                .get(&token.key)
                .ok_or(RenderError::BrowserMutation)?;
            attribute(
                &nodes.group,
                "transform",
                &format!("translate({} {})", token.position.x, token.position.y),
            )?;
            attribute(&nodes.circle, "r", &token.radius.to_string())?;
            attribute(&nodes.circle, "fill", token.color.css())?;
            attribute(&nodes.label, "y", &(token.radius + 5.0).to_string())?;
            nodes.title.set_text_content(Some(&token.label));
            nodes.label.set_text_content(Some(&token.label));
            append(&self.tokens_root, &nodes.group)?;
        }
        Ok(())
    }

    pub(crate) fn dispose(&mut self) -> Result<(), RenderError> {
        let mut failure = None;
        for element in self.layers.values() {
            if retire_layer(element).is_err() {
                failure = Some(RenderError::BrowserMutation);
            }
        }
        for nodes in self.tokens.values() {
            if nodes.retire().is_err() {
                failure = Some(RenderError::BrowserMutation);
            }
        }
        for name in ["aria-label", "viewBox"] {
            if self.root.remove_attribute(name).is_err() {
                failure = Some(RenderError::BrowserMutation);
            }
        }
        if self.camera_root.remove_attribute("transform").is_err() {
            failure = Some(RenderError::BrowserMutation);
        }
        self.root.set_text_content(None);
        self.image_host.set_text_content(None);
        self.mount_root.set_text_content(None);
        self.mount_root.remove();
        self.layers.clear();
        self.tokens.clear();
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn svg_element(document: &Document, name: &str) -> Result<Element, RenderError> {
    document
        .create_element_ns(Some(SVG_NAMESPACE), name)
        .map_err(|_| RenderError::BrowserMutation)
}

fn attribute(element: &Element, name: &str, value: &str) -> Result<(), RenderError> {
    element
        .set_attribute(name, value)
        .map_err(|_| RenderError::BrowserMutation)
}

fn append(parent: &Element, child: &Element) -> Result<(), RenderError> {
    parent
        .append_child(child)
        .map(|_| ())
        .map_err(|_| RenderError::BrowserMutation)
}

fn retire_layer(element: &Element) -> Result<(), RenderError> {
    let cleanup_result = clear_attributes(
        element,
        &["data-layer-key", "x", "y", "width", "height", "fill"],
    );
    element.remove();
    cleanup_result
}

fn clear_attributes(element: &Element, names: &[&str]) -> Result<(), RenderError> {
    let mut failure = None;
    for name in names {
        if element.remove_attribute(name).is_err() {
            failure = Some(RenderError::BrowserMutation);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

impl TokenNodes {
    fn retire(&self) -> Result<(), RenderError> {
        self.title.set_text_content(None);
        self.label.set_text_content(None);
        self.group.set_text_content(None);
        let mut failure = None;
        for cleanup_result in [
            clear_attributes(&self.group, &["data-token-key", "transform"]),
            clear_attributes(&self.circle, &["r", "fill"]),
            clear_attributes(&self.label, &["y"]),
        ] {
            if let Err(error) = cleanup_result {
                failure = Some(error);
            }
        }
        self.group.remove();
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
