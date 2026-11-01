//! Requested heap bound for libwebp 1.5.0 method 4, one partition,
//! low-memory, single-threaded, no autofilter, raw alpha at quality 100.
//!
//! Sums allocation families even when their lifetimes do not overlap. Codec
//! stack and allocator bookkeeping are outside this requested-byte estimate.

// `vp8i_enc.h`: sizeof(VP8Encoder) on the largest supported native ABI
// (64-bit MSVC) is 23704; pointers/alignment and MSVC bitfields are no smaller
// than the supported 32/64-bit Unix ABIs. Revalidate on a library/ABI change.
const ENCODER_STRUCT_BYTES: u64 = 23_704;
// `utils/utils.h`: WEBP_ALIGN_CST padding at each of three allocation seams.
const ALIGNMENT_PADDING_BYTES: u64 = 31;
// `vp8i_enc.h`: MSVC unsigned bitfield unit plus alpha byte and padding.
const MACROBLOCK_INFO_BYTES: u64 = 8;
// `bit_writer_utils.c:26-49`: minimum allocation for every VP8 bitwriter.
const MINIMUM_BITWRITER_BYTES: u64 = 1_024;
// `tree_enc.c:485-504`: 4 types * 8 bands * 3 contexts * 11 probabilities;
// each update uses at most seven renormalization bits and eight uniform bits.
const PROBABILITY_UPDATE_BITS: u64 = 4 * 8 * 3 * 11 * (7 + 8);
// `syntax_enc.c:185-293`: colorspace, segmentation (99), filter (27),
// partitions, quantization (37), update flag, and skip-probability (9).
const FRAME_HEADER_BITS: u64 = 2 + 99 + 27 + 2 + 37 + 1 + PROBABILITY_UPDATE_BITS + 9;
// `tree_enc.c:262-347`: at most nine tree decisions per each of 16 i4 modes,
// two segment decisions, skip, block type, three UV decisions; <=7 bits each.
const MACROBLOCK_MODE_BITS: u64 = (16 * 9 + 2 + 1 + 1 + 3) * 7;
// `frame_enc.c:236-351`: at most 25 residual blocks, initial EOB, 16 levels.
// Each level has <=18 probability calls (including 11 Cat6 and EOB) plus
// one uniform sign. `bit_writer_utils.c:109-143` emits <=7 bits/probability.
const MACROBLOCK_RESIDUAL_BITS: u64 = 25 * (7 + 16 * (18 * 7 + 1));
// `VP8BitWriterFinish`: <=17 uniform padding bits and a final byte flush.
const BITWRITER_FINISH_BYTES: u64 = 4;
// `syntax_enc.c`: RIFF 12 + VP8X 18 + ALPH header 8 + alpha padding 1
// + VP8 chunk header 8 + frame header 10 + VP8 padding 1. Raw alpha has
// its own one-byte header, already included in alpha_payload_bytes below.
const CONTAINER_HEADER_BYTES: u64 = 12 + 18 + 8 + 1 + 8 + 10 + 1;

/// Bounds requested native heap plus both sides of possible Rust output
/// reallocation. Caller-retained RGBA/RGB/matte buffers are added by admission.
pub(super) fn estimate_peak_bytes(
    width_px: u32,
    height_px: u32,
    has_alpha: bool,
    max_output_bytes: u64,
) -> u64 {
    let width_px = u64::from(width_px);
    let height_px = u64::from(height_px);
    let pixels = width_px.saturating_mul(height_px);
    let chroma_width_px = width_px.div_ceil(2);
    let chroma_height_px = height_px.div_ceil(2);
    let macroblock_width = width_px.div_ceil(16);
    let macroblock_height = height_px.div_ceil(16);
    let macroblocks = macroblock_width.saturating_mul(macroblock_height);
    let alpha_bytes = if has_alpha { pixels } else { 0 };
    // `picture_enc.c:104-136`: exact YUV420(A) plane allocation.
    let planes_bytes = pixels
        .saturating_add(2 * chroma_width_px.saturating_mul(chroma_height_px))
        .saturating_add(alpha_bytes);
    // `picture_csp_enc.c:521-522`: four u16 values per chroma-column.
    let import_bytes = chroma_width_px.saturating_mul(8);
    let encoder_bytes = estimate_encoder_bytes(macroblock_width, macroblock_height);
    let header_bytes = FRAME_HEADER_BITS
        .saturating_add(macroblocks.saturating_mul(MACROBLOCK_MODE_BITS))
        .div_ceil(8)
        .saturating_add(BITWRITER_FINISH_BYTES);
    let residual_bytes = macroblocks
        .saturating_mul(MACROBLOCK_RESIDUAL_BITS)
        .div_ceil(8)
        .saturating_add(BITWRITER_FINISH_BYTES);
    // `frame_enc.c:675-688`: initial coefficient allocation <=50 bytes/MB.
    let residual_initial_bytes = macroblocks.saturating_mul(50);
    let writers_bytes = estimate_bitwriter_bytes(header_bytes).saturating_add(
        estimate_bitwriter_bytes(residual_bytes.max(residual_initial_bytes)),
    );
    // `alpha_enc.c:160-173,332-365`: extraction plus raw payload. Filtering
    // is forced off for raw alpha, and the exact payload is allocated once.
    let alpha_payload_bytes = if has_alpha {
        pixels.saturating_add(1)
    } else {
        0
    };
    let alpha_writer_bytes = if has_alpha {
        alpha_payload_bytes.max(MINIMUM_BITWRITER_BYTES)
    } else {
        0
    };
    let encoded_bytes = CONTAINER_HEADER_BYTES
        .saturating_add(header_bytes)
        .saturating_add(residual_bytes)
        .saturating_add(alpha_payload_bytes);
    // try_reserve_exact requests no more than the bounded output; realloc
    // may hold the old and new buffers concurrently. Allocator slack excluded.
    let output_bytes = max_output_bytes.min(encoded_bytes).saturating_mul(2);
    planes_bytes
        .saturating_add(import_bytes)
        .saturating_add(encoder_bytes)
        .saturating_add(macroblocks) // `analysis_enc.c:33`: segment-map scratch.
        .saturating_add(alpha_bytes)
        .saturating_add(alpha_writer_bytes)
        .saturating_add(writers_bytes)
        .saturating_add(output_bytes)
}

fn estimate_encoder_bytes(macroblock_width: u64, macroblock_height: u64) -> u64 {
    // `webp_enc.c:149-174`: struct, MB info, predictions, top samples,
    // top diffusion error (present through q98), nonzero context, padding.
    // Default autofilter=0 has no LFStats allocation; low_memory=1 disables tokens.
    let macroblocks = macroblock_width.saturating_mul(macroblock_height);
    let predictions_bytes = (4 * macroblock_width + 1).saturating_mul(4 * macroblock_height + 1);
    ENCODER_STRUCT_BYTES
        .saturating_add(3 * ALIGNMENT_PADDING_BYTES)
        .saturating_add(macroblocks.saturating_mul(MACROBLOCK_INFO_BYTES))
        .saturating_add(predictions_bytes)
        .saturating_add(macroblock_width.saturating_mul(32 + 4))
        .saturating_add((macroblock_width + 1).saturating_mul(4))
}

fn estimate_bitwriter_bytes(maximum_written_bytes: u64) -> u64 {
    // `bit_writer_utils.c:26-49`: at growth old < needed <= logical bound;
    // new=max(2*old,needed,1024), so old+new <=3*max(bound,1024).
    maximum_written_bytes
        .max(MINIMUM_BITWRITER_BYTES)
        .saturating_mul(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_bound_covers_observed_native_allocations_and_rust_output() {
        for (width, height, alpha, native_peak_bytes, output_bytes) in [
            (1, 1, false, 25_921, 68),
            (1, 1, true, 26_946, 96),
            (7, 5, true, 27_036, 262),
            (16, 16, true, 27_582, 768),
            (16_383, 1, false, 179_453, 46_446),
            (1, 16_383, false, 142_625, 53_582),
            (1_024, 1_024, true, 6_252_539, 2_216_032),
        ] {
            let bound = estimate_peak_bytes(width, height, alpha, output_bytes);
            assert!(bound >= native_peak_bytes + 2 * output_bytes);
        }
    }

    #[test]
    fn dimensions_round_chroma_and_macroblocks_up() {
        assert_eq!(estimate_encoder_bytes(1, 1), 23_874);
        assert!(estimate_peak_bytes(17, 1, false, 0) > estimate_peak_bytes(16, 1, false, 0));
        assert!(estimate_peak_bytes(1, 17, false, 0) > estimate_peak_bytes(1, 16, false, 0));
        assert!(estimate_peak_bytes(7, 5, true, 0) > estimate_peak_bytes(7, 5, false, 0));
    }

    #[test]
    fn output_bound_counts_both_sides_of_reallocation() {
        let native_bytes = estimate_peak_bytes(1, 1, false, 0);
        assert_eq!(
            estimate_peak_bytes(1, 1, false, 1_024),
            native_bytes + 2_048
        );
        assert_eq!(estimate_bitwriter_bytes(1), 3_072);
        assert_eq!(estimate_bitwriter_bytes(2_048), 6_144);
    }
}
