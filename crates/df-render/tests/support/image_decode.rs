use df_assets::AssetManifest;
use df_client::cache::{AssetCache, CacheKey, CacheLimits, CacheScope};
use df_render::{ImageDecodeLimits, SceneOwner};
use df_types::{ClientBindingId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision};
use std::cell::RefCell;
use std::rc::Rc;

// Complete 1x1 RGBA8 PNG, stored Deflate block, opaque pixel [16,32,48,255].
pub const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 16, 73, 68, 65, 84, 120, 1, 1, 5, 0, 250, 255, 0, 16, 32, 48,
    255, 2, 4, 1, 96, 145, 5, 159, 157, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const CORRUPT_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 16, 73, 68, 65, 84, 120, 1, 7, 5, 0, 250, 255, 0, 16, 32, 48,
    255, 2, 4, 1, 96, 105, 199, 212, 43, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const TOO_WIDE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 16, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 177, 227, 0, 66, 0, 0, 0, 16, 73, 68, 65, 84, 120, 1, 1, 5, 0, 250, 255, 0, 16, 32, 48,
    255, 2, 4, 1, 96, 145, 5, 159, 157, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const UNSUPPORTED_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 16, 6, 0,
    0, 0, 79, 133, 24, 202, 0, 0, 0, 16, 73, 68, 65, 84, 120, 1, 1, 5, 0, 250, 255, 0, 16, 32, 48,
    255, 2, 4, 1, 96, 145, 5, 159, 157, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const PNG_DIGEST: [u8; 32] = [
    222, 192, 39, 112, 37, 240, 57, 61, 121, 122, 164, 72, 115, 222, 203, 189, 117, 216, 238, 33,
    18, 247, 83, 188, 89, 237, 149, 178, 217, 61, 158, 233,
];
pub const CORRUPT_DIGEST: [u8; 32] = [
    75, 159, 41, 196, 9, 137, 105, 56, 167, 87, 88, 209, 184, 86, 59, 122, 64, 15, 40, 129, 185,
    81, 156, 51, 191, 91, 36, 29, 240, 137, 0, 44,
];
pub const WIDE_DIGEST: [u8; 32] = [
    142, 168, 21, 165, 198, 1, 21, 117, 83, 238, 248, 142, 81, 36, 132, 90, 222, 177, 13, 174, 185,
    202, 194, 41, 138, 210, 169, 120, 229, 41, 194, 90,
];

pub fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}
pub fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}
pub fn owner() -> SceneOwner {
    (
        SessionId::from_bytes(&[1; 16]).unwrap(),
        RunId::from_bytes(&[2; 16]).unwrap(),
        ClientBindingId::from_bytes(&[3; 16]).unwrap(),
    )
}
pub fn scope() -> CacheScope {
    let (session, run, binding) = owner();
    CacheScope {
        session,
        run,
        binding,
    }
}
pub fn limits() -> ImageDecodeLimits {
    ImageDecodeLimits {
        max_encoded_bytes: 1024,
        max_dimension: 4096,
        max_decoded_bytes: 1024,
        max_work_bytes: 4096,
    }
}
pub fn cached(source: &[u8], digest: [u8; 32]) -> (Rc<RefCell<AssetCache>>, CacheKey) {
    let key = CacheKey {
        version: label("current-png-fixture-v1"),
        bytes: AssetManifest {
            byte_len: source.len() as u64,
            sha256: digest,
        },
    };
    let mut cache = AssetCache::new(
        scope(),
        CacheLimits {
            max_assets: 2,
            max_pending: 1,
            max_leases: 4,
            max_bytes: 1024,
        },
    )
    .unwrap();
    cache
        .apply_current(scope(), revision(1, 0), std::slice::from_ref(&key))
        .unwrap();
    let fetch = cache.fetch(&key).unwrap();
    cache.complete(&fetch, source.to_vec()).unwrap();
    (Rc::new(RefCell::new(cache)), key)
}

// Exact current fixed public route fixtures, not server publication or authorization.
pub struct PreparedAsset {
    pub route: &'static str,
    pub bytes: &'static [u8],
    pub digest: [u8; 32],
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}
impl PreparedAsset {
    pub fn metadata(&self) -> df_render::PreparedImageMetadata<'static> {
        df_render::PreparedImageMetadata {
            mime: self.mime,
            width: self.width,
            height: self.height,
            max_ancillary_bytes: 32 * 1024,
        }
    }
    pub fn key(&self) -> CacheKey {
        CacheKey {
            version: label(self.route),
            bytes: AssetManifest {
                byte_len: self.bytes.len() as u64,
                sha256: self.digest,
            },
        }
    }
}
pub fn prepared_limits() -> ImageDecodeLimits {
    ImageDecodeLimits {
        max_encoded_bytes: 4 * 1024 * 1024,
        max_dimension: 2048,
        max_decoded_bytes: 8 * 1024 * 1024,
        max_work_bytes: 64 * 1024 * 1024,
    }
}
/// Tiny lossy WebP (VP8 keyframe, 16x16) generated for these tests.
const TINY_SCENE_A: [u8; 126] = [
    82, 73, 70, 70, 118, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 32, 106, 0, 0, 0, 80, 1, 0, 157, 1,
    42, 16, 0, 16, 0, 2, 128, 66, 37, 176, 0, 4, 116, 0, 0, 254, 234, 215, 198, 63, 241, 44, 125,
    213, 55, 243, 174, 16, 63, 164, 7, 240, 168, 217, 7, 127, 58, 225, 3, 250, 64, 127, 10, 141,
    144, 119, 243, 174, 16, 63, 164, 7, 240, 168, 106, 2, 254, 117, 194, 7, 244, 128, 254, 21, 31,
    250, 189, 55, 240, 184, 32, 111, 119, 138, 0, 36, 255, 11, 130, 6, 247, 120, 160, 2, 79, 240,
    184, 32, 111, 119, 135, 231, 77, 252, 46, 8, 27, 221, 212, 128, 0,
];

/// Tiny lossy WebP (VP8 keyframe, 24x16) generated for these tests.
const TINY_SCENE_B: [u8; 160] = [
    82, 73, 70, 70, 152, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 32, 140, 0, 0, 0, 112, 5, 0, 157, 1,
    42, 24, 0, 16, 0, 62, 181, 74, 159, 73, 167, 36, 34, 161, 48, 8, 0, 224, 22, 137, 108, 0, 157,
    50, 130, 142, 7, 240, 15, 224, 23, 31, 253, 32, 31, 255, 244, 153, 191, 64, 62, 0, 63, 255,
    131, 34, 216, 0, 254, 225, 79, 142, 191, 222, 139, 126, 237, 59, 172, 139, 197, 4, 131, 249,
    243, 144, 77, 121, 27, 64, 154, 242, 117, 194, 99, 132, 179, 88, 142, 54, 86, 196, 252, 182,
    154, 76, 79, 66, 127, 254, 248, 223, 254, 251, 78, 255, 254, 252, 159, 223, 88, 197, 208, 79,
    244, 12, 64, 95, 144, 63, 65, 63, 208, 227, 143, 142, 210, 86, 153, 137, 149, 113, 196, 27,
    193, 250, 102, 38, 100, 202, 188, 95, 192, 0, 0,
];

/// Tiny lossy WebP (VP8 keyframe, 32x24) generated for these tests.
const TINY_SCENE_C: [u8; 214] = [
    82, 73, 70, 70, 206, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 32, 194, 0, 0, 0, 240, 6, 0, 157, 1,
    42, 32, 0, 24, 0, 62, 165, 74, 157, 73, 166, 37, 35, 33, 55, 250, 168, 0, 192, 20, 137, 102, 0,
    247, 0, 254, 1, 72, 104, 137, 229, 112, 128, 57, 4, 224, 14, 15, 252, 223, 249, 255, 253, 32,
    31, 255, 252, 224, 55, 163, 191, 98, 61, 255, 193, 84, 24, 26, 80, 0, 254, 240, 83, 241, 29,
    11, 132, 101, 210, 18, 46, 192, 85, 191, 128, 201, 65, 147, 38, 143, 97, 53, 36, 105, 95, 173,
    46, 120, 197, 8, 163, 221, 252, 244, 20, 6, 37, 81, 115, 142, 131, 238, 13, 9, 184, 52, 38,
    104, 115, 187, 174, 216, 223, 255, 157, 235, 195, 183, 24, 63, 237, 81, 175, 255, 237, 68, 127,
    218, 66, 33, 148, 50, 44, 115, 169, 59, 194, 77, 172, 86, 205, 64, 145, 224, 71, 227, 81, 173,
    129, 31, 141, 167, 139, 40, 95, 84, 203, 35, 255, 145, 215, 127, 255, 38, 223, 252, 145, 15, 4,
    146, 28, 199, 81, 188, 106, 5, 30, 76, 88, 82, 33, 221, 101, 175, 58, 238, 0, 0, 0,
];

pub const PREPARED_ASSETS: &[PreparedAsset] = &[
    PreparedAsset {
        route: "/assets/ui/scenes/mara-harbor-v4.png",
        bytes: include_bytes!("../../../../assets/ui/scenes/mara-harbor-v4.png"),
        digest: [
            73, 131, 190, 194, 29, 120, 124, 64, 103, 104, 106, 154, 68, 0, 236, 94, 5, 183, 195,
            221, 175, 182, 245, 166, 225, 136, 113, 198, 247, 118, 94, 162,
        ],
        mime: "image/png",
        width: 1672,
        height: 941,
    },
    PreparedAsset {
        route: "/assets/placeholder/scene-a.webp",
        bytes: &TINY_SCENE_A,
        digest: [
            98, 28, 182, 194, 123, 212, 16, 229, 182, 118, 82, 197, 121, 158, 87, 197, 183, 32, 76,
            73, 65, 145, 16, 88, 20, 252, 233, 104, 30, 165, 216, 136,
        ],
        mime: "image/webp",
        width: 16,
        height: 16,
    },
    PreparedAsset {
        route: "/assets/placeholder/avatar-b.webp",
        bytes: &TINY_SCENE_B,
        digest: [
            21, 133, 46, 232, 30, 57, 146, 119, 65, 53, 155, 12, 160, 192, 222, 99, 237, 12, 192,
            75, 173, 16, 174, 199, 190, 42, 12, 229, 48, 34, 230, 246,
        ],
        mime: "image/webp",
        width: 24,
        height: 16,
    },
    PreparedAsset {
        route: "/assets/placeholder/scene-c.webp",
        bytes: &TINY_SCENE_C,
        digest: [
            241, 48, 105, 254, 18, 10, 15, 12, 63, 54, 112, 133, 71, 123, 16, 132, 173, 104, 135,
            187, 67, 163, 139, 48, 2, 36, 101, 55, 78, 134, 249, 93,
        ],
        mime: "image/webp",
        width: 32,
        height: 24,
    },
];
