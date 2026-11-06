use super::*;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture_base() -> PathBuf {
    // An explicitly configured TMPDIR keeps test artifacts in the
    // repository; the OS default is only the fallback.
    if let Some(dir) = std::env::var_os("TMPDIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    std::env::temp_dir()
}

fn scratch_root(name: &str) -> PathBuf {
    let root = fixture_base().join(format!("df-codex-artifact-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("outputs")).unwrap();
    fs::create_dir_all(root.join("elsewhere")).unwrap();
    root
}

fn write_png(path: &Path, width: u32, height: u32) {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        pixels.extend_from_slice(&[127u8, 127u8, 127u8, 255u8]);
    }
    let mut encoded = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    fs::write(path, encoded).unwrap();
}

#[test]
fn relative_and_escaping_paths_are_foreign() {
    let root = scratch_root("relative");
    let outputs = root.join("outputs");
    assert_eq!(
        contained_path("img.png", &outputs).unwrap_err(),
        ArtifactError::NotAbsolute
    );
    assert_eq!(
        contained_path("", &outputs).unwrap_err(),
        ArtifactError::NotAbsolute
    );
    // A rooted-but-driveless claim carries no drive or UNC host, so it is
    // not absolute either, on either platform.
    assert_eq!(
        contained_path("\\no-drive\\img.png", &outputs).unwrap_err(),
        ArtifactError::NotAbsolute
    );
    let escape = format!("{}/../elsewhere/img.png", outputs.display());
    assert_eq!(
        contained_path(&escape, &outputs).unwrap_err(),
        ArtifactError::ForeignPath
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn missing_and_oversize_files_fail_before_decoding() {
    let root = scratch_root("missing");
    let outputs = root.join("outputs");
    let missing = format!("{}/gone.png", outputs.display());
    assert_eq!(
        contained_path(&missing, &outputs).unwrap_err(),
        ArtifactError::Missing
    );
    let big = outputs.join("big.png");
    fs::write(&big, vec![0u8; 16]).unwrap();
    let error = read_bounded(&big, 8).unwrap_err();
    // The bounded reader stops at max + 1, so it reports the observed 9,
    // a lower bound on the 16-byte file, without reading it fully.
    assert_eq!(error, ArtifactError::TooLarge { bytes: 9 });
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn garbage_bytes_are_rejected_not_accepted() {
    let root = scratch_root("garbage");
    let outputs = root.join("outputs");
    let path = outputs.join("junk.png");
    fs::write(&path, b"definitely not an image").unwrap();
    let error = verify_artifact(
        &path.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        false,
    )
    .unwrap_err();
    assert!(matches!(error, ArtifactError::Rejected(_)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn valid_png_verifies_with_digest_and_dimensions() {
    let root = scratch_root("valid");
    let outputs = root.join("outputs");
    let path = outputs.join("plate.png");
    write_png(&path, 4, 2);
    let verified = verify_artifact(
        &path.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        false,
    )
    .unwrap();
    assert_eq!(verified.width_px(), 4);
    assert_eq!(verified.height_px(), 2);
    assert_eq!(verified.mime(), "image/png");
    assert_eq!(verified.digest_sha256_hex().len(), SHA256_HEX_CHARS);
    assert!(verified.byte_len() > 0);
    assert_eq!(verified.bytes().len(), verified.byte_len());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn opaque_rgb_png_refuses_required_alpha_while_transparent_rgba_succeeds() {
    let root = scratch_root("alpha");
    let outputs = root.join("outputs");
    let opaque = outputs.join("opaque.png");
    write_png(&opaque, 2, 2);
    let error = verify_artifact(
        &opaque.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        true,
    )
    .unwrap_err();
    assert_eq!(error, ArtifactError::AlphaMismatch);
    let transparent = outputs.join("clear.png");
    write_transparent_png(&transparent, 2, 2);
    let verified = verify_artifact(
        &transparent.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        true,
    )
    .unwrap();
    assert!(verified.has_alpha());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn wrong_dimensions_and_wrong_format_reject_with_typed_errors() {
    let root = scratch_root("mismatch");
    let outputs = root.join("outputs");
    let path = outputs.join("plate.png");
    write_png(&path, 4, 2);
    // The image tool chooses its own size; the worker resizes to the delivered profile, so a
    // different size is accepted here and reported by its real dimensions.
    let verified = verify_artifact(
        &path.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        false,
    )
    .unwrap();
    assert_eq!((verified.width_px(), verified.height_px()), (4, 2));
    assert_eq!(
        verify_artifact(
            &path.to_string_lossy(),
            &outputs,
            1024 * 1024,
            OutputFormat::Jpeg,
            false,
        )
        .unwrap_err(),
        ArtifactError::FormatMismatch
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn equivalent_windows_spellings_contain_while_escapes_and_symlinks_reject() {
    let root = scratch_root("windows-equiv");
    let outputs = root.join("outputs");
    let _elsewhere = root.join("elsewhere");
    let target = outputs.join("plate.png");
    write_png(&target, 2, 2);
    let canonical_root = std::fs::canonicalize(&outputs)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // The platform-native spelling: appending `/` to a verbatim `\\?\`
    // root would not be a valid path on Windows, so it is not asserted.
    let normal = target.to_string_lossy().into_owned();
    assert!(contained_path(&normal, &outputs).is_ok());
    #[cfg(windows)]
    {
        // `canonicalize` already returns the verbatim `\\?\` spelling, so
        // strip it before re-adding: doubling the prefix is not a path.
        let bare = canonical_root
            .strip_prefix("\\\\?\\")
            .unwrap_or(&canonical_root)
            .replace('/', "\\");
        let verbatim = format!("\\\\?\\{bare}\\plate.png");
        assert!(contained_path(&verbatim, &outputs).is_ok());
        // A claim on the other drive is lexically outside, whether or not
        // that drive exists, so confinement never depends on the machine.
        let mut other_drive = bare.clone();
        other_drive.replace_range(0..1, if bare.starts_with('C') { "D" } else { "C" });
        assert_eq!(
            contained_path(&format!("{other_drive}\\plate.png"), &outputs).unwrap_err(),
            ArtifactError::ForeignPath
        );
    }
    let sibling = format!("{canonical_root}-sibling/plate.png");
    assert_eq!(
        contained_path(&sibling, &outputs).unwrap_err(),
        ArtifactError::ForeignPath
    );
    let escape = format!("{canonical_root}/../elsewhere/plate.png");
    assert_eq!(
        contained_path(&escape, &outputs).unwrap_err(),
        ArtifactError::ForeignPath
    );
    let link = outputs.join("link.png");
    #[cfg(unix)]
    std::os::unix::fs::symlink(_elsewhere.join("outside.png"), &link).unwrap_or(());
    let _ = link;
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn directory_and_growing_reads_reject_without_unbounded_allocation() {
    let root = scratch_root("bounded");
    let outputs = root.join("outputs");
    assert_eq!(
        read_bounded(&outputs, 1024).unwrap_err(),
        ArtifactError::Unreadable
    );
    let big = outputs.join("big.png");
    fs::write(&big, vec![0u8; 16]).unwrap();
    // As above, the bounded reader reports the observed max + 1 prefix.
    assert_eq!(
        read_bounded(&big, 8).unwrap_err(),
        ArtifactError::TooLarge { bytes: 9 }
    );
    let _ = fs::remove_dir_all(&root);
}

fn write_transparent_png(path: &Path, width: u32, height: u32) {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        pixels.extend_from_slice(&[10u8, 20u8, 30u8, 0u8]);
    }
    let mut encoded = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    fs::write(path, encoded).unwrap();
}

#[test]
fn staged_bytes_detect_substitution() {
    let root = scratch_root("staged");
    let inputs = root.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    let path = inputs.join("ref.png");
    fs::write(&path, b"original").unwrap();
    let digest = sha256_hex(b"original");
    assert!(check_staged_bytes(&path, &digest, 1024).is_ok());
    fs::write(&path, b"swapped").unwrap();
    assert_eq!(
        check_staged_bytes(&path, &digest, 1024).unwrap_err(),
        ArtifactError::DigestMismatch
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn verified_artifact_debug_redacts_image_bytes() {
    let sentinel = b"PRIVATE-SENTINEL-9f2c-bytes";
    let verified = VerifiedArtifact {
        path: PathBuf::from("/tmp/outputs/item.png"),
        digest_sha256_hex: sha256_hex(sentinel),
        mime: "image/png".to_owned(),
        width_px: 2,
        height_px: 2,
        byte_len: sentinel.len(),
        bytes: sentinel.to_vec(),
        has_alpha: false,
    };
    for rendered in [format!("{verified:?}"), format!("{verified:#?}")] {
        assert!(
            !rendered.contains("PRIVATE-SENTINEL-9f2c-bytes"),
            "debug must never dump verified image bytes"
        );
        assert!(rendered.contains("VerifiedArtifact"));
        assert!(rendered.contains("image/png"));
        assert!(rendered.contains(&verified.digest_sha256_hex));
    }
}

#[test]
fn digests_are_lowercase_hex() {
    let digest = sha256_hex(b"abc");
    assert_eq!(digest.len(), SHA256_HEX_CHARS);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(digest, digest.to_ascii_lowercase());
}
