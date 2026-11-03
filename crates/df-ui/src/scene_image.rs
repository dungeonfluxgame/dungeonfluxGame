use df_client::cache::{CacheError, CacheKey, CacheLimits, CacheScope};
use df_types::SessionRevision;

/// Complete already permitted caller references; identities are correlation, not access.
#[derive(Clone, Copy)]
pub struct CampaignSceneAssets<'a> {
    pub scope: CacheScope,
    pub revision: SessionRevision,
    pub references: &'a [CacheKey],
    pub selected: Option<&'a CacheKey>,
}

/// Finite encoded and decoded image bounds selected by the mounted owner.
#[derive(Clone, Copy, Debug)]
pub struct SceneImageLimits {
    pub cache: CacheLimits,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneImageError {
    InvalidLimits,
    NotEnabled,
    NotSelected,
    InvalidPng,
    Dimensions,
    Cache(CacheError),
    Decode(df_render::ImageDecodeError),
    Resource(df_render::ResourceLifecycleError),
}

impl SceneImageLimits {
    pub fn validate(self) -> Result<(), SceneImageError> {
        if self.max_width == 0
            || self.max_height == 0
            || self.max_pixels == 0
            || self.cache.max_assets == 0
            || self.cache.max_pending == 0
            || self.cache.max_leases == 0
            || self.cache.max_bytes == 0
        {
            return Err(SceneImageError::InvalidLimits);
        }
        self.capacities()?;
        Ok(())
    }

    // One selected illustration, one real codec operation. Checked ceilings include
    // canonical input/browser copies, RGBA staging and the persistent presentation surface.
    fn capacities(self) -> Result<(usize, usize, usize), SceneImageError> {
        let pixels = u64::from(self.max_width)
            .checked_mul(u64::from(self.max_height))
            .ok_or(SceneImageError::InvalidLimits)?
            .min(self.max_pixels);
        let decoded = usize::try_from(pixels)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(SceneImageError::InvalidLimits)?;
        let scanline = usize::try_from(self.max_width)
            .ok()
            .and_then(|width| width.checked_mul(8))
            .and_then(|row| row.checked_add(7))
            .and_then(|row| {
                usize::try_from(self.max_height)
                    .ok()
                    .and_then(|height| row.checked_mul(height))
            })
            .ok_or(SceneImageError::InvalidLimits)?;
        let work = self
            .cache
            .max_bytes
            .checked_mul(3)
            .and_then(|input| {
                decoded
                    .checked_mul(5)
                    .and_then(|rgba| input.checked_add(rgba))
            })
            .and_then(|sum| sum.checked_add(scanline))
            .ok_or(SceneImageError::InvalidLimits)?;
        let surface = decoded
            .checked_mul(2)
            .ok_or(SceneImageError::InvalidLimits)?;
        Ok((decoded, work, surface))
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn prepared_layout(self, bytes: &[u8]) -> Result<((u32, u32), &'static str), SceneImageError> {
        if bytes.get(..8) == Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
            return self
                .dimensions(bytes)
                .map(|dimensions| (dimensions, "image/png"));
        }
        // Only the canonical decoder's existing single static VP8 keyframe is added.
        // The header supplies layout, never rights or a caller-controlled MIME contract.
        if bytes.get(..4) != Some(b"RIFF".as_slice())
            || bytes.get(8..12) != Some(b"WEBP".as_slice())
        {
            return Err(SceneImageError::Decode(
                df_render::ImageDecodeError::UnsupportedMime,
            ));
        }
        if bytes.get(12..16) != Some(b"VP8 ".as_slice()) {
            return Err(SceneImageError::Decode(
                df_render::ImageDecodeError::UnsupportedWebp,
            ));
        }
        let width = bytes
            .get(26..28)
            .and_then(|value| value.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or(SceneImageError::Decode(
                df_render::ImageDecodeError::CorruptWebp,
            ))?;
        let height = bytes
            .get(28..30)
            .and_then(|value| value.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or(SceneImageError::Decode(
                df_render::ImageDecodeError::CorruptWebp,
            ))?;
        let (decoded, work, _) = self.capacities()?;
        let plan = df_render::inspect_prepared_image(
            bytes,
            df_render::PreparedImageMetadata {
                mime: "image/webp",
                width: u32::from(width),
                height: u32::from(height),
                max_ancillary_bytes: self.cache.max_bytes,
            },
            df_render::ImageDecodeLimits {
                max_encoded_bytes: self.cache.max_bytes,
                max_dimension: self.max_width.max(self.max_height),
                max_decoded_bytes: decoded,
                max_work_bytes: work,
            },
        )
        .map_err(|error| match error {
            df_render::ImageDecodeError::DimensionCapacity => SceneImageError::Dimensions,
            error => SceneImageError::Decode(error),
        })?;
        if plan.width > self.max_width
            || plan.height > self.max_height
            || u64::from(plan.width) * u64::from(plan.height) > self.max_pixels
        {
            return Err(SceneImageError::Dimensions);
        }
        Ok(((plan.width, plan.height), "image/webp"))
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn dimensions(self, bytes: &[u8]) -> Result<(u32, u32), SceneImageError> {
        // IHDR is fixed size. Validate dimensions before any browser image decoder sees bytes.
        if bytes.len() < 33
            || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice())
            || bytes.get(8..12) != Some([0, 0, 0, 13].as_slice())
            || bytes.get(12..16) != Some(b"IHDR".as_slice())
        {
            return Err(SceneImageError::InvalidPng);
        }
        let width = u32::from_be_bytes(
            bytes
                .get(16..20)
                .and_then(|v| v.try_into().ok())
                .ok_or(SceneImageError::InvalidPng)?,
        );
        let height = u32::from_be_bytes(
            bytes
                .get(20..24)
                .and_then(|v| v.try_into().ok())
                .ok_or(SceneImageError::InvalidPng)?,
        );
        if width == 0
            || height == 0
            || width > self.max_width
            || height > self.max_height
            || u64::from(width) * u64::from(height) > self.max_pixels
        {
            return Err(SceneImageError::Dimensions);
        }
        let valid_depth = matches!(
            (bytes.get(25), bytes.get(24)),
            (Some(0), Some(1 | 2 | 4 | 8 | 16))
                | (Some(2 | 4 | 6), Some(8 | 16))
                | (Some(3), Some(1 | 2 | 4 | 8))
        );
        if !valid_depth
            || bytes.get(26) != Some(&0)
            || bytes.get(27) != Some(&0)
            || !matches!(bytes.get(28), Some(0 | 1))
        {
            return Err(SceneImageError::InvalidPng);
        }
        Ok((width, height))
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod browser {
    use super::*;
    use crate::CampaignError;
    use df_client::cache::FetchToken;
    use df_render::{
        BrowserImageDecode, ImageDecodeError, ImageDecodeLimits, PreparedImageMetadata,
        ResourceError, ResourceLifecycle, ResourceLifecycleError, ResourceLimits,
    };
    use df_types::RevisionLabel;
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, closure::Closure};
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{Element, HtmlImageElement, MutationObserver, MutationObserverInit};

    // Presentation policy selected by this owner, not a provider timeout or measured device cap.
    const DECODE_DEADLINE_MS: i32 = 10_000;

    struct MountWatch {
        observer: MutationObserver,
        _callback: Closure<dyn FnMut(js_sys::Array, MutationObserver)>,
    }
    impl Drop for MountWatch {
        fn drop(&mut self) {
            self.observer.disconnect();
        }
    }
    struct ActiveDecode {
        identity: Rc<()>,
        key: CacheKey,
    }
    struct Layout {
        key: CacheKey,
        mime: &'static str,
        dimensions: (u32, u32),
    }

    /// One canonical lifecycle owns encoded bytes, exact leases, decoded handles and
    /// actual codec reservations. This component owns selection and fallback only.
    pub(crate) struct SceneImageOwner {
        lifecycle: Rc<RefCell<ResourceLifecycle>>,
        scope: CacheScope,
        limits: SceneImageLimits,
        decode_limits: ImageDecodeLimits,
        revision: Option<SessionRevision>,
        references: Box<[CacheKey]>,
        selected: Option<CacheKey>,
        selection: Rc<()>,
        requested: Option<FetchToken>,
        layouts: Vec<Layout>,
        active: Option<ActiveDecode>,
        terminal: Option<Box<dyn FnOnce()>>,
        root: Element,
        fallback: Element,
        existing_art_host: bool,
        watch: Option<MountWatch>,
        closed: bool,
        failure: RefCell<Option<CampaignError>>,
    }

    fn resource(error: ResourceLifecycleError) -> CampaignError {
        match error {
            ResourceLifecycleError::Cache(error) => {
                CampaignError::SceneImage(SceneImageError::Cache(error))
            }
            ResourceLifecycleError::Decode(error) => decode(error),
            error => CampaignError::SceneImage(SceneImageError::Resource(error)),
        }
    }
    fn decode(error: ImageDecodeError) -> CampaignError {
        match error {
            ImageDecodeError::CorruptPng => CampaignError::SceneImage(SceneImageError::InvalidPng),
            ImageDecodeError::DimensionCapacity => {
                CampaignError::SceneImage(SceneImageError::Dimensions)
            }
            ImageDecodeError::Lease(error) => {
                CampaignError::SceneImage(SceneImageError::Cache(error))
            }
            error => CampaignError::SceneImage(SceneImageError::Decode(error)),
        }
    }

    impl SceneImageOwner {
        pub(crate) fn new(
            scope: CacheScope,
            limits: SceneImageLimits,
            root: &Element,
            fallback: &Element,
        ) -> Result<Rc<RefCell<Self>>, CampaignError> {
            limits.validate().map_err(CampaignError::SceneImage)?;
            let (decoded, work, surface) =
                limits.capacities().map_err(CampaignError::SceneImage)?;
            // This is the actual local decoder configuration revision, never a server scene label.
            let preparation = RevisionLabel::new(Some("campaign-static-png-vp8-rgba-v1"))
                .map_err(|_| CampaignError::SceneImage(SceneImageError::InvalidLimits))?;
            let lifecycle = ResourceLifecycle::mount_illustration(
                root,
                scope,
                limits.cache,
                ResourceLimits {
                    max_references: limits.cache.max_assets,
                    max_resident: 1,
                    max_pending: 1,
                    max_decoded_bytes: decoded,
                    max_work_bytes: work,
                },
                preparation,
                surface,
            )
            .map_err(resource)?;
            let owner = Rc::new(RefCell::new(Self {
                lifecycle: Rc::new(RefCell::new(lifecycle)),
                scope,
                limits,
                decode_limits: ImageDecodeLimits {
                    max_encoded_bytes: limits.cache.max_bytes,
                    max_dimension: limits.max_width.max(limits.max_height),
                    max_decoded_bytes: decoded,
                    max_work_bytes: work,
                },
                revision: None,
                references: Box::default(),
                selected: None,
                selection: Rc::new(()),
                requested: None,
                layouts: Vec::new(),
                active: None,
                terminal: None,
                root: root.clone(),
                fallback: fallback.clone(),
                existing_art_host: false,
                watch: None,
                closed: false,
                failure: RefCell::new(None),
            }));
            if !root.is_connected() {
                let document = root.owner_document().ok_or(CampaignError::Disposed)?;
                let weak = Rc::downgrade(&owner);
                let callback = Closure::<dyn FnMut(js_sys::Array, MutationObserver)>::new(
                    move |_: js_sys::Array, observer: MutationObserver| {
                        let Some(owner) = weak.upgrade() else {
                            observer.disconnect();
                            return;
                        };
                        if owner.borrow().closed {
                            observer.disconnect();
                            return;
                        }
                        if owner.borrow().root.is_connected() {
                            observer.disconnect();
                            // One finite mount notification; the closure stays owned until disposal.
                            if let Err(error) = Self::install_selected(&owner, false) {
                                *owner.borrow().failure.borrow_mut() = Some(error);
                            }
                        }
                    },
                );
                let observer = MutationObserver::new(callback.as_ref().unchecked_ref())?;
                let options = MutationObserverInit::new();
                options.set_child_list(true);
                options.set_subtree(true);
                observer.observe_with_options(&document, &options)?;
                owner.borrow_mut().watch = Some(MountWatch {
                    observer,
                    _callback: callback,
                });
            }
            Ok(owner)
        }

        pub(crate) fn after_terminal(&mut self, callback: Box<dyn FnOnce()>) -> bool {
            if self.active.is_none() {
                return false;
            }
            // One newest consumer notification; never release codec work early.
            self.terminal = Some(callback);
            true
        }

        pub(crate) fn use_existing_art_host(&mut self) {
            self.existing_art_host = true;
        }

        pub(crate) fn take_failure(&mut self) -> Result<(), CampaignError> {
            match self.failure.borrow_mut().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        pub(crate) fn scope(&self) -> CacheScope {
            self.scope
        }
        pub(crate) fn ensure_drained(&self) -> Result<(), CampaignError> {
            if self.lifecycle.borrow().work_bytes() != 0 {
                return Err(decode(ImageDecodeError::Resource(
                    ResourceError::PendingCapacity,
                )));
            }
            Ok(())
        }
        pub(crate) fn validate(
            &self,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<(), CampaignError> {
            let error = if self.closed {
                Some(SceneImageError::Cache(CacheError::Closed))
            } else if self.scope != assets.scope {
                Some(SceneImageError::Cache(CacheError::WrongScope))
            } else if self
                .revision
                .is_some_and(|revision| assets.revision <= revision)
            {
                Some(SceneImageError::Cache(CacheError::StaleRevision))
            } else if assets.references.len() > self.limits.cache.max_assets {
                Some(SceneImageError::Cache(CacheError::ReferenceCapacity))
            } else if assets
                .selected
                .is_some_and(|key| !assets.references.contains(key))
            {
                Some(SceneImageError::NotSelected)
            } else {
                None
            };
            if let Some(error) = error {
                return Err(CampaignError::SceneImage(error));
            }
            for (index, key) in assets.references.iter().enumerate() {
                if assets
                    .references
                    .iter()
                    .take(index)
                    .any(|old| old.version == key.version)
                {
                    return Err(CampaignError::SceneImage(SceneImageError::Cache(
                        CacheError::ConflictingReference,
                    )));
                }
            }
            Ok(())
        }
        pub(crate) fn reconcile(
            owner: &Rc<RefCell<Self>>,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<Option<FetchToken>, CampaignError> {
            let mut current = owner.borrow_mut();
            current.validate(assets)?;
            current
                .lifecycle
                .borrow_mut()
                .apply_current(assets.scope, assets.revision, assets.references)
                .map_err(resource)?;
            let replaced = current
                .revision
                .is_some_and(|old| old.epoch() != assets.revision.epoch())
                || current.selected.as_ref() != assets.selected;
            current.revision = Some(assets.revision);
            current.references = assets.references.to_vec().into_boxed_slice();
            current
                .layouts
                .retain(|layout| assets.references.contains(&layout.key));
            if replaced {
                current.selection = Rc::new(());
                current.cancel_request()?;
                if let Some(old) = current.selected.take() {
                    match current.lifecycle.borrow_mut().release(&old) {
                        Ok(()) | Err(ResourceLifecycleError::Cache(CacheError::NotCurrent)) => {}
                        Err(error) => return Err(resource(error)),
                    }
                }
                current.show_fallback()?;
            }
            current.selected = assets.selected.cloned();
            if current.selected.is_none() {
                current.show_fallback()?;
                return Ok(None);
            }
            if current.requested.is_some() {
                return Ok(None);
            }
            drop(current);
            Self::install_selected(owner, true)
        }

        pub(crate) fn complete(
            owner: &Rc<RefCell<Self>>,
            token: &FetchToken,
            bytes: Vec<u8>,
        ) -> Result<(), CampaignError> {
            let mut current = owner.borrow_mut();
            if current.closed || current.selected.as_ref() != Some(token.key()) {
                return Err(CampaignError::SceneImage(SceneImageError::NotSelected));
            }
            // Describe the incoming actual bytes, but publish no layout until canonical
            // hash/length/token verification consumes that exact Vec successfully.
            let prepared = current.limits.prepared_layout(&bytes);
            let completion = current.lifecycle.borrow_mut().complete_fetch(token, bytes);
            if let Err(error) = completion {
                if !matches!(
                    error,
                    ResourceLifecycleError::Cache(CacheError::StaleFetch | CacheError::Closed)
                ) {
                    current.requested = None;
                }
                return Err(resource(error));
            }
            current.requested = None;
            let (dimensions, mime) = prepared.map_err(CampaignError::SceneImage)?;
            current.layouts.retain(|layout| &layout.key != token.key());
            current.layouts.push(Layout {
                key: token.key().clone(),
                mime,
                dimensions,
            });
            drop(current);
            Self::install_selected(owner, false).map(|_| ())
        }

        fn install_selected(
            owner: &Rc<RefCell<Self>>,
            allow_fetch: bool,
        ) -> Result<Option<FetchToken>, CampaignError> {
            let mut current = owner.borrow_mut();
            if current.closed {
                return Ok(None);
            }
            let Some(key) = current.selected.clone() else {
                return Ok(None);
            };
            let resident = current
                .lifecycle
                .borrow_mut()
                .get(&key)
                .map_err(|error| decode(ImageDecodeError::Resource(error)))?
                .is_some();
            if resident && current.root.is_connected() {
                current
                    .lifecycle
                    .borrow_mut()
                    .present(&key)
                    .map_err(|error| resource(ResourceLifecycleError::Surface(error)))?;
                current.show_generated()?;
                return Ok(None);
            }
            let present = current
                .lifecycle
                .borrow_mut()
                .contains_bytes(&key)
                .map_err(|error| CampaignError::SceneImage(SceneImageError::Cache(error)))?;
            if !present {
                current.layouts.retain(|layout| layout.key != key);
                if current.requested.is_some() || !allow_fetch {
                    return Ok(None);
                }
                let token = current
                    .lifecycle
                    .borrow_mut()
                    .fetch(&key)
                    .map_err(|error| CampaignError::SceneImage(SceneImageError::Cache(error)))?;
                current.requested = Some(token.clone());
                return Ok(Some(token));
            }
            let (dimensions, mime) = current
                .layouts
                .iter()
                .find(|layout| layout.key == key)
                .map(|layout| (layout.dimensions, layout.mime))
                .ok_or(CampaignError::SceneImage(SceneImageError::InvalidPng))?;
            // No decode while detached, and no replacement operation until real terminal.
            // Retain only the newest selected key/layout while the single old runner drains.
            if !current.root.is_connected() || current.active.is_some() {
                return Ok(None);
            }
            current.root.set_attribute("data-scene-image", "loading")?;
            let mut operation = match BrowserImageDecode::start_owned_prepared(
                current.lifecycle.clone(),
                &key,
                PreparedImageMetadata {
                    mime,
                    width: dimensions.0,
                    height: dimensions.1,
                    max_ancillary_bytes: current.limits.cache.max_bytes,
                },
                current.decode_limits,
                DECODE_DEADLINE_MS,
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    current.show_fallback()?;
                    return Err(decode(error));
                }
            };
            let identity = Rc::new(());
            let selection = current.selection.clone();
            current.active = Some(ActiveDecode {
                identity: identity.clone(),
                key: key.clone(),
            });
            let weak = Rc::downgrade(owner);
            drop(current);
            spawn_local(async move {
                let result = operation.wait().await;
                let Some(owner) = weak.upgrade() else {
                    return;
                };
                let mut current = owner.borrow_mut();
                if !current.active.as_ref().is_some_and(|active| {
                    Rc::ptr_eq(&active.identity, &identity) && active.key == key
                }) {
                    return;
                }
                current.active = None;
                let terminal = current.terminal.take();
                if current.closed {
                    drop(current);
                    if let Some(callback) = terminal {
                        callback();
                    }
                    return;
                }
                let same_selection = current.selected.as_ref() == Some(&key)
                    && Rc::ptr_eq(&current.selection, &selection);
                let display = if same_selection && result == Ok(true) && current.root.is_connected()
                {
                    current.show_generated()
                } else {
                    current.show_fallback()
                };
                if let Err(error) = display {
                    *current.failure.borrow_mut() = Some(error);
                    if let Err(cleanup) = current.show_fallback() {
                        *current.failure.borrow_mut() = Some(cleanup);
                    }
                }
                if same_selection && let Err(error) = result {
                    *current.failure.borrow_mut() = Some(decode(error));
                }
                let deferred = !same_selection && current.selected.is_some();
                drop(current);
                if deferred && let Err(error) = Self::install_selected(&owner, false) {
                    *owner.borrow().failure.borrow_mut() = Some(error);
                }
                if let Some(callback) = terminal {
                    callback();
                }
            });
            Ok(None)
        }

        fn show_generated(&mut self) -> Result<(), CampaignError> {
            if self.existing_art_host
                && let Some(canvas) = self.lifecycle.borrow().illustration_canvas()
            {
                canvas.set_class_name("art");
            }
            self.lifecycle
                .borrow_mut()
                .set_illustration_visible(true)
                .map_err(resource)?;
            self.fallback.remove();
            self.root.set_attribute("data-scene-image", "generated")?;
            Self::failed_class(&self.root, false);
            Ok(())
        }
        fn show_fallback(&mut self) -> Result<(), CampaignError> {
            self.lifecycle
                .borrow_mut()
                .set_illustration_visible(false)
                .map_err(resource)?;
            if self.fallback.parent_node().is_none() {
                self.root.append_child(&self.fallback)?;
            }
            let failed = self
                .fallback
                .dyn_ref::<HtmlImageElement>()
                .is_some_and(|image| {
                    image.has_attribute("src") && image.complete() && image.natural_width() == 0
                });
            Self::failed_class(&self.root, failed);
            self.root.set_attribute("data-scene-image", "fallback")?;
            Ok(())
        }
        fn failed_class(root: &Element, failed: bool) {
            let mut names: Vec<_> = root
                .class_name()
                .split_whitespace()
                .filter(|name| *name != "exploration-art-failed")
                .map(str::to_owned)
                .collect();
            if failed {
                names.push("exploration-art-failed".to_owned());
            }
            root.set_class_name(&names.join(" "));
        }
        fn cancel_request(&mut self) -> Result<(), CampaignError> {
            if let Some(request) = self.requested.take() {
                match self.lifecycle.borrow_mut().cancel_fetch(&request) {
                    Ok(()) | Err(CacheError::StaleFetch) => {}
                    Err(error) => {
                        return Err(CampaignError::SceneImage(SceneImageError::Cache(error)));
                    }
                }
            }
            Ok(())
        }
        pub(crate) fn replace_fallback(&mut self, fallback: &Element) {
            self.fallback = fallback.clone();
        }
        pub(crate) fn set_description(&self, description: &str) {
            if let Err(error) = self
                .lifecycle
                .borrow_mut()
                .set_illustration_description(description)
            {
                *self.failure.borrow_mut() = Some(resource(error));
            }
        }
        pub(crate) fn legacy(&mut self) -> Result<(), CampaignError> {
            if self.closed {
                return Ok(());
            }
            self.cancel_request()?;
            for key in &self.references {
                self.lifecycle.borrow_mut().release(key).map_err(resource)?;
            }
            self.layouts.clear();
            self.selection = Rc::new(());
            self.selected = None;
            self.show_fallback()
        }
        pub(crate) fn dispose(&mut self) -> Result<(), CampaignError> {
            if self.closed {
                return Ok(());
            }
            self.closed = true;
            if let Some(watch) = &self.watch {
                watch.observer.disconnect();
            }
            let fallback = self.show_fallback();
            let result = self.lifecycle.borrow_mut().dispose().map_err(|error| {
                resource(ResourceLifecycleError::Scene(
                    df_render::ResourceSceneError::Scene(error),
                ))
            });
            self.layouts.clear();
            self.references = Box::default();
            self.selected = None;
            self.requested = None;
            // active identity remains until its genuine terminal callback. The runner
            // retains the closed lifecycle and work budget; replacement checks drainage.
            fallback?;
            result.map(|_| ())
        }
    }
    impl Drop for SceneImageOwner {
        fn drop(&mut self) {
            let _cleanup = self.dispose();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> SceneImageLimits {
        SceneImageLimits {
            cache: CacheLimits {
                max_assets: 1,
                max_pending: 1,
                max_leases: 1,
                max_bytes: 64,
            },
            max_width: 8,
            max_height: 8,
            max_pixels: 32,
        }
    }
    fn header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }
    #[test]
    fn png_dimensions_are_checked_before_decode() {
        assert_eq!(limits().dimensions(&header(4, 4)), Ok((4, 4)));
        for (w, h) in [(0, 1), (1, 0), (9, 1), (8, 8), (u32::MAX, u32::MAX)] {
            assert_eq!(
                limits().dimensions(&header(w, h)),
                Err(SceneImageError::Dimensions)
            );
        }
    }
    #[test]
    fn truncated_non_png_and_unsupported_ihdr_are_refused() {
        let valid = header(1, 1);
        for size in 0..33 {
            assert_eq!(
                limits().dimensions(&valid[..size]),
                Err(SceneImageError::InvalidPng)
            );
        }
        let mut wrong = valid.clone();
        wrong[0] = 0;
        assert_eq!(
            limits().dimensions(&wrong),
            Err(SceneImageError::InvalidPng)
        );
        wrong = valid.clone();
        wrong[26] = 1;
        assert_eq!(
            limits().dimensions(&wrong),
            Err(SceneImageError::InvalidPng)
        );
    }
    #[test]
    fn actual_public_png_and_static_vp8_use_the_canonical_prepared_decoder() {
        let limits = SceneImageLimits {
            cache: CacheLimits {
                max_assets: 2,
                max_pending: 1,
                max_leases: 1,
                max_bytes: 2_316_859,
            },
            max_width: 1672,
            max_height: 941,
            max_pixels: 1_573_352,
        };
        assert_eq!(
            limits.prepared_layout(include_bytes!(
                "../../../assets/ui/scenes/mara-harbor-v4.png"
            )),
            Ok(((1672, 941), "image/png"))
        );
        // Tiny lossy VP8 keyframe (24x16) generated for this test.
        let webp: &[u8] = &[
            82, 73, 70, 70, 152, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 32, 140, 0, 0, 0, 112, 5, 0,
            157, 1, 42, 24, 0, 16, 0, 62, 181, 74, 159, 73, 167, 36, 34, 161, 48, 8, 0, 224, 22,
            137, 108, 0, 157, 50, 130, 142, 7, 240, 15, 224, 23, 31, 253, 32, 31, 255, 244, 153,
            191, 64, 62, 0, 63, 255, 131, 34, 216, 0, 254, 225, 79, 142, 191, 222, 139, 126, 237,
            59, 172, 139, 197, 4, 131, 249, 243, 144, 77, 121, 27, 64, 154, 242, 117, 194, 99, 132,
            179, 88, 142, 54, 86, 196, 252, 182, 154, 76, 79, 66, 127, 254, 248, 223, 254, 251, 78,
            255, 254, 252, 159, 223, 88, 197, 208, 79, 244, 12, 64, 95, 144, 63, 65, 63, 208, 227,
            143, 142, 210, 86, 153, 137, 149, 113, 196, 27, 193, 250, 102, 38, 100, 202, 188, 95,
            192, 0, 0,
        ];
        assert_eq!(limits.prepared_layout(webp), Ok(((24, 16), "image/webp")));
        let mut corrupt = webp.to_vec();
        corrupt[4] ^= 1;
        assert_eq!(
            limits.prepared_layout(&corrupt),
            Err(SceneImageError::Decode(
                df_render::ImageDecodeError::CorruptWebp
            ))
        );
        assert_eq!(
            SceneImageLimits {
                max_width: 23,
                ..limits
            }
            .prepared_layout(webp),
            Err(SceneImageError::Dimensions)
        );
    }
    #[test]
    fn private_decoder_selection_does_not_admit_extended_animated_or_unknown_formats() {
        let mut bytes = b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
        for kind in [b"VP8X", b"VP8L", b"ANIM"] {
            bytes[12..16].copy_from_slice(kind);
            assert_eq!(
                limits().prepared_layout(&bytes),
                Err(SceneImageError::Decode(
                    df_render::ImageDecodeError::UnsupportedWebp
                ))
            );
        }
        assert_eq!(
            limits().prepared_layout(b"image/jpeg"),
            Err(SceneImageError::Decode(
                df_render::ImageDecodeError::UnsupportedMime
            ))
        );
    }
}
