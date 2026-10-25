use std::{collections::BTreeSet, fmt};

/// Local navigation only. These categories confer no gameplay permission.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SheetTab {
    Character,
    Equipment,
    Spells,
    Journal,
    Progression,
}

impl SheetTab {
    pub const ALL: [Self; 5] = [
        Self::Character,
        Self::Equipment,
        Self::Spells,
        Self::Journal,
        Self::Progression,
    ];

    #[cfg(target_arch = "wasm32")]
    fn index(self) -> usize {
        match self {
            Self::Character => 0,
            Self::Equipment => 1,
            Self::Spells => 2,
            Self::Journal => 3,
            Self::Progression => 4,
        }
    }
}

/// An opaque presentation fence supplied by the private-view owner. The owner must
/// change it whenever member, run, binding, recovery epoch or access changes.
/// This token neither grants access nor replaces the canonical client's ViewStore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SheetOwnerGeneration(pub u64);

pub struct SheetField<'a> {
    pub label: &'a str,
    pub value: &'a str,
}

/// Exact current offer identity, supplied by the server-facing composition owner.
/// Enabled/pending are supplied observations, never derived from item/rules data.
pub struct SheetOffer<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub pending: bool,
}

/// A keyed expandable presentation row. Values and detail labels are already
/// formatted by the owner: ability modifiers, capacities, damage, level and reward
/// states are never calculated here. Every string is rendered as literal text.
pub struct SheetRow<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub value: &'a str,
    pub summary: &'a str,
    pub details: &'a [SheetField<'a>],
    pub offers: &'a [SheetOffer<'a>],
}

pub struct SheetSection<'a> {
    pub key: &'a str,
    pub tab: SheetTab,
    pub title: &'a str,
    pub caption: &'a str,
    pub rows: &'a [SheetRow<'a>],
}

/// Localized interface labels, separately supplied from character facts.
pub struct SheetLabels<'a> {
    pub tabs: [&'a str; 5],
    pub navigation: &'a str,
    pub filter: &'a str,
    pub no_matches: &'a str,
    pub art_fallback: &'a str,
}

/// Bounded, already audience-safe props, not a PlayerView, RPC, projection or
/// server model. Only accepted current views may be mapped here by df-player.
/// In particular, never pass checkpoint records and ask this component to redact.
pub struct CharacterSheetView<'a> {
    pub owner: SheetOwnerGeneration,
    pub revision: u64,
    pub name: &'a str,
    pub identity: &'a str,
    pub subtitle: &'a str,
    pub connection: &'a str,
    pub notice: &'a str,
    pub labels: SheetLabels<'a>,
    pub sections: &'a [SheetSection<'a>],
}

/// Callback intent retains the exact advertised ID and current presentation fence.
/// The caller performs typed command mapping, admission and authoritative validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetSubmission {
    pub owner: SheetOwnerGeneration,
    pub offer_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetValidationError {
    EmptyText,
    InvalidGeneration,
    DuplicateKey,
    Capacity,
}

impl fmt::Display for SheetValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyText => "sheet presentation contains empty text",
            Self::InvalidGeneration => "sheet presentation owner generation is invalid",
            Self::DuplicateKey => "sheet presentation identities repeat",
            Self::Capacity => "sheet presentation exceeds bounded capacity",
        })
    }
}
impl std::error::Error for SheetValidationError {}

impl CharacterSheetView<'_> {
    pub fn validate(&self) -> Result<(), SheetValidationError> {
        if self.owner.0 == 0 {
            return Err(SheetValidationError::InvalidGeneration);
        }
        if self.sections.len() > 24 {
            return Err(SheetValidationError::Capacity);
        }
        let mut bytes = 0usize;
        let mut charge = |value: &str| -> Result<(), SheetValidationError> {
            if value.trim().is_empty() {
                return Err(SheetValidationError::EmptyText);
            }
            bytes = bytes
                .checked_add(value.len())
                .ok_or(SheetValidationError::Capacity)?;
            if value.len() > 4096 || bytes > 65_536 {
                return Err(SheetValidationError::Capacity);
            }
            Ok(())
        };
        for value in [
            self.name,
            self.identity,
            self.subtitle,
            self.connection,
            self.notice,
            self.labels.navigation,
            self.labels.filter,
            self.labels.no_matches,
            self.labels.art_fallback,
        ] {
            charge(value)?;
        }
        for value in self.labels.tabs {
            charge(value)?;
        }
        let mut section_keys = BTreeSet::new();
        let mut row_keys = BTreeSet::new();
        let mut offers = BTreeSet::new();
        let mut row_count = 0usize;
        let mut offer_count = 0usize;
        for section in self.sections {
            if !section_keys.insert(section.key) {
                return Err(SheetValidationError::DuplicateKey);
            }
            for value in [section.key, section.title, section.caption] {
                charge(value)?;
            }
            row_count += section.rows.len();
            if row_count > 256 {
                return Err(SheetValidationError::Capacity);
            }
            for row in section.rows {
                if !row_keys.insert(row.key) {
                    return Err(SheetValidationError::DuplicateKey);
                }
                for value in [row.key, row.title, row.value, row.summary] {
                    charge(value)?;
                }
                if row.details.len() > 24 {
                    return Err(SheetValidationError::Capacity);
                }
                for field in row.details {
                    charge(field.label)?;
                    charge(field.value)?;
                }
                offer_count += row.offers.len();
                if offer_count > 64 {
                    return Err(SheetValidationError::Capacity);
                }
                for offer in row.offers {
                    if !offers.insert(offer.id) {
                        return Err(SheetValidationError::DuplicateKey);
                    }
                    charge(offer.id)?;
                    charge(offer.label)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        ActionView, ConceptScene, ControlError, ControlledAction, ControlledTextInput, DraftUpdate,
        InputFeedback, TextInputView, UiError,
    };
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{
        Document, Element, Event, EventTarget, HtmlButtonElement, HtmlElement, KeyboardEvent,
    };

    #[derive(Debug)]
    pub enum SheetError {
        InvalidView(SheetValidationError),
        Dom(UiError),
        Control(ControlError),
        InvalidMountId,
        OwnerChanged,
        StaleView,
        FocusUnavailable,
        Disposed,
    }
    impl fmt::Display for SheetError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(error) => fmt::Display::fmt(error, f),
                Self::Dom(error) => fmt::Display::fmt(error, f),
                Self::Control(error) => fmt::Display::fmt(error, f),
                Self::InvalidMountId => f.write_str("sheet mount identifier is invalid or in use"),
                Self::OwnerChanged => f.write_str("sheet private owner changed; scope disposed"),
                Self::StaleView => f.write_str("sheet presentation revision is stale"),
                Self::FocusUnavailable => f.write_str("sheet could not restore usable focus"),
                Self::Disposed => f.write_str("sheet scope is disposed"),
            }
        }
    }
    impl std::error::Error for SheetError {}
    impl From<wasm_bindgen::JsValue> for SheetError {
        fn from(value: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(value))
        }
    }
    impl From<ControlError> for SheetError {
        fn from(value: ControlError) -> Self {
            Self::Control(value)
        }
    }
    impl From<SheetValidationError> for SheetError {
        fn from(value: SheetValidationError) -> Self {
            Self::InvalidView(value)
        }
    }

    struct Listener {
        target: EventTarget,
        event: &'static str,
        callback: Option<Closure<dyn FnMut(Event)>>,
    }
    impl Listener {
        fn bind(
            target: EventTarget,
            event: &'static str,
            operation: impl FnMut(Event) + 'static,
        ) -> Result<Self, SheetError> {
            let callback = Closure::wrap(Box::new(operation) as Box<dyn FnMut(Event)>);
            target.add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())?;
            Ok(Self {
                target,
                event,
                callback: Some(callback),
            })
        }
        fn dispose(&mut self) -> Result<(), SheetError> {
            if let Some(callback) = self.callback.as_ref() {
                self.target.remove_event_listener_with_callback(
                    self.event,
                    callback.as_ref().unchecked_ref(),
                )?;
            }
            self.callback = None;
            Ok(())
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            if self.dispose().is_err()
                && let Some(callback) = self.callback.take()
            {
                callback.forget();
            }
        }
    }

    struct FilterRow {
        root: Element,
        tab: SheetTab,
        text: String,
    }
    struct LocalPresentation {
        active: bool,
        tab: SheetTab,
        filter: String,
        buttons: Vec<HtmlButtonElement>,
        panels: Vec<Element>,
        rows: BTreeMap<String, FilterRow>,
        empty: Element,
    }
    impl LocalPresentation {
        fn apply(&self) -> Result<(), SheetError> {
            if !self.active {
                return Ok(());
            }
            for (index, button) in self.buttons.iter().enumerate() {
                let selected = index == self.tab.index();
                button.set_attribute("aria-selected", if selected { "true" } else { "false" })?;
                button.set_tab_index(if selected { 0 } else { -1 });
            }
            for (index, panel) in self.panels.iter().enumerate() {
                set_hidden(panel, index != self.tab.index())?;
            }
            let query = self.filter.trim().to_lowercase();
            let mut found = false;
            for row in self.rows.values() {
                let matches = query.is_empty() || row.text.contains(&query);
                set_hidden(&row.root, !matches)?;
                if matches && row.tab == self.tab {
                    found = true;
                }
            }
            set_hidden(&self.empty, found)?;
            Ok(())
        }
        fn clear(&mut self) {
            self.active = false;
            self.filter.clear();
            self.rows.clear();
        }
    }
    type Submit = Box<dyn FnMut(SheetSubmission)>;
    struct OfferFence {
        owner: SheetOwnerGeneration,
        revision: u64,
        active: bool,
        offers: BTreeMap<String, bool>,
        submit: Option<Submit>,
    }
    struct RowNodes {
        root: Element,
        title: Element,
        value: Element,
        summary: Element,
        fields: Element,
        actions: Element,
        offers: BTreeMap<String, ControlledAction>,
    }
    struct SectionNodes {
        root: Element,
        title: Element,
        caption: Element,
        rows_root: Element,
        rows: BTreeMap<String, RowNodes>,
    }

    /// One mounted private sheet. Keyed summaries, native details, tab controls and
    /// the search field survive ordinary updates. No timers, async tasks, network,
    /// rules resolution, inventory mutations or optimistic payout occur here.
    pub struct CharacterSheetSurface {
        document: Document,
        root: Element,
        name: Element,
        identity: Element,
        subtitle: Element,
        connection: Element,
        notice: Element,
        art_fallback: Element,
        tabs: Vec<ControlledAction>,
        filter: ControlledTextInput,
        local: Rc<RefCell<LocalPresentation>>,
        fence: Rc<RefCell<OfferFence>>,
        sections: RefCell<BTreeMap<String, SectionNodes>>,
        listeners: RefCell<Vec<Listener>>,
    }
    fn node(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, SheetError> {
        let element = document.create_element(tag)?;
        element.set_class_name(class);
        element.set_text_content(text);
        parent.append_child(&element)?;
        Ok(element)
    }
    fn scrub(node: &web_sys::Node) {
        // Visit every Node, including Text and Comment, before detaching children.
        // Retained DOM references must not preserve superseded private text.
        let mut next = node.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            scrub(&child);
        }
        node.set_node_value(Some(""));
        node.set_text_content(None);
    }
    fn literal(element: &Element, text: Option<&str>) {
        // Text-only leaves keep their node identity on ordinary updates. Existing
        // children are sanitized before any replacement can detach them.
        if let Some(child) = element.first_child()
            && child.node_type() == web_sys::Node::TEXT_NODE
            && child.next_sibling().is_none()
            && let Some(text) = text
        {
            child.set_node_value(Some(text));
            return;
        }
        scrub(element);
        element.set_text_content(text);
    }
    fn set_hidden(element: &Element, is_hidden: bool) -> Result<(), SheetError> {
        if is_hidden {
            element.set_attribute("hidden", "")?;
        } else {
            element.remove_attribute("hidden")?;
        }
        Ok(())
    }
    fn ordered(
        parent: &Element,
        child: &Element,
        cursor: &mut Option<Element>,
    ) -> Result<(), SheetError> {
        // A keyed section may have moved to another panel earlier in this update.
        // An insert-before reference must still belong to this parent.
        if cursor.as_ref().is_some_and(|next| {
            !next
                .parent_node()
                .is_some_and(|owner| owner.is_same_node(Some(parent)))
        }) {
            *cursor = parent.first_element_child();
        }
        if !cursor
            .as_ref()
            .is_some_and(|next| child.is_same_node(Some(next)))
        {
            parent.insert_before(child, cursor.as_ref().map(|node| node.as_ref()))?;
        }
        *cursor = child.next_element_sibling();
        Ok(())
    }
    impl CharacterSheetSurface {
        pub fn create(
            document: &Document,
            mount_id: &str,
            view: &CharacterSheetView<'_>,
            submit: impl FnMut(SheetSubmission) + 'static,
        ) -> Result<Self, SheetError> {
            view.validate()?;
            if mount_id.is_empty()
                || mount_id.len() > 48
                || !mount_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                || document.get_element_by_id(mount_id).is_some()
            {
                return Err(SheetError::InvalidMountId);
            }
            let root = document.create_element("section")?;
            root.set_id(mount_id);
            root.set_class_name("df-sheet");
            node(
                document,
                &root,
                "style",
                "",
                Some(crate::character_sheet_phase_theme::STYLES),
            )?;
            let art = node(document, &root, "img", "sheet-art", None)?;
            art.set_attribute("src", ConceptScene::Campfire.asset_path())?;
            art.set_attribute("alt", "")?;
            art.set_attribute("aria-hidden", "true")?;
            let header = node(document, &root, "header", "sheet-header", None)?;
            node(document, &header, "div", "sheet-brand", Some("DungeonFlux"))?;
            let connection = node(document, &header, "p", "sheet-connection", None)?;
            let hero = node(document, &root, "div", "sheet-hero", None)?;
            let identity = node(document, &hero, "p", "sheet-identity", None)?;
            let name = node(document, &hero, "h1", "", None)?;
            let subtitle = node(document, &hero, "p", "sheet-subtitle", None)?;
            let art_fallback = node(document, &hero, "p", "sheet-art-fallback", None)?;
            set_hidden(&art_fallback, true)?;
            let notice = node(document, &root, "p", "sheet-notice", None)?;
            notice.set_attribute("role", "status")?;
            let workspace = node(document, &root, "div", "sheet-workspace", None)?;
            let navigation = node(document, &workspace, "div", "sheet-tabs", None)?;
            navigation.set_attribute("role", "tablist")?;
            let filter = ControlledTextInput::create(
                document,
                &format!("{mount_id}-filter"),
                TextInputView {
                    label: view.labels.filter,
                    enabled: true,
                    feedback: InputFeedback::None,
                },
                "",
            )?;
            filter.root().set_class_name("sheet-filter");
            workspace.append_child(filter.root())?;
            let content = node(document, &workspace, "div", "sheet-content", None)?;
            let mut tabs = Vec::new();
            let mut panels = Vec::new();
            for tab in SheetTab::ALL {
                let id = format!("{mount_id}-tab-{}", tab.index());
                let panel_id = format!("{mount_id}-panel-{}", tab.index());
                let button = ControlledAction::create(
                    document,
                    ActionView {
                        label: view.labels.tabs[tab.index()],
                        enabled: true,
                        pending: false,
                    },
                )?;
                button.element().set_id(&id);
                button.element().set_attribute("role", "tab")?;
                button.element().set_attribute("aria-controls", &panel_id)?;
                navigation.append_child(button.element())?;
                let panel = node(document, &content, "div", "sheet-tab-panel", None)?;
                panel.set_id(&panel_id);
                panel.set_attribute("role", "tabpanel")?;
                panel.set_attribute("aria-labelledby", &id)?;
                panel.set_attribute("tabindex", "0")?;
                panels.push(panel);
                tabs.push(button);
            }
            let empty = node(document, &content, "p", "sheet-empty", None)?;
            let local = Rc::new(RefCell::new(LocalPresentation {
                active: true,
                tab: SheetTab::Character,
                filter: String::new(),
                buttons: tabs.iter().map(|tab| tab.element().clone()).collect(),
                panels,
                rows: BTreeMap::new(),
                empty,
            }));
            for (index, tab) in tabs.iter().enumerate() {
                let presentation = Rc::clone(&local);
                let error_notice = notice.clone();
                tab.on_activate(move || {
                    let mut presentation = presentation.borrow_mut();
                    if presentation.active
                        && let Some(tab) = SheetTab::ALL.get(index)
                    {
                        presentation.tab = *tab;
                        if presentation.apply().is_err() {
                            literal(&error_notice, Some("Sheet navigation failed"));
                        }
                    }
                })?;
            }
            let presentation = Rc::clone(&local);
            let filter_notice = notice.clone();
            filter.on_change(move |draft| {
                let mut presentation = presentation.borrow_mut();
                if !presentation.active {
                    return;
                }
                match draft {
                    Ok(value) => {
                        presentation.filter = value;
                        if presentation.apply().is_err() {
                            literal(&filter_notice, Some("Sheet filter failed"));
                        }
                    }
                    Err(_) => literal(&filter_notice, Some("Sheet filter is too long")),
                }
            })?;
            let presentation = Rc::clone(&local);
            let key_notice = notice.clone();
            let keyboard = Listener::bind(navigation.clone().into(), "keydown", move |event| {
                let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                    return;
                };
                let mut presentation = presentation.borrow_mut();
                if !presentation.active {
                    return;
                }
                let active_tab_index = presentation.tab.index();
                let next_tab_index = match event.key().as_str() {
                    "ArrowRight" => (active_tab_index + 1) % 5,
                    "ArrowLeft" => (active_tab_index + 4) % 5,
                    "Home" => 0,
                    "End" => 4,
                    _ => return,
                };
                event.prevent_default();
                if let Some(tab) = SheetTab::ALL.get(next_tab_index) {
                    presentation.tab = *tab;
                }
                let navigation_result = presentation.apply().and_then(|()| {
                    if let Some(button) = presentation.buttons.get(next_tab_index) {
                        button.focus()?;
                    }
                    Ok(())
                });
                if navigation_result.is_err() {
                    literal(&key_notice, Some("Sheet navigation failed"));
                }
            })?;
            let fallback = art_fallback.clone();
            let art_node = art.clone();
            let presentation = Rc::clone(&local);
            let art_error = Listener::bind(art.into(), "error", move |_| {
                if presentation.borrow().active {
                    // The flat ink-and-gold layout is usable without optional artwork.
                    scrub(&art_node);
                    if set_hidden(&art_node, true).is_err() || set_hidden(&fallback, false).is_err()
                    {
                        literal(&fallback, Some("Optional artwork unavailable"));
                    }
                }
            })?;
            let surface = Self {
                document: document.clone(),
                root,
                name,
                identity,
                subtitle,
                connection,
                notice,
                art_fallback,
                tabs,
                filter,
                local,
                fence: Rc::new(RefCell::new(OfferFence {
                    owner: view.owner,
                    revision: view.revision,
                    active: true,
                    offers: BTreeMap::new(),
                    submit: Some(Box::new(submit)),
                })),
                sections: RefCell::new(BTreeMap::new()),
                listeners: RefCell::new(vec![keyboard, art_error]),
            };
            navigation.set_attribute("aria-label", view.labels.navigation)?;
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Owner changes invalidate and scrub the complete private scope. The outer
        /// application remains mounted and may create the replacement private sheet.
        pub fn update(&self, view: &CharacterSheetView<'_>) -> Result<(), SheetError> {
            {
                let fence = self.fence.borrow();
                if !fence.active {
                    return Err(SheetError::Disposed);
                }
                if fence.owner != view.owner {
                    drop(fence);
                    self.dispose()?;
                    return Err(SheetError::OwnerChanged);
                }
                if view.revision < fence.revision {
                    return Err(SheetError::StaleView);
                }
            }
            if let Err(error) = view.validate() {
                self.dispose()?;
                return Err(error.into());
            }
            // Close the offer fence before any DOM reconciliation. A partial render
            // never leaves an older action eligible for submission.
            self.fence.borrow_mut().offers.clear();
            let focused = self
                .document
                .active_element()
                .filter(|element| self.root.contains(Some(element.as_ref())));
            if let Err(error) = self.render(view) {
                self.dispose()?;
                return Err(error);
            }
            if let Err(error) = self.restore_focus(focused) {
                self.dispose()?;
                return Err(error);
            }
            let mut fence = self.fence.borrow_mut();
            fence.revision = view.revision;
            for section in view.sections {
                for row in section.rows {
                    for offer in row.offers {
                        fence
                            .offers
                            .insert(offer.id.to_owned(), offer.enabled && !offer.pending);
                    }
                }
            }
            Ok(())
        }
        fn focus_usable(&self, target: &Element) -> bool {
            if !self.root.contains(Some(target)) {
                return false;
            }
            let mut ancestor = Some(target.clone());
            while let Some(element) = ancestor {
                if element.has_attribute("hidden")
                    || element.has_attribute("disabled")
                    || element.get_attribute("aria-disabled").as_deref() == Some("true")
                    || element.get_attribute("aria-hidden").as_deref() == Some("true")
                {
                    return false;
                }
                if element.is_same_node(Some(&self.root)) {
                    return true;
                }
                ancestor = element.parent_element();
            }
            false
        }
        fn restore_focus(&self, focused: Option<Element>) -> Result<(), SheetError> {
            let Some(before) = focused else {
                return Ok(());
            };
            if self.focus_usable(&before) {
                if self
                    .document
                    .active_element()
                    .as_ref()
                    .is_some_and(|after| before.is_same_node(Some(after)))
                {
                    return Ok(());
                }
                if let Some(element) = before.dyn_ref::<HtmlElement>() {
                    element.focus()?;
                    if self
                        .document
                        .active_element()
                        .as_ref()
                        .is_some_and(|after| before.is_same_node(Some(after)))
                    {
                        return Ok(());
                    }
                }
            }
            let button = {
                let local = self.local.borrow();
                local.buttons.get(local.tab.index()).cloned()
            }
            .ok_or(SheetError::FocusUnavailable)?;
            button.focus()?;
            if self
                .document
                .active_element()
                .as_ref()
                .is_some_and(|after| button.is_same_node(Some(after)))
            {
                Ok(())
            } else {
                Err(SheetError::FocusUnavailable)
            }
        }
        fn render(&self, view: &CharacterSheetView<'_>) -> Result<(), SheetError> {
            for (node, text) in [
                (&self.name, view.name),
                (&self.identity, view.identity),
                (&self.subtitle, view.subtitle),
                (&self.connection, view.connection),
                (&self.notice, view.notice),
                (&self.art_fallback, view.labels.art_fallback),
            ] {
                literal(node, Some(text));
            }
            self.root
                .set_attribute("data-sheet-revision", &view.revision.to_string())?;
            if let Some(navigation) = self.root.query_selector(".sheet-tabs")? {
                navigation.set_attribute("aria-label", view.labels.navigation)?;
            }
            for (index, tab) in self.tabs.iter().enumerate() {
                if let Some(label) = view.labels.tabs.get(index) {
                    scrub(tab.element());
                    tab.update(ActionView {
                        label,
                        enabled: true,
                        pending: false,
                    })?;
                }
            }
            if let Some(caption) = self.filter.root().query_selector("label span")? {
                scrub(&caption);
            }
            if let Some(feedback) = self.filter.root().query_selector(".df-ui-feedback")? {
                scrub(&feedback);
            }
            self.filter.update(
                TextInputView {
                    label: view.labels.filter,
                    enabled: true,
                    feedback: InputFeedback::None,
                },
                DraftUpdate::Preserve,
            )?;
            let mut mounted = self.sections.borrow_mut();
            let obsolete: Vec<_> = mounted
                .keys()
                .filter(|key| {
                    !view
                        .sections
                        .iter()
                        .any(|section| section.key == key.as_str())
                })
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(mut section) = mounted.remove(&key) {
                    clear_section(&mut section)?;
                }
            }
            self.local.borrow_mut().rows.clear();
            let panels = self.local.borrow().panels.clone();
            let mut cursors: Vec<_> = panels.iter().map(Element::first_element_child).collect();
            for section in view.sections {
                let Some(parent) = panels.get(section.tab.index()) else {
                    continue;
                };
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    mounted.entry(section.key.to_owned())
                {
                    let root = self.document.create_element("section")?;
                    root.set_class_name("sheet-section");
                    let title = node(&self.document, &root, "h2", "", None)?;
                    let caption = node(&self.document, &root, "p", "sheet-caption", None)?;
                    let rows_root = node(&self.document, &root, "div", "sheet-rows", None)?;
                    entry.insert(SectionNodes {
                        root,
                        title,
                        caption,
                        rows_root,
                        rows: BTreeMap::new(),
                    });
                }
                if let Some(nodes) = mounted.get_mut(section.key) {
                    nodes
                        .root
                        .set_attribute("data-sheet-tab", &section.tab.index().to_string())?;
                    literal(&nodes.title, Some(section.title));
                    literal(&nodes.caption, Some(section.caption));
                    let obsolete: Vec<_> = nodes
                        .rows
                        .keys()
                        .filter(|key| !section.rows.iter().any(|row| row.key == key.as_str()))
                        .cloned()
                        .collect();
                    for key in obsolete {
                        if let Some(mut row) = nodes.rows.remove(&key) {
                            clear_row(&mut row)?;
                        }
                    }
                    let mut cursor = nodes.rows_root.first_element_child();
                    for row in section.rows {
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            nodes.rows.entry(row.key.to_owned())
                        {
                            entry.insert(self.create_row()?);
                        }
                        if let Some(row_nodes) = nodes.rows.get_mut(row.key) {
                            self.update_row(row_nodes, row)?;
                            ordered(&nodes.rows_root, &row_nodes.root, &mut cursor)?;
                            let mut text = format!("{} {} {}", row.title, row.value, row.summary);
                            for field in row.details {
                                text.push_str(&format!(" {} {}", field.label, field.value));
                            }
                            self.local.borrow_mut().rows.insert(
                                row.key.to_owned(),
                                FilterRow {
                                    root: row_nodes.root.clone(),
                                    tab: section.tab,
                                    text: text.to_lowercase(),
                                },
                            );
                        }
                    }
                    if let Some(cursor) = cursors.get_mut(section.tab.index()) {
                        ordered(parent, &nodes.root, cursor)?;
                    }
                }
            }
            literal(&self.local.borrow().empty, Some(view.labels.no_matches));
            self.local.borrow().apply()?;
            Ok(())
        }
        fn create_row(&self) -> Result<RowNodes, SheetError> {
            let root = self.document.create_element("details")?;
            root.set_class_name("sheet-row");
            let heading = node(&self.document, &root, "summary", "sheet-row-heading", None)?;
            let title = node(&self.document, &heading, "span", "sheet-row-title", None)?;
            let value = node(&self.document, &heading, "span", "sheet-row-value", None)?;
            let content = node(&self.document, &root, "div", "sheet-row-body", None)?;
            let summary = node(&self.document, &content, "p", "sheet-row-summary", None)?;
            let fields = node(&self.document, &content, "dl", "sheet-fields", None)?;
            let actions = node(&self.document, &content, "div", "sheet-actions", None)?;
            Ok(RowNodes {
                root,
                title,
                value,
                summary,
                fields,
                actions,
                offers: BTreeMap::new(),
            })
        }
        fn update_row(&self, nodes: &mut RowNodes, row: &SheetRow<'_>) -> Result<(), SheetError> {
            literal(&nodes.title, Some(row.title));
            literal(&nodes.value, Some(row.value));
            literal(&nodes.summary, Some(row.summary));
            nodes.root.set_attribute("data-sheet-row", row.key)?;
            scrub(&nodes.fields);
            for field in row.details {
                node(&self.document, &nodes.fields, "dt", "", Some(field.label))?;
                node(&self.document, &nodes.fields, "dd", "", Some(field.value))?;
            }
            let obsolete: Vec<_> = nodes
                .offers
                .keys()
                .filter(|id| !row.offers.iter().any(|offer| offer.id == id.as_str()))
                .cloned()
                .collect();
            for id in obsolete {
                if let Some(control) = nodes.offers.remove(&id) {
                    scrub(control.element());
                    control.element().remove_attribute("data-sheet-offer")?;
                    control.dispose()?;
                }
            }
            let mut cursor = nodes.actions.first_element_child();
            for offer in row.offers {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    nodes.offers.entry(offer.id.to_owned())
                {
                    let control = ControlledAction::create(
                        &self.document,
                        ActionView {
                            label: offer.label,
                            enabled: offer.enabled,
                            pending: offer.pending,
                        },
                    )?;
                    control
                        .element()
                        .set_attribute("data-sheet-offer", offer.id)?;
                    let id = offer.id.to_owned();
                    let fence = Rc::clone(&self.fence);
                    control.on_activate(move || {
                        let (owner, callback) = {
                            let mut state = fence.borrow_mut();
                            if !state.active || state.offers.get(&id) != Some(&true) {
                                return;
                            }
                            (state.owner, state.submit.take())
                        };
                        if let Some(mut callback) = callback {
                            callback(SheetSubmission {
                                owner,
                                offer_id: id.clone(),
                            });
                            let mut state = fence.borrow_mut();
                            if state.active {
                                state.submit = Some(callback);
                            }
                        }
                    })?;
                    entry.insert(control);
                }
                if let Some(control) = nodes.offers.get(offer.id) {
                    scrub(control.element());
                    control.update(ActionView {
                        label: offer.label,
                        enabled: offer.enabled,
                        pending: offer.pending,
                    })?;
                    ordered(&nodes.actions, control.element().as_ref(), &mut cursor)?;
                }
            }
            Ok(())
        }
        /// Idempotent; gates callbacks and scrubs private text/filter state before
        /// detaching listeners and DOM. Drop also fences every owned callback.
        pub fn dispose(&self) -> Result<(), SheetError> {
            {
                let mut fence = self.fence.borrow_mut();
                fence.active = false;
                fence.offers.clear();
                fence.submit.take();
            }
            self.local.borrow_mut().clear();
            scrub(&self.root);
            let mut first_error = None;
            for listener in self.listeners.borrow_mut().iter_mut() {
                if let Err(error) = listener.dispose() {
                    first_error.get_or_insert(error);
                }
            }
            for control in &self.tabs {
                if let Err(error) = control.dispose() {
                    first_error.get_or_insert(error.into());
                }
            }
            if let Err(error) = self.filter.dispose() {
                first_error.get_or_insert(error.into());
            }
            for section in self.sections.borrow_mut().values_mut() {
                if let Err(error) = clear_section(section) {
                    first_error.get_or_insert(error);
                }
            }
            self.sections.borrow_mut().clear();
            if let Some(parent) = self.root.parent_node()
                && let Err(error) = parent.remove_child(&self.root)
            {
                first_error.get_or_insert(error.into());
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    fn clear_row(row: &mut RowNodes) -> Result<(), SheetError> {
        scrub(&row.root);
        let mut first_error = None;
        if let Err(error) = row.root.remove_attribute("data-sheet-row") {
            first_error.get_or_insert(error.into());
        }
        for control in row.offers.values() {
            scrub(control.element());
            if let Err(error) = control.element().remove_attribute("data-sheet-offer") {
                first_error.get_or_insert(error.into());
            }
            if let Err(error) = control.dispose() {
                first_error.get_or_insert(SheetError::from(error));
            }
        }
        row.offers.clear();
        if let Some(parent) = row.root.parent_node()
            && let Err(error) = parent.remove_child(&row.root)
        {
            first_error.get_or_insert(error.into());
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn clear_section(section: &mut SectionNodes) -> Result<(), SheetError> {
        let mut first_error = None;
        for row in section.rows.values_mut() {
            if let Err(error) = clear_row(row) {
                first_error.get_or_insert(error);
            }
        }
        section.rows.clear();
        scrub(&section.root);
        if let Some(parent) = section.root.parent_node()
            && let Err(error) = parent.remove_child(&section.root)
        {
            first_error.get_or_insert(error.into());
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    impl Drop for CharacterSheetSurface {
        fn drop(&mut self) {
            // Explicit dispose reports DOM errors. Drop still scrubs all retained
            // private values; owned controls/listeners fence their own callbacks.
            if self.dispose().is_err() {
                self.fence.borrow_mut().submit.take();
                self.local.borrow_mut().clear();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{CharacterSheetSurface, SheetError};

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(sections: &'a [SheetSection<'a>]) -> CharacterSheetView<'a> {
        CharacterSheetView {
            owner: SheetOwnerGeneration(1),
            revision: 1,
            name: "Vell",
            identity: "Ranger · Level supplied by server",
            subtitle: "Own character",
            connection: "Connected",
            notice: "Current permitted view",
            labels: SheetLabels {
                tabs: ["Character", "Equipment", "Spells", "Journal", "Progression"],
                navigation: "Sheet",
                filter: "Filter",
                no_matches: "No matching entries",
                art_fallback: "Artwork unavailable",
            },
            sections,
        }
    }
    #[test]
    fn accepts_literal_markup_without_interpreting_game_values() {
        let rows = [SheetRow {
            key: "ability",
            title: "<b>Ability</b>",
            value: "Server +99",
            summary: "<img onerror=x>",
            details: &[],
            offers: &[],
        }];
        let sections = [SheetSection {
            key: "stats",
            tab: SheetTab::Character,
            title: "Abilities",
            caption: "Supplied values",
            rows: &rows,
        }];
        assert_eq!(view(&sections).validate(), Ok(()));
    }
    #[test]
    fn refuses_ambiguous_offer_identity_across_rows() {
        let offer = [SheetOffer {
            id: "exact-id",
            label: "Equip",
            enabled: true,
            pending: false,
        }];
        let rows = [
            SheetRow {
                key: "one",
                title: "One",
                value: "Stored",
                summary: "Supplied",
                details: &[],
                offers: &offer,
            },
            SheetRow {
                key: "two",
                title: "Two",
                value: "Stored",
                summary: "Supplied",
                details: &[],
                offers: &offer,
            },
        ];
        let sections = [SheetSection {
            key: "items",
            tab: SheetTab::Equipment,
            title: "Items",
            caption: "Current equipment",
            rows: &rows,
        }];
        assert_eq!(
            view(&sections).validate(),
            Err(SheetValidationError::DuplicateKey)
        );
    }
    #[test]
    fn bounds_whole_view_and_requires_owned_generation() {
        let mut current = view(&[]);
        current.owner = SheetOwnerGeneration(0);
        assert_eq!(
            current.validate(),
            Err(SheetValidationError::InvalidGeneration)
        );
        current.owner = SheetOwnerGeneration(1);
        let oversized = "x".repeat(4097);
        current.name = &oversized;
        assert_eq!(current.validate(), Err(SheetValidationError::Capacity));
    }
}
