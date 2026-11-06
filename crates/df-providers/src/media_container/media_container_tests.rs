use super::{EncodedMediaFacts, crc32, inspect_mp4, inspect_png_off_worker};

#[test]
fn crc32_matches_the_standard_check_value_and_the_png_iend_chunk() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    // PNG stores the CRC of the chunk type and data; an empty IEND chunk is this constant.
    assert_eq!(crc32(b"IEND"), 0xae42_6082);
}

#[test]
fn crc32_detects_a_single_flipped_bit_in_a_long_chunk() {
    let mut chunk = vec![0x5au8; 65_536];
    let original = crc32(&chunk);
    chunk[40_000] ^= 0x01;
    assert_ne!(crc32(&chunk), original);
}

/// The previous bit-serial pass, kept only as the oracle for the table and as the "before"
/// timing.
fn crc32_bitwise_reference(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

/// A structurally valid 4096 x 4096 RGB PNG whose IDAT fills the 16 MiB response cap.
fn sixteen_mebibyte_png() -> Vec<u8> {
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&4096u32.to_be_bytes());
    header.extend_from_slice(&4096u32.to_be_bytes());
    // 8-bit depth, RGB colour type, deflate, adaptive filtering, no interlace.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let image_data = vec![0x5au8; 16 * 1024 * 1024 - 4096];
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(png_chunk(b"IHDR", &header));
    png.extend(png_chunk(b"IDAT", &image_data));
    png.extend(png_chunk(b"IEND", &[]));
    png
}

#[test]
fn table_crc_matches_the_bitwise_reference_on_a_sixteen_mebibyte_png() {
    let png = sixteen_mebibyte_png();
    assert_eq!(crc32(&png), crc32_bitwise_reference(&png));
}

/// Measurement and behaviour: the 16 MiB scan must not hold the async worker. Before the
/// change the scan ran inline, so no heartbeat could tick during it (zero ticks by
/// construction); after the change the heartbeat keeps ticking while the scan runs.
/// The printed timings are the before/after numbers for the round report.
#[tokio::test(flavor = "current_thread")]
async fn a_sixteen_mebibyte_png_scan_leaves_the_async_worker_free() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    use std::time::Instant;

    let png = sixteen_mebibyte_png();
    let length = png.len();

    let started = Instant::now();
    let bitwise_crc = crc32_bitwise_reference(&png);
    let before_crc = started.elapsed();

    let started = Instant::now();
    let table_crc = crc32(&png);
    let after_crc = started.elapsed();
    assert_eq!(bitwise_crc, table_crc);

    // A task that only yields counts the worker turns it gets while the scan runs elsewhere.
    // No sleeps: the count is what the worker did, not a wall-clock tick.
    let turns = Arc::new(AtomicU64::new(0));
    let heartbeat = {
        let turns = Arc::clone(&turns);
        tokio::spawn(async move {
            loop {
                tokio::task::yield_now().await;
                turns.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    let started = Instant::now();
    let (returned, facts) = inspect_png_off_worker(png)
        .await
        .expect("the generated PNG is valid");
    let after_scan = started.elapsed();
    heartbeat.abort();
    let observed_turns = turns.load(Ordering::SeqCst);

    eprintln!(
        "png 16 MiB ({length} bytes): before crc bitwise {before_crc:?} on worker; \
         after crc table {after_crc:?}; after off-worker scan {after_scan:?} with \
         {observed_turns} worker turns of other work"
    );
    assert_eq!(returned.len(), length);
    assert_eq!(
        facts,
        EncodedMediaFacts::Png {
            width: 4096,
            height: 4096
        }
    );
    assert!(
        observed_turns > 0,
        "the async worker must run other work during the scan"
    );
}

/// Development check, not a gate: the spike's real Kling clip must pass the container
/// inspection the adapter applies to every provider output. Runs only with
/// `DF_PROVIDER_MP4_SAMPLE`.
#[test]
#[ignore = "reads a provider sample; set DF_PROVIDER_MP4_SAMPLE"]
fn a_real_provider_clip_is_accepted_by_the_container_inspection() {
    let Some(path) = std::env::var_os("DF_PROVIDER_MP4_SAMPLE") else {
        return;
    };
    let bytes = std::fs::read(path).expect("the sample is readable");
    inspect_mp4(&bytes).expect("the provider clip is a valid MP4 for the adapter");
}
