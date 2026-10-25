use crate::{
    DecodeToken, DecodedImage, FlatScene, ImageDecodeError, ImageDecodeLimits,
    PreparedImageMetadata, PresentationOutcome, RenderError, ResourceError, ResourceLimits,
    ResourceReadiness, ResourceSceneError, ResourceSceneRenderer, SceneRenderer, VerifiedImage,
    VerifiedPng, WorkOutcome,
};
use df_client::cache::{
    AssetCache, CacheError, CacheKey, CacheLease, CacheLimits, CacheScope, FetchToken,
};
use df_types::{RevisionLabel, SessionRevision};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceLifecycleError {
    Cache(CacheError),
    Decode(ImageDecodeError),
    Resource(ResourceError),
    Scene(ResourceSceneError),
    UnsupportedTarget,
    #[cfg(target_arch = "wasm32")]
    Surface(crate::ImageSurfaceError),
}

enum Target {
    Flat(ResourceSceneRenderer<DecodedImage<CacheKey>>),
    #[cfg(target_arch = "wasm32")]
    Browser(crate::BrowserResourceScene<CacheKey>),
    #[cfg(target_arch = "wasm32")]
    Illustration(crate::browser_illustration::BrowserIllustration),
}

struct PendingLease {
    token: DecodeToken<CacheKey>,
    lease: CacheLease,
}

/// Closed presentation owner connecting one canonical AssetCache to its actual scene.
/// All byte-cache mutations finish before lease guards, retirement or abort callbacks run.
/// There is no escaping cache/scene Rc, subscription, polling loop or second byte cache.
///
/// Pending records clone the decoder's actual lease and DecodeToken, never allocate a
/// second lease slot or work identity. Cancellation keeps both records and the scene's
/// work reservation until a terminal codec callback. Async decoders must retain this
/// owner through that callback, including after scope replacement or disposal.
pub struct ResourceLifecycle {
    scope: CacheScope,
    asset_cache: Rc<RefCell<AssetCache>>,
    target: Target,
    pending: Vec<PendingLease>,
    resident_leases: Vec<CacheLease>,
    closed: bool,
}

impl ResourceLifecycle {
    pub fn from_scene(
        scene: SceneRenderer,
        scope: CacheScope,
        cache_limits: CacheLimits,
        resource_limits: ResourceLimits,
        preparation_revision: RevisionLabel,
    ) -> Result<Self, ResourceLifecycleError> {
        let owner = (scope.session, scope.run, scope.binding);
        if scene.owner() != owner {
            return Err(ResourceLifecycleError::Resource(ResourceError::WrongOwner));
        }
        let bytes = AssetCache::new(scope, cache_limits).map_err(ResourceLifecycleError::Cache)?;
        let target =
            ResourceSceneRenderer::from_scene(scene, owner, resource_limits, preparation_revision)
                .map_err(ResourceLifecycleError::Resource)?;
        Ok(Self::new(scope, bytes, Target::Flat(target)))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn mount(
        parent: &web_sys::Element,
        scope: CacheScope,
        cache_limits: CacheLimits,
        resource_limits: ResourceLimits,
        preparation_revision: RevisionLabel,
        max_surface_bytes: usize,
    ) -> Result<Self, ResourceLifecycleError> {
        let bytes = AssetCache::new(scope, cache_limits).map_err(ResourceLifecycleError::Cache)?;
        let target = crate::BrowserResourceScene::mount(
            parent,
            (scope.session, scope.run, scope.binding),
            resource_limits,
            preparation_revision,
            max_surface_bytes,
        )
        .map_err(ResourceLifecycleError::Surface)?;
        Ok(Self::new(scope, bytes, Target::Browser(target)))
    }

    /// Illustration-only target: no scene geometry or synthetic scene label. The
    /// canvas is allocated lazily on first connected presentation; one canonical
    /// encoded cache is owned from enablement through real terminal codec settlement.
    #[cfg(target_arch = "wasm32")]
    pub fn mount_illustration(
        parent: &web_sys::Element,
        scope: CacheScope,
        cache_limits: CacheLimits,
        resource_limits: ResourceLimits,
        preparation_revision: RevisionLabel,
        max_surface_bytes: usize,
    ) -> Result<Self, ResourceLifecycleError> {
        let bytes = AssetCache::new(scope, cache_limits).map_err(ResourceLifecycleError::Cache)?;
        let target = crate::browser_illustration::BrowserIllustration::new(
            parent,
            (scope.session, scope.run, scope.binding),
            resource_limits,
            preparation_revision,
            max_surface_bytes,
        )
        .map_err(ResourceLifecycleError::Surface)?;
        Ok(Self::new(scope, bytes, Target::Illustration(target)))
    }

    /// The exact owned illustration surface; closed owners expose no live DOM handle.
    #[cfg(target_arch = "wasm32")]
    pub fn illustration_canvas(&self) -> Option<&web_sys::HtmlCanvasElement> {
        if self.closed {
            return None;
        }
        match &self.target {
            Target::Illustration(target) => target.canvas(),
            _ => None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn set_illustration_visible(
        &mut self,
        visible: bool,
    ) -> Result<(), ResourceLifecycleError> {
        match &self.target {
            Target::Illustration(target) => {
                target.set_visible(visible);
                Ok(())
            }
            _ => Err(ResourceLifecycleError::UnsupportedTarget),
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn set_illustration_description(
        &mut self,
        description: &str,
    ) -> Result<(), ResourceLifecycleError> {
        match &mut self.target {
            Target::Illustration(target) => target
                .set_description(description)
                .map_err(ResourceLifecycleError::Resource),
            _ => Err(ResourceLifecycleError::UnsupportedTarget),
        }
    }

    fn new(scope: CacheScope, bytes: AssetCache, target: Target) -> Self {
        Self {
            scope,
            asset_cache: Rc::new(RefCell::new(bytes)),
            target,
            pending: Vec::new(),
            resident_leases: Vec::new(),
            closed: false,
        }
    }

    pub fn scope(&self) -> CacheScope {
        self.scope
    }

    /// Only the accepted-view owner supplies complete already permitted references.
    /// Rejected byte-cache revisions do not touch the mounted scene or live decodes.
    /// An accepted revocation scrubs pixels synchronously without a scene snapshot.
    pub fn apply_current(
        &mut self,
        scope: CacheScope,
        revision: SessionRevision,
        references: &[CacheKey],
    ) -> Result<(), ResourceLifecycleError> {
        if self.closed {
            return Err(ResourceLifecycleError::Cache(CacheError::Closed));
        }
        if scope != self.scope {
            return Err(ResourceLifecycleError::Cache(CacheError::WrongScope));
        }
        #[cfg(target_arch = "wasm32")]
        if let Target::Illustration(target) = &self.target {
            target
                .validate_current(revision, references)
                .map_err(ResourceLifecycleError::Resource)?;
        }
        self.asset_cache
            .borrow_mut()
            .apply_current(scope, revision, references)
            .map_err(ResourceLifecycleError::Cache)?;
        #[cfg(target_arch = "wasm32")]
        if let Target::Illustration(target) = &mut self.target {
            target
                .apply_current(revision, references)
                .map_err(ResourceLifecycleError::Resource)?;
        }
        self.reconcile_leases()
    }

    /// Current encoded presence only; no byte/cache handle escapes this owner.
    pub fn contains_bytes(&mut self, key: &CacheKey) -> Result<bool, CacheError> {
        self.asset_cache
            .borrow_mut()
            .get(key)
            .map(|bytes| bytes.is_some())
    }

    pub fn fetch(&mut self, key: &CacheKey) -> Result<FetchToken, CacheError> {
        self.asset_cache.borrow_mut().fetch(key)
    }

    /// SHA-256/length validation and any LRU eviction remain canonical AssetCache work.
    /// A successful eviction immediately cancels affected decode work and scrubs surfaces.
    pub fn complete_fetch(
        &mut self,
        token: &FetchToken,
        bytes: Vec<u8>,
    ) -> Result<(), ResourceLifecycleError> {
        self.asset_cache
            .borrow_mut()
            .complete(token, bytes)
            .map_err(ResourceLifecycleError::Cache)?;
        self.reconcile_leases()
    }

    pub fn cancel_fetch(&mut self, token: &FetchToken) -> Result<(), CacheError> {
        self.asset_cache.borrow_mut().cancel(token)
    }

    pub fn release(&mut self, key: &CacheKey) -> Result<(), ResourceLifecycleError> {
        self.asset_cache
            .borrow_mut()
            .release(key)
            .map_err(ResourceLifecycleError::Cache)?;
        self.reconcile_leases()
    }

    /// Revokes the existing decoder input's exact lease identity, including its clones.
    /// A PNG acquired from another canonical cache cannot affect this owner.
    pub fn release_lease(&mut self, input: &VerifiedPng) -> Result<(), ResourceLifecycleError> {
        self.check_input(input.image())?;
        self.asset_cache
            .borrow_mut()
            .release_lease(input.image().cache_lease())
            .map_err(ResourceLifecycleError::Cache)?;
        self.reconcile_leases()
    }

    pub fn prepare_png(
        &self,
        key: &CacheKey,
        limits: ImageDecodeLimits,
    ) -> Result<VerifiedPng, ImageDecodeError> {
        VerifiedPng::acquire(Rc::clone(&self.asset_cache), key, limits)
    }

    /// MIME/dimensions describe this exact already permitted asset; they cannot authorize
    /// acquisition. The prepared decoder still uses this owner's single canonical byte lease.
    pub fn prepare_image(
        &self,
        key: &CacheKey,
        metadata: PreparedImageMetadata<'_>,
        limits: ImageDecodeLimits,
    ) -> Result<VerifiedImage, ImageDecodeError> {
        VerifiedImage::acquire_prepared(Rc::clone(&self.asset_cache), key, metadata, limits)
    }

    pub fn update_scene(
        &mut self,
        scene_label: RevisionLabel,
        flat: FlatScene,
        references: &[CacheKey],
    ) -> Result<PresentationOutcome, ResourceLifecycleError> {
        let owner = (self.scope.session, self.scope.run, self.scope.binding);
        let outcome = match &mut self.target {
            Target::Flat(target) => target
                .update(owner, scene_label, flat, references)
                .map_err(ResourceLifecycleError::Scene)?,
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target
                .update(owner, scene_label, flat, references)
                .map_err(ResourceLifecycleError::Surface)?,
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(_) => return Err(ResourceLifecycleError::UnsupportedTarget),
        };
        if matches!(outcome, PresentationOutcome::Applied { .. }) {
            self.reconcile_leases()?;
        }
        Ok(outcome)
    }

    /// The existing decoder owns input acquisition and its plan. No browser allocations
    /// may start until this admission succeeds. The returned token is the scene's token.
    pub fn begin(
        &mut self,
        input: &VerifiedPng,
        abort: impl FnOnce() + 'static,
    ) -> Result<DecodeToken<CacheKey>, ResourceLifecycleError> {
        self.begin_image(input.image(), abort)
    }

    /// Admits the neutral prepared image through the same exact canonical lease and token.
    /// This shares legacy PNG bookkeeping and starts no browser allocation itself.
    pub fn begin_image(
        &mut self,
        input: &VerifiedImage,
        abort: impl FnOnce() + 'static,
    ) -> Result<DecodeToken<CacheKey>, ResourceLifecycleError> {
        self.check_input(input)?;
        input
            .validate_current()
            .map_err(ResourceLifecycleError::Decode)?;
        let token = match &mut self.target {
            Target::Flat(target) => target.begin(input.key(), input.plan().budget, abort),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.begin(input.key(), input.plan().budget, abort),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.begin(input.key(), input.plan().budget, abort),
        }
        .map_err(ResourceLifecycleError::Resource)?;
        self.pending.push(PendingLease {
            token: token.clone(),
            lease: input.cache_lease().clone(),
        });
        Ok(token)
    }

    /// Actual terminal codec success, never an abort request. The incoming decoded image
    /// must carry the exact verified image lease admitted for this operation. A foreign or
    /// unbound image is refused before consuming work; this owner does not copy pixels.
    pub fn complete(
        &mut self,
        token: &DecodeToken<CacheKey>,
        image: DecodedImage<CacheKey>,
    ) -> Result<(), ResourceError> {
        let canonical_lease = self
            .pending
            .iter()
            .find(|pending| pending.token.same_operation(token))
            .map(|pending| pending.lease.clone())
            .ok_or(ResourceError::StaleDecode)?;
        if !image
            .cache_lease()
            .is_some_and(|lease| lease.same_identity(&canonical_lease))
        {
            return Err(ResourceError::WrongResource);
        }
        if self
            .asset_cache
            .borrow()
            .lease_bytes(&canonical_lease)
            .is_err()
        {
            // Current authority still fences the exact original operation, even when
            // its codec reports success after canonical cache invalidation.
            self.cancel(token)?;
        }
        let result = match &mut self.target {
            Target::Flat(target) => target.complete(token, image),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.complete(token, image),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.complete(token, image),
        };
        // Only StaleDecode means the scene did not consume an operation. Old callbacks
        // therefore cannot retire a newer pending lease with a reused immutable key.
        if result != Err(ResourceError::StaleDecode) {
            self.pending
                .retain(|pending| !pending.token.same_operation(token));
        }
        if result.is_ok() {
            self.resident_leases.push(canonical_lease);
            self.prune_resident_leases()?;
        }
        result
    }

    pub fn finish_failed(
        &mut self,
        token: &DecodeToken<CacheKey>,
    ) -> Result<WorkOutcome, ResourceError> {
        let result = match &mut self.target {
            Target::Flat(target) => target.finish_failed(token),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.finish_failed(token),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.finish_failed(token),
        };
        if result.is_ok() {
            self.pending
                .retain(|pending| !pending.token.same_operation(token));
        }
        result
    }

    pub fn cancel(&mut self, token: &DecodeToken<CacheKey>) -> Result<(), ResourceError> {
        match &mut self.target {
            Target::Flat(target) => target.cancel(token),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.cancel(token),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.cancel(token),
        }
    }

    pub fn get(
        &mut self,
        key: &CacheKey,
    ) -> Result<Option<&DecodedImage<CacheKey>>, ResourceError> {
        match &mut self.target {
            Target::Flat(target) => target.get(key),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.get(key),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.get(key),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn present(&mut self, key: &CacheKey) -> Result<bool, crate::ImageSurfaceError> {
        match &mut self.target {
            Target::Flat(_) => Err(crate::ImageSurfaceError::Browser),
            Target::Browser(target) => target.present(key),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.present(key),
        }
    }

    /// A replacement scope needs a newly mounted owner. This old owner stays retained by
    /// actual decoder runners until terminal callbacks; it never changes their identity.
    /// Same-scope reconnect does not dispose anything; accepted epoch updates fence leases.
    pub fn replace_scope(
        &mut self,
        replacement: CacheScope,
    ) -> Result<Option<PresentationOutcome>, RenderError> {
        if replacement == self.scope {
            return Ok(None);
        }
        self.dispose().map(Some)
    }

    pub fn dispose(&mut self) -> Result<PresentationOutcome, RenderError> {
        self.asset_cache.borrow_mut().dispose();
        self.closed = true;
        self.resident_leases = Vec::new();
        match &mut self.target {
            Target::Flat(target) => target.dispose(),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.dispose(),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.dispose(),
        }
    }

    pub fn resident_bytes(&self) -> usize {
        self.asset_cache.borrow().resident_bytes()
    }
    pub fn lease_count(&self) -> usize {
        self.asset_cache.borrow().lease_count()
    }
    pub fn work_bytes(&self) -> usize {
        match &self.target {
            Target::Flat(target) => target.work_bytes(),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.work_bytes(),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.work_bytes(),
        }
    }
    pub fn decoded_bytes(&self) -> usize {
        match &self.target {
            Target::Flat(target) => target.decoded_bytes(),
            #[cfg(target_arch = "wasm32")]
            Target::Browser(target) => target.decoded_bytes(),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.decoded_bytes(),
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn surface_bytes(&self) -> usize {
        match &self.target {
            Target::Flat(_) => 0,
            Target::Browser(target) => target.surface_bytes(),
            #[cfg(target_arch = "wasm32")]
            Target::Illustration(target) => target.surface_bytes(),
        }
    }

    fn check_input(&self, input: &VerifiedImage) -> Result<(), ResourceLifecycleError> {
        if !input.belongs_to(&self.asset_cache) {
            return Err(ResourceLifecycleError::Decode(ImageDecodeError::WrongOwner));
        }
        Ok(())
    }

    fn reconcile_leases(&mut self) -> Result<(), ResourceLifecycleError> {
        if self.closed {
            return Ok(());
        }
        for pending in &self.pending {
            let invalid = self
                .asset_cache
                .borrow()
                .lease_bytes(&pending.lease)
                .is_err();
            if invalid {
                match &mut self.target {
                    Target::Flat(target) => target.cancel(&pending.token),
                    #[cfg(target_arch = "wasm32")]
                    Target::Browser(target) => target.cancel(&pending.token),
                    #[cfg(target_arch = "wasm32")]
                    Target::Illustration(target) => target.cancel(&pending.token),
                }
                .map_err(ResourceLifecycleError::Resource)?;
            }
        }
        for lease in &self.resident_leases {
            let invalid = self.asset_cache.borrow().lease_bytes(lease).is_err();
            if invalid {
                match &mut self.target {
                    Target::Flat(target) => target.release(lease.key()),
                    #[cfg(target_arch = "wasm32")]
                    Target::Browser(target) => target.release(lease.key()),
                    #[cfg(target_arch = "wasm32")]
                    Target::Illustration(target) => target.release(lease.key()),
                }
                .map_err(ResourceLifecycleError::Resource)?;
            }
        }
        self.prune_resident_leases()
            .map_err(ResourceLifecycleError::Resource)?;
        #[cfg(target_arch = "wasm32")]
        match &mut self.target {
            Target::Browser(target) => {
                target.refresh().map_err(ResourceLifecycleError::Surface)?;
            }
            Target::Illustration(target) => {
                target.refresh().map_err(ResourceLifecycleError::Surface)?;
            }
            Target::Flat(_) => {}
        }
        Ok(())
    }

    fn prune_resident_leases(&mut self) -> Result<(), ResourceError> {
        let mut index = 0;
        while let Some(lease) = self.resident_leases.get(index) {
            // Readiness validates the real image guard without changing decoded LRU order.
            let result = match &self.target {
                Target::Flat(target) => target.readiness(lease.key()),
                #[cfg(target_arch = "wasm32")]
                Target::Browser(target) => target.readiness(lease.key()),
                #[cfg(target_arch = "wasm32")]
                Target::Illustration(target) => target.readiness(lease.key()),
            };
            match result {
                Ok(ResourceReadiness::Resident) => index += 1,
                Ok(ResourceReadiness::Missing | ResourceReadiness::Pending)
                | Err(ResourceError::NotCurrent | ResourceError::RevokedResource) => {
                    match &mut self.target {
                        Target::Flat(target) => target.release(lease.key()),
                        #[cfg(target_arch = "wasm32")]
                        Target::Browser(target) => target.release(lease.key()),
                        #[cfg(target_arch = "wasm32")]
                        Target::Illustration(target) => target.release(lease.key()),
                    }?;
                    self.resident_leases.remove(index);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl Drop for ResourceLifecycle {
    fn drop(&mut self) {
        // The target owns terminal DOM Drop cleanup. Avoid fallible DOM calls here.
        // Explicit disposal is required before releasing the runner's final ownership.
        self.asset_cache.borrow_mut().dispose();
    }
}
