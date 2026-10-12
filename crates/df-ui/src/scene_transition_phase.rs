use std::fmt;

use df_locale::VersionedFormattedMessage;

use crate::ConceptScene;

const MAX_TEXT_BYTES: usize = 4096;

/// Already localized, audience-safe transition content from the presentation owner.
/// Continue is a proposal callback; displaying this screen never advances game state.
/// The owner admits ordered snapshots and revalidates its current offer on activation.
pub struct SceneTransitionView<'a> {
    pub scene: ConceptScene,
    pub location: &'a str,
    pub title: &'a str,
    pub narration: &'a str,
    pub continue_label: &'a str,
    pub continue_enabled: bool,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneTransitionValidationError {
    EmptyText,
    TextLimit,
    Disposed,
    CatalogVersionMismatch,
}

impl fmt::Display for SceneTransitionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyText => "scene transition text is empty",
            Self::TextLimit => "scene transition text exceeds its bound",
            Self::Disposed => "scene transition was disposed",
            Self::CatalogVersionMismatch => "scene transition catalog revision does not match",
        })
    }
}

impl std::error::Error for SceneTransitionValidationError {}

impl SceneTransitionView<'_> {
    /// Consumes location, title, narration, and Continue messages in that order.
    /// The presentation owner selects stable keys and negotiates the locale before
    /// formatting. Every message must belong to the owner's admitted revision;
    /// rejection returns no partial text and includes neither values nor revisions.
    pub fn catalog_text<V: PartialEq>(
        expected_version: &V,
        messages: [&VersionedFormattedMessage<'_, V>; 4],
    ) -> Result<[String; 4], SceneTransitionValidationError> {
        if messages
            .iter()
            .any(|message| message.version != expected_version)
        {
            return Err(SceneTransitionValidationError::CatalogVersionMismatch);
        }
        Ok(messages.map(|message| message.message.plain_text()))
    }

    #[cfg(target_arch = "wasm32")]
    fn with_catalog_text<'a>(&self, text: &'a [String; 4]) -> SceneTransitionView<'a> {
        SceneTransitionView {
            scene: self.scene,
            location: &text[0],
            title: &text[1],
            narration: &text[2],
            continue_label: &text[3],
            continue_enabled: self.continue_enabled,
            pending: self.pending,
        }
    }

    pub fn validate(&self) -> Result<(), SceneTransitionValidationError> {
        for text in [
            self.location,
            self.title,
            self.narration,
            self.continue_label,
        ] {
            if text.trim().is_empty() {
                return Err(SceneTransitionValidationError::EmptyText);
            }
            if text.len() > MAX_TEXT_BYTES {
                return Err(SceneTransitionValidationError::TextLimit);
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{cell::Cell, fmt, rc::Rc};

    use df_locale::VersionedFormattedMessage;
    use web_sys::{Document, Element, Node};

    use super::{SceneTransitionValidationError, SceneTransitionView};
    use crate::{ActionView, ControlError, ControlledAction, ThemeToken, UiError};

    const STYLES: &str = r#"
.df-scene-transition{container-type:inline-size;position:relative;isolation:isolate;overflow:hidden;background:var(--df-background);color:var(--df-text);font-family:Georgia,'Times New Roman',serif;min-width:0}
.df-scene-transition *{box-sizing:border-box;min-width:0}.df-scene-transition .transition-art{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;object-position:center;z-index:-3}
.df-scene-transition:before{content:'';position:absolute;inset:0;z-index:-2;background:linear-gradient(90deg,#11100dc9,transparent 70%),linear-gradient(180deg,#14131026,transparent 35%,#141310f2 100%)}
.df-scene-transition .transition-stage{min-height:min(92svh,960px);display:flex;flex-direction:column;justify-content:flex-end;align-items:flex-start;padding:clamp(32px,5cqw,80px);gap:18px;background:radial-gradient(ellipse at 10% 100%,#49391c40,transparent 65%)}
.df-scene-transition .transition-location{margin:0;font:11px/1.7 system-ui,sans-serif;letter-spacing:2.5px;text-transform:uppercase;color:var(--df-story);max-width:680px;overflow-wrap:anywhere}
.df-scene-transition .transition-location:before{content:'◇';margin-right:12px}.df-scene-transition .transition-title{margin:0;font-weight:400;font-size:clamp(38px,5cqw,76px);line-height:1.05;letter-spacing:-1.5px;max-width:760px;text-shadow:0 3px 30px #0008;overflow-wrap:anywhere}
.df-scene-transition .transition-narration{margin:0;max-width:670px;font-size:clamp(17px,1.7cqw,23px);line-height:1.65;color:var(--df-text);text-shadow:0 2px 12px #000;overflow-wrap:anywhere}
.df-scene-transition .transition-continue{margin-top:10px;min-height:48px;min-width:180px;padding:13px 28px;border:1px solid #e5c28c;border-radius:4px;background:linear-gradient(135deg,#e3c592,#b99151);color:#211b11;font:600 14px/1.5 system-ui,sans-serif;letter-spacing:.4px;cursor:pointer;touch-action:manipulation;max-width:100%;overflow-wrap:anywhere;box-shadow:0 8px 28px #9f6b3033,inset 0 1px 0 #fff6}
.df-scene-transition .transition-continue:hover:enabled{background:#edcea0}.df-scene-transition .transition-continue:disabled{background:#27231deb;color:var(--df-secondary);border-color:#bda16855;cursor:default;box-shadow:none}.df-scene-transition .transition-continue[aria-busy=true]{border-style:dashed}.df-scene-transition :focus-visible{outline:3px solid var(--df-story);outline-offset:5px}
@container(max-width:600px){.df-scene-transition .transition-stage{min-height:min(92svh,820px);padding:260px 24px 32px;gap:14px}.df-scene-transition .transition-art{object-position:58% center}.df-scene-transition .transition-title{font-size:38px;letter-spacing:-.7px}.df-scene-transition .transition-location{font-size:10px;letter-spacing:1.6px}.df-scene-transition .transition-narration{font-size:17px;line-height:1.6}.df-scene-transition .transition-continue{width:100%;margin-top:10px}.df-scene-transition .transition-stage{background:linear-gradient(180deg,transparent 20%,#14131026 40%,#141310d9 100%)}}
@container(max-width:360px){.df-scene-transition .transition-stage{padding:210px 18px 26px}.df-scene-transition .transition-title{font-size:33px}.df-scene-transition .transition-narration{font-size:16px}}
"#;

    #[derive(Debug)]
    pub enum SceneTransitionError {
        Dom(UiError),
        Control(ControlError),
        InvalidView(SceneTransitionValidationError),
    }

    impl fmt::Display for SceneTransitionError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Dom(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::InvalidView(error) => fmt::Display::fmt(error, formatter),
            }
        }
    }

    impl std::error::Error for SceneTransitionError {}

    impl From<wasm_bindgen::JsValue> for SceneTransitionError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(error))
        }
    }

    impl From<ControlError> for SceneTransitionError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }

    impl From<SceneTransitionValidationError> for SceneTransitionError {
        fn from(error: SceneTransitionValidationError) -> Self {
            Self::InvalidView(error)
        }
    }

    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, SceneTransitionError> {
        let child = document.create_element(tag)?;
        child.set_class_name(class);
        parent.append_child(&child)?;
        Ok(child)
    }

    fn erase_text(root: &Node) {
        let mut pending = vec![root.clone()];
        while let Some(node) = pending.pop() {
            let mut child = node.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
            node.set_node_value(Some(""));
        }
    }

    fn replace_text(element: &Element, value: &str) {
        erase_text(element);
        element.set_text_content(Some(value));
    }

    /// Stable local-art transition, with one owned native action listener.
    /// Text and Continue remain usable if the optional decorative image fails.
    pub struct SceneTransitionPhase {
        document: Document,
        root: Element,
        artwork: Element,
        location: Element,
        title: Element,
        narration: Element,
        continue_action: ControlledAction,
        visible: Rc<Cell<bool>>,
        offered: Cell<bool>,
        disposed: Cell<bool>,
    }

    impl SceneTransitionPhase {
        /// Mounts caller-formatted catalog output through the same literal text sink.
        pub fn create_localized<V: PartialEq>(
            document: &Document,
            view: &SceneTransitionView<'_>,
            expected_version: &V,
            messages: [&VersionedFormattedMessage<'_, V>; 4],
        ) -> Result<Self, SceneTransitionError> {
            let text = SceneTransitionView::catalog_text(expected_version, messages)?;
            Self::create(document, &view.with_catalog_text(&text))
        }

        /// Validates every catalog revision and all text before changing mounted nodes.
        /// Invalid catalog output preserves the previous complete view and focus.
        pub fn update_localized<V: PartialEq>(
            &self,
            view: &SceneTransitionView<'_>,
            expected_version: &V,
            messages: [&VersionedFormattedMessage<'_, V>; 4],
        ) -> Result<(), SceneTransitionError> {
            self.require_active()?;
            let text = SceneTransitionView::catalog_text(expected_version, messages)?;
            self.update(&view.with_catalog_text(&text))
        }

        pub fn create(
            document: &Document,
            view: &SceneTransitionView<'_>,
        ) -> Result<Self, SceneTransitionError> {
            view.validate()?;
            let root = document.create_element("section")?;
            root.set_class_name("df-scene-transition");
            root.set_attribute("aria-label", "Scene transition")?;
            root.set_attribute(
                "style",
                &format!(
                    "--df-background:{};--df-text:{};--df-secondary:{};--df-story:{}",
                    ThemeToken::Background.css_value(),
                    ThemeToken::Text.css_value(),
                    ThemeToken::SecondaryText.css_value(),
                    ThemeToken::StoryAccent.css_value(),
                ),
            )?;
            let style = child(document, &root, "style", "")?;
            style.set_text_content(Some(STYLES));
            let artwork = child(document, &root, "img", "transition-art")?;
            artwork.set_attribute("alt", "")?;
            let stage = child(document, &root, "div", "transition-stage")?;
            let location = child(document, &stage, "p", "transition-location")?;
            let title = child(document, &stage, "h1", "transition-title")?;
            let narration = child(document, &stage, "p", "transition-narration")?;
            let continue_action = ControlledAction::create(
                document,
                ActionView {
                    label: view.continue_label,
                    enabled: view.continue_enabled,
                    pending: view.pending,
                },
            )?;
            continue_action
                .element()
                .set_class_name("transition-continue");
            stage.append_child(continue_action.element())?;
            let phase = Self {
                document: document.clone(),
                root,
                artwork,
                location,
                title,
                narration,
                continue_action,
                visible: Rc::new(Cell::new(true)),
                offered: Cell::new(false),
                disposed: Cell::new(false),
            };
            phase.update(view)?;
            Ok(phase)
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Registers the sole callback. The owner decides how to submit or navigate.
        pub fn on_continue(
            &self,
            mut callback: impl FnMut() + 'static,
        ) -> Result<(), SceneTransitionError> {
            self.require_active()?;
            let document = self.document.clone();
            let visible = Rc::clone(&self.visible);
            self.continue_action.on_activate(move || {
                if visible.get() && !document.hidden() {
                    callback();
                }
            })?;
            Ok(())
        }

        /// Reconciles caller-admitted props in place; the Continue node keeps focus.
        pub fn update(&self, view: &SceneTransitionView<'_>) -> Result<(), SceneTransitionError> {
            self.require_active()?;
            view.validate()?;
            self.offered.set(false);
            self.continue_action.element().set_disabled(true);
            replace_text(&self.location, view.location);
            replace_text(&self.title, view.title);
            replace_text(&self.narration, view.narration);
            let source = view.scene.asset_path();
            if self.artwork.get_attribute("src").as_deref() != Some(source) {
                self.artwork.set_attribute("src", source)?;
            }
            erase_text(self.continue_action.element());
            self.continue_action.update(ActionView {
                label: view.continue_label,
                enabled: view.continue_enabled && self.visible.get(),
                pending: view.pending,
            })?;
            self.offered.set(view.continue_enabled && !view.pending);
            Ok(())
        }

        /// Suspension disables input without replacing content or establishing authority.
        pub fn set_visible(&self, visible: bool) -> Result<(), SceneTransitionError> {
            self.require_active()?;
            self.visible.set(visible);
            if !visible {
                self.offered.set(false);
            }
            self.continue_action
                .element()
                .set_disabled(!visible || !self.offered.get());
            Ok(())
        }

        fn require_active(&self) -> Result<(), SceneTransitionError> {
            if self.disposed.get() {
                Err(SceneTransitionValidationError::Disposed.into())
            } else {
                Ok(())
            }
        }

        /// Terminal and idempotent. Scrubs caller-held text and attributes in place.
        pub fn dispose(&self) -> Result<(), SceneTransitionError> {
            self.disposed.set(true);
            self.visible.set(false);
            self.offered.set(false);
            let mut failure = self.continue_action.dispose().err().map(Into::into);
            // The action is already detached; erase it separately for retained handles.
            for root in [self.continue_action.element().as_ref(), &self.root] {
                erase_text(root);
                let mut pending = vec![root.clone()];
                while let Some(element) = pending.pop() {
                    let mut child = element.first_element_child();
                    while let Some(current) = child {
                        child = current.next_element_sibling();
                        pending.push(current);
                    }
                    for name in element.get_attribute_names().iter() {
                        if let Some(name) = name.as_string()
                            && let Err(error) = element.remove_attribute(&name)
                            && failure.is_none()
                        {
                            failure = Some(SceneTransitionError::from(error));
                        }
                    }
                }
            }
            self.root.remove();
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    impl Drop for SceneTransitionPhase {
        fn drop(&mut self) {
            if self.dispose().is_err() {
                self.root.remove();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{SceneTransitionError, SceneTransitionPhase};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_rejects_empty_or_overlong_supplied_text() {
        let mut view = SceneTransitionView {
            scene: ConceptScene::SunkenHall,
            location: "Below Greyhaven",
            title: "The sunken halls",
            narration: "Torchlight catches the water beneath the archway.",
            continue_label: "Enter the halls",
            continue_enabled: true,
            pending: false,
        };
        assert_eq!(view.validate(), Ok(()));
        view.continue_label = "  ";
        assert_eq!(
            view.validate(),
            Err(SceneTransitionValidationError::EmptyText)
        );
        view.continue_label = "Enter the halls";
        let overlong = "x".repeat(MAX_TEXT_BYTES + 1);
        view.narration = &overlong;
        assert_eq!(
            view.validate(),
            Err(SceneTransitionValidationError::TextLimit)
        );
    }
}
