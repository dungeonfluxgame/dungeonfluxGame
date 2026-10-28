//! Unit tests for the private image-only codec.

use super::codec::{jpeg_size, webp_facts};

fn minimal_jpeg(width: u16, height: u16) -> Vec<u8> {
    vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x08, 0x08, (height >> 8) as u8, height as u8,
        (width >> 8) as u8, width as u8, 0x01, 0xff, 0xd9,
    ]
}

#[test]
fn a_minimal_jpeg_reports_its_frame_dimensions() {
    assert_eq!(jpeg_size(&minimal_jpeg(1024, 1536)), Some((1024, 1536)));
}

#[test]
fn a_truncated_jpeg_is_not_a_valid_output() {
    assert_eq!(jpeg_size(&[0xff, 0xd8, 0xff]), None);
    assert_eq!(jpeg_size(&[]), None);
}

#[test]
fn a_non_webp_container_is_not_a_valid_output() {
    assert_eq!(webp_facts(b"not a webp container"), None);
    assert_eq!(webp_facts(&[]), None);
}
