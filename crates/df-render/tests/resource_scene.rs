#![cfg(not(target_arch = "wasm32"))]
#[path = "support/resource_flat.rs"]
mod geometry;
#[path = "support/authorized_image.rs"]
mod support;
use df_client::cache::CacheKey;
use df_render::{
    DecodeBudget, DecodedImage, PresentationOutcome, RenderError, ResourceError, ResourceLimits,
    ResourceSceneError, ResourceSceneRenderer, SceneRenderer,
};
use df_types::ClientBindingId;
use support::{Consumer, label, owner, revision};

#[test]
fn one_actual_scene_owner_preflights_complete_resources_and_disposes_before_geometry() {
    let consumer = Consumer::new();
    consumer.cache_streamed();
    let mut renderer: ResourceSceneRenderer<DecodedImage<CacheKey>> =
        ResourceSceneRenderer::from_scene(
            SceneRenderer::new(owner()),
            owner(),
            ResourceLimits {
                max_references: 2,
                max_resident: 1,
                max_pending: 1,
                max_decoded_bytes: 4,
                max_work_bytes: 24,
            },
            label("rgba8-surface-v1"),
        )
        .unwrap();
    assert_eq!(
        renderer.update(
            owner(),
            label("scene-one"),
            geometry::flat(revision(1, 0)),
            std::slice::from_ref(&consumer.key)
        ),
        Ok(PresentationOutcome::Applied {
            layers: 0,
            tokens: 1
        })
    );
    let work = renderer
        .begin(
            &consumer.key,
            DecodeBudget {
                decoded_bytes: 4,
                work_bytes: 24,
            },
            || {},
        )
        .unwrap();
    renderer.complete(&work, consumer.decode_fixture()).unwrap();
    let original = renderer.current().unwrap().clone();
    let invalid_refs = [consumer.key.clone(), consumer.key.clone()];
    assert_eq!(
        renderer.update(
            owner(),
            label("scene-one"),
            geometry::flat(revision(1, 1)),
            &invalid_refs
        ),
        Err(ResourceSceneError::Resource(
            ResourceError::DuplicateReference
        ))
    );
    assert_eq!(renderer.current(), Some(&original));
    assert_eq!(renderer.decoded_bytes(), 4);
    let mut wrong_owner = owner();
    wrong_owner.2 = ClientBindingId::from_bytes(&[9; 16]).unwrap();
    assert_eq!(
        renderer.update(
            wrong_owner,
            label("scene-one"),
            geometry::flat(revision(1, 1)),
            &[]
        ),
        Err(ResourceSceneError::Scene(RenderError::WrongOwner))
    );
    let mut invalid = geometry::flat(revision(1, 1));
    invalid.viewport.width = f64::NAN;
    assert_eq!(
        renderer.update(owner(), label("scene-one"), invalid, &[]),
        Err(ResourceSceneError::Scene(RenderError::InvalidGeometry))
    );
    assert_eq!(renderer.current(), Some(&original));
    assert_eq!(
        renderer
            .get(&consumer.key)
            .unwrap()
            .unwrap()
            .pixels()
            .unwrap(),
        [16, 32, 48, 255]
    );
    assert_eq!(
        renderer.update(owner(), label("changed-stale-label"), original.clone(), &[]),
        Ok(PresentationOutcome::Duplicate {
            current: revision(1, 0)
        })
    );
    assert_eq!(renderer.decoded_bytes(), 4);
    assert_eq!(consumer.released.get(), 0);
    renderer
        .update(
            owner(),
            label("scene-one"),
            geometry::flat(revision(1, 1)),
            &[],
        )
        .unwrap();
    assert_eq!(consumer.released.get(), 1);
    assert_eq!(renderer.decoded_bytes(), 0);
    assert_eq!(renderer.dispose(), Ok(PresentationOutcome::Disposed));
    assert_eq!(renderer.dispose(), Ok(PresentationOutcome::AlreadyDisposed));
    assert_eq!(
        renderer
            .begin(
                &consumer.key,
                DecodeBudget {
                    decoded_bytes: 4,
                    work_bytes: 24
                },
                || {}
            )
            .err(),
        Some(ResourceError::Closed)
    );
}

#[test]
fn already_initialized_scene_cannot_skip_resource_admission_as_duplicate() {
    let mut scene = SceneRenderer::new(owner());
    scene
        .update(owner(), geometry::flat(revision(1, 0)))
        .unwrap();
    let initialization_result = ResourceSceneRenderer::<DecodedImage<CacheKey>>::from_scene(
        scene,
        owner(),
        ResourceLimits {
            max_references: 2,
            max_resident: 1,
            max_pending: 1,
            max_decoded_bytes: 4,
            max_work_bytes: 24,
        },
        label("rgba8-surface-v1"),
    );
    assert_eq!(
        initialization_result.err(),
        Some(ResourceError::SceneAlreadyInitialized)
    );
}

#[test]
fn disposed_scene_wrapper_retains_bounded_live_work_and_consumes_terminal_resource() {
    let consumer = Consumer::new();
    consumer.cache_streamed();
    let mut renderer: ResourceSceneRenderer<DecodedImage<CacheKey>> =
        ResourceSceneRenderer::from_scene(
            SceneRenderer::new(owner()),
            owner(),
            ResourceLimits {
                max_references: 2,
                max_resident: 1,
                max_pending: 1,
                max_decoded_bytes: 4,
                max_work_bytes: 24,
            },
            label("rgba8-surface-v1"),
        )
        .unwrap();
    renderer
        .update(
            owner(),
            label("scene-one"),
            geometry::flat(revision(1, 0)),
            std::slice::from_ref(&consumer.key),
        )
        .unwrap();
    let pending = renderer
        .begin(
            &consumer.key,
            DecodeBudget {
                decoded_bytes: 4,
                work_bytes: 24,
            },
            || {},
        )
        .unwrap();
    renderer.dispose().unwrap();
    assert!(renderer.current().is_none());
    assert_eq!(renderer.work_bytes(), 24);
    assert_eq!(
        renderer.complete(&pending, consumer.decode_fixture()),
        Err(ResourceError::Closed)
    );
    assert_eq!(renderer.work_bytes(), 0);
    assert_eq!(renderer.pending_count(), 0);
    assert_eq!(consumer.released.get(), 1);
    assert_eq!(consumer.bytes.borrow().lease_count(), 0);
}
