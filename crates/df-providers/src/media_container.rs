//! Finite encoded-container inspection; no pixel, video-bitstream or audio playback decoding.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaContainerRefusal {
    Malformed,
    Unsupported,
    Bounds,
    Dimensions,
    Timebase,
    SampleRange,
}

/// Actual container clock; values are never copied from requested duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaTimebase {
    pub ticks: u64,
    pub ticks_per_second: u32,
}

impl MediaTimebase {
    pub fn milliseconds_ceil(self) -> Result<u128, MediaContainerRefusal> {
        if self.ticks_per_second == 0 {
            return Err(MediaContainerRefusal::Timebase);
        }
        let numerator = u128::from(self.ticks) * 1000;
        Ok(numerator.div_ceil(u128::from(self.ticks_per_second)))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodedMediaFacts {
    Png {
        width: u32,
        height: u32,
    },
    Mp4 {
        width: u32,
        height: u32,
        frames: u64,
        timebase: MediaTimebase,
    },
    PcmWave {
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
        frames: u64,
        timebase: MediaTimebase,
    },
}

fn bytes(data: &[u8], start: usize, length: usize) -> Result<&[u8], MediaContainerRefusal> {
    data.get(
        start
            ..start
                .checked_add(length)
                .ok_or(MediaContainerRefusal::Bounds)?,
    )
    .ok_or(MediaContainerRefusal::Malformed)
}

fn be32(data: &[u8], offset: usize) -> Result<u32, MediaContainerRefusal> {
    Ok(u32::from_be_bytes(
        bytes(data, offset, 4)?
            .try_into()
            .map_err(|_| MediaContainerRefusal::Malformed)?,
    ))
}

fn be64(data: &[u8], offset: usize) -> Result<u64, MediaContainerRefusal> {
    Ok(u64::from_be_bytes(
        bytes(data, offset, 8)?
            .try_into()
            .map_err(|_| MediaContainerRefusal::Malformed)?,
    ))
}

fn le16(data: &[u8], offset: usize) -> Result<u16, MediaContainerRefusal> {
    Ok(u16::from_le_bytes(
        bytes(data, offset, 2)?
            .try_into()
            .map_err(|_| MediaContainerRefusal::Malformed)?,
    ))
}

fn le32(data: &[u8], offset: usize) -> Result<u32, MediaContainerRefusal> {
    Ok(u32::from_le_bytes(
        bytes(data, offset, 4)?
            .try_into()
            .map_err(|_| MediaContainerRefusal::Malformed)?,
    ))
}

fn dimensions(width: u32, height: u32) -> Result<(), MediaContainerRefusal> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 16_777_216
    {
        return Err(MediaContainerRefusal::Dimensions);
    }
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

pub(crate) fn inspect_png(data: &[u8]) -> Result<EncodedMediaFacts, MediaContainerRefusal> {
    if bytes(data, 0, 8)? != b"\x89PNG\r\n\x1a\n" {
        return Err(MediaContainerRefusal::Malformed);
    }
    let mut position = 8usize;
    let mut size = None;
    let mut image_data = 0usize;
    let mut data_ended = false;
    for count in 0..65_536 {
        let length =
            usize::try_from(be32(data, position)?).map_err(|_| MediaContainerRefusal::Bounds)?;
        let kind = bytes(data, position + 4, 4)?;
        if !kind.iter().all(u8::is_ascii_alphabetic) || kind[2].is_ascii_lowercase() {
            return Err(MediaContainerRefusal::Malformed);
        }
        let chunk = bytes(data, position + 8, length)?;
        let crc_offset = position
            .checked_add(8)
            .and_then(|v| v.checked_add(length))
            .ok_or(MediaContainerRefusal::Bounds)?;
        if crc32(bytes(data, position + 4, length + 4)?) != be32(data, crc_offset)? {
            return Err(MediaContainerRefusal::Malformed);
        }
        position = crc_offset
            .checked_add(4)
            .ok_or(MediaContainerRefusal::Bounds)?;
        match kind {
            b"IHDR" => {
                if count != 0 || size.is_some() || length != 13 {
                    return Err(MediaContainerRefusal::Malformed);
                }
                let width = be32(chunk, 0)?;
                let height = be32(chunk, 4)?;
                dimensions(width, height)?;
                if !matches!(chunk[8], 8 | 16)
                    || !matches!(chunk[9], 0 | 2 | 4 | 6)
                    || chunk[10] != 0
                    || chunk[11] != 0
                    || chunk[12] != 0
                {
                    return Err(MediaContainerRefusal::Unsupported);
                }
                size = Some((width, height));
            }
            b"IDAT" => {
                if size.is_none() || data_ended {
                    return Err(MediaContainerRefusal::Malformed);
                }
                image_data = image_data
                    .checked_add(length)
                    .ok_or(MediaContainerRefusal::Bounds)?;
            }
            b"IEND" => {
                if length != 0 || image_data == 0 || position != data.len() {
                    return Err(MediaContainerRefusal::Malformed);
                }
                let (width, height) = size.ok_or(MediaContainerRefusal::Malformed)?;
                return Ok(EncodedMediaFacts::Png { width, height });
            }
            b"acTL" | b"fcTL" | b"fdAT" => return Err(MediaContainerRefusal::Unsupported),
            _ => {
                if kind[0].is_ascii_uppercase() {
                    return Err(MediaContainerRefusal::Unsupported);
                }
                if image_data > 0 {
                    data_ended = true;
                }
            }
        }
    }
    Err(MediaContainerRefusal::Bounds)
}

pub(crate) fn inspect_wave(data: &[u8]) -> Result<EncodedMediaFacts, MediaContainerRefusal> {
    if bytes(data, 0, 4)? != b"RIFF"
        || bytes(data, 8, 4)? != b"WAVE"
        || u64::from(le32(data, 4)?) + 8 != data.len() as u64
    {
        return Err(MediaContainerRefusal::Malformed);
    }
    let mut position = 12usize;
    let mut format = None;
    let mut data_length = None;
    let mut chunks = 0usize;
    while position < data.len() {
        chunks += 1;
        if chunks > 65_536 {
            return Err(MediaContainerRefusal::Bounds);
        }
        let kind = bytes(data, position, 4)?;
        let length = usize::try_from(le32(data, position + 4)?)
            .map_err(|_| MediaContainerRefusal::Bounds)?;
        let chunk = bytes(data, position + 8, length)?;
        match kind {
            b"fmt " => {
                if format.is_some() || length != 16 || le16(chunk, 0)? != 1 {
                    return Err(MediaContainerRefusal::Unsupported);
                }
                let channels = le16(chunk, 2)?;
                let rate = le32(chunk, 4)?;
                let byte_rate = le32(chunk, 8)?;
                let alignment = le16(chunk, 12)?;
                let bits = le16(chunk, 14)?;
                if !matches!(channels, 1 | 2)
                    || !(8000..=192000).contains(&rate)
                    || !matches!(bits, 8 | 16 | 24 | 32)
                    || alignment != channels * (bits / 8)
                    || byte_rate
                        != rate
                            .checked_mul(u32::from(alignment))
                            .ok_or(MediaContainerRefusal::Bounds)?
                {
                    return Err(MediaContainerRefusal::Timebase);
                }
                format = Some((channels, rate, bits, alignment));
            }
            b"data" => {
                if data_length.is_some() || format.is_none() || length == 0 {
                    return Err(MediaContainerRefusal::Malformed);
                }
                data_length = Some(length);
            }
            b"JUNK" | b"LIST" => {}
            _ => return Err(MediaContainerRefusal::Unsupported),
        }
        position = position
            .checked_add(8)
            .and_then(|v| v.checked_add(length))
            .ok_or(MediaContainerRefusal::Bounds)?;
        if length % 2 != 0 {
            if bytes(data, position, 1)? != [0] {
                return Err(MediaContainerRefusal::Malformed);
            }
            position += 1;
        }
    }
    let (channels, sample_rate, bits_per_sample, alignment) =
        format.ok_or(MediaContainerRefusal::Malformed)?;
    let length = data_length.ok_or(MediaContainerRefusal::Malformed)?;
    if position != data.len() || length % usize::from(alignment) != 0 {
        return Err(MediaContainerRefusal::Malformed);
    }
    let frames = u64::try_from(length / usize::from(alignment))
        .map_err(|_| MediaContainerRefusal::Bounds)?;
    Ok(EncodedMediaFacts::PcmWave {
        sample_rate,
        channels,
        bits_per_sample,
        frames,
        timebase: MediaTimebase {
            ticks: frames,
            ticks_per_second: sample_rate,
        },
    })
}

struct IsoBox<'a> {
    kind: &'a [u8],
    payload: &'a [u8],
    start: usize,
    end: usize,
}

fn boxes(data: &[u8], absolute: usize) -> Result<Vec<IsoBox<'_>>, MediaContainerRefusal> {
    let mut found = Vec::new();
    let mut position = 0usize;
    while position < data.len() {
        if found.len() == 65_536 {
            return Err(MediaContainerRefusal::Bounds);
        }
        let short = be32(data, position)?;
        let kind = bytes(data, position + 4, 4)?;
        let (length, header) = match short {
            0 => return Err(MediaContainerRefusal::Unsupported),
            1 => (
                usize::try_from(be64(data, position + 8)?)
                    .map_err(|_| MediaContainerRefusal::Bounds)?,
                16,
            ),
            v => (
                usize::try_from(v).map_err(|_| MediaContainerRefusal::Bounds)?,
                8,
            ),
        };
        if length < header {
            return Err(MediaContainerRefusal::Malformed);
        }
        let end = position
            .checked_add(length)
            .ok_or(MediaContainerRefusal::Bounds)?;
        let payload = bytes(data, position + header, length - header)?;
        found.push(IsoBox {
            kind,
            payload,
            start: absolute
                .checked_add(position + header)
                .ok_or(MediaContainerRefusal::Bounds)?,
            end: absolute
                .checked_add(end)
                .ok_or(MediaContainerRefusal::Bounds)?,
        });
        position = end;
    }
    Ok(found)
}

fn one<'a, 'b>(
    children: &'b [IsoBox<'a>],
    kind: &[u8],
) -> Result<&'b IsoBox<'a>, MediaContainerRefusal> {
    let mut selected = children.iter().filter(|child| child.kind == kind);
    let value = selected.next().ok_or(MediaContainerRefusal::Malformed)?;
    if selected.next().is_some() {
        return Err(MediaContainerRefusal::Malformed);
    }
    Ok(value)
}

fn only(children: &[IsoBox<'_>], allowed: &[&[u8]]) -> Result<(), MediaContainerRefusal> {
    if children.iter().any(|child| !allowed.contains(&child.kind)) {
        return Err(MediaContainerRefusal::Unsupported);
    }
    Ok(())
}

fn full_zero(data: &[u8]) -> Result<(), MediaContainerRefusal> {
    if bytes(data, 0, 4)? != [0, 0, 0, 0] {
        return Err(MediaContainerRefusal::Unsupported);
    }
    Ok(())
}

pub(crate) fn inspect_mp4(data: &[u8]) -> Result<EncodedMediaFacts, MediaContainerRefusal> {
    let top = boxes(data, 0)?;
    only(&top, &[b"ftyp", b"moov", b"mdat", b"free"])?;
    let file_type = one(&top, b"ftyp")?.payload;
    if file_type.len() < 8
        || file_type.len() % 4 != 0
        || ![b"isom".as_slice(), b"iso2", b"avc1", b"mp41", b"mp42"]
            .contains(&bytes(file_type, 0, 4)?)
    {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let movie = one(&top, b"moov")?;
    let media = one(&top, b"mdat")?;
    if media.payload.is_empty() {
        return Err(MediaContainerRefusal::Malformed);
    }
    let movie_children = boxes(movie.payload, movie.start)?;
    only(&movie_children, &[b"mvhd", b"trak", b"udta"])?;
    let movie_header = one(&movie_children, b"mvhd")?.payload;
    let movie_time = match bytes(movie_header, 0, 4)? {
        [0, 0, 0, 0] if movie_header.len() == 100 => MediaTimebase {
            ticks: u64::from(be32(movie_header, 16)?),
            ticks_per_second: be32(movie_header, 12)?,
        },
        [1, 0, 0, 0] if movie_header.len() == 112 => MediaTimebase {
            ticks: be64(movie_header, 24)?,
            ticks_per_second: be32(movie_header, 20)?,
        },
        _ => return Err(MediaContainerRefusal::Unsupported),
    };
    if movie_time.ticks == 0 || movie_time.ticks_per_second == 0 {
        return Err(MediaContainerRefusal::Timebase);
    }
    let track = one(&movie_children, b"trak")?;
    let track_children = boxes(track.payload, track.start)?;
    only(&track_children, &[b"tkhd", b"mdia"])?;
    let header = one(&track_children, b"tkhd")?.payload;
    let (matrix_offset, width_offset) = match bytes(header, 0, 1)? {
        [0] if header.len() == 84 => (40, 76),
        [1] if header.len() == 96 => (52, 88),
        _ => return Err(MediaContainerRefusal::Unsupported),
    };
    let matrix = [65536u32, 0, 0, 0, 65536, 0, 0, 0, 0x4000_0000];
    for (index, expected) in matrix.into_iter().enumerate() {
        if be32(header, matrix_offset + index * 4)? != expected {
            return Err(MediaContainerRefusal::Unsupported);
        }
    }
    let width_fixed = be32(header, width_offset)?;
    let height_fixed = be32(header, width_offset + 4)?;
    if width_fixed & 0xffff != 0 || height_fixed & 0xffff != 0 {
        return Err(MediaContainerRefusal::Dimensions);
    }
    let width = width_fixed >> 16;
    let height = height_fixed >> 16;
    dimensions(width, height)?;
    let mdia = one(&track_children, b"mdia")?;
    let mdia_children = boxes(mdia.payload, mdia.start)?;
    only(&mdia_children, &[b"mdhd", b"hdlr", b"minf"])?;
    let timing = one(&mdia_children, b"mdhd")?.payload;
    let timebase = match bytes(timing, 0, 4)? {
        [0, 0, 0, 0] if timing.len() == 24 => MediaTimebase {
            ticks: u64::from(be32(timing, 16)?),
            ticks_per_second: be32(timing, 12)?,
        },
        [1, 0, 0, 0] if timing.len() == 36 => MediaTimebase {
            ticks: be64(timing, 24)?,
            ticks_per_second: be32(timing, 20)?,
        },
        _ => return Err(MediaContainerRefusal::Unsupported),
    };
    if timebase.ticks == 0 || timebase.ticks_per_second == 0 {
        return Err(MediaContainerRefusal::Timebase);
    }
    let track_duration = if header[0] == 0 {
        u64::from(be32(header, 20)?)
    } else {
        be64(header, 28)?
    };
    if u128::from(movie_time.ticks) * u128::from(timebase.ticks_per_second)
        != u128::from(timebase.ticks) * u128::from(movie_time.ticks_per_second)
        || track_duration != movie_time.ticks
    {
        return Err(MediaContainerRefusal::Timebase);
    }
    let handler = one(&mdia_children, b"hdlr")?.payload;
    full_zero(handler)?;
    if bytes(handler, 8, 4)? != b"vide" {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let minf = one(&mdia_children, b"minf")?;
    let minf_children = boxes(minf.payload, minf.start)?;
    only(&minf_children, &[b"vmhd", b"dinf", b"stbl"])?;
    let vmhd = one(&minf_children, b"vmhd")?.payload;
    if vmhd.len() != 12 || bytes(vmhd, 0, 4)? != [0, 0, 0, 1] {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let dinf = one(&minf_children, b"dinf")?;
    let dinf_children = boxes(dinf.payload, dinf.start)?;
    only(&dinf_children, &[b"dref"])?;
    let dref = one(&dinf_children, b"dref")?;
    full_zero(dref.payload)?;
    if be32(dref.payload, 4)? != 1 {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let refs = boxes(
        bytes(dref.payload, 8, dref.payload.len().saturating_sub(8))?,
        dref.start + 8,
    )?;
    let reference = one(&refs, b"url ")?;
    if refs.len() != 1 || reference.payload != [0, 0, 0, 1] {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let stbl = one(&minf_children, b"stbl")?;
    let tables = boxes(stbl.payload, stbl.start)?;
    only(
        &tables,
        &[
            b"stsd", b"stts", b"stsc", b"stsz", b"stco", b"co64", b"stss", b"ctts",
        ],
    )?;
    let description = one(&tables, b"stsd")?;
    full_zero(description.payload)?;
    if be32(description.payload, 4)? != 1 {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let entries = boxes(
        bytes(
            description.payload,
            8,
            description.payload.len().saturating_sub(8),
        )?,
        description.start + 8,
    )?;
    let codec = one(&entries, b"avc1")?;
    if entries.len() != 1
        || codec.payload.len() < 78
        || bytes(codec.payload, 0, 6)? != [0; 6]
        || bytes(codec.payload, 6, 2)? != [0, 1]
        || u32::from(u16::from_be_bytes(
            bytes(codec.payload, 24, 2)?
                .try_into()
                .map_err(|_| MediaContainerRefusal::Malformed)?,
        )) != width
        || u32::from(u16::from_be_bytes(
            bytes(codec.payload, 26, 2)?
                .try_into()
                .map_err(|_| MediaContainerRefusal::Malformed)?,
        )) != height
    {
        return Err(MediaContainerRefusal::Dimensions);
    }
    let codec_children = boxes(
        bytes(codec.payload, 78, codec.payload.len() - 78)?,
        codec.start + 78,
    )?;
    only(&codec_children, &[b"avcC", b"btrt", b"pasp"])?;
    let configuration = one(&codec_children, b"avcC")?.payload;
    validate_avcc(configuration)?;
    if codec_children.iter().any(|c| {
        c.kind == b"pasp"
            && (c.payload.len() != 8 || be32(c.payload, 0).ok() != be32(c.payload, 4).ok())
    }) {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let stsz = one(&tables, b"stsz")?.payload;
    full_zero(stsz)?;
    let common_size = be32(stsz, 4)?;
    let frames = usize::try_from(be32(stsz, 8)?).map_err(|_| MediaContainerRefusal::Bounds)?;
    if frames == 0
        || frames > 65_536
        || stsz.len() != 12 + if common_size == 0 { frames * 4 } else { 0 }
    {
        return Err(MediaContainerRefusal::Bounds);
    }
    let mut sizes = Vec::with_capacity(frames);
    for i in 0..frames {
        let size = if common_size == 0 {
            be32(stsz, 12 + i * 4)?
        } else {
            common_size
        };
        if size == 0 {
            return Err(MediaContainerRefusal::SampleRange);
        }
        sizes.push(u64::from(size));
    }
    let stts = one(&tables, b"stts")?.payload;
    let timing_entries = table_count(stts, 8)?;
    let mut counted_frames = 0u64;
    let mut counted_ticks = 0u64;
    for i in 0..timing_entries {
        let count = u64::from(be32(stts, 8 + i * 8)?);
        let delta = u64::from(be32(stts, 12 + i * 8)?);
        if count == 0 || delta == 0 {
            return Err(MediaContainerRefusal::Timebase);
        }
        counted_frames = counted_frames
            .checked_add(count)
            .ok_or(MediaContainerRefusal::Bounds)?;
        counted_ticks = counted_ticks
            .checked_add(
                count
                    .checked_mul(delta)
                    .ok_or(MediaContainerRefusal::Bounds)?,
            )
            .ok_or(MediaContainerRefusal::Bounds)?;
    }
    if counted_frames != frames as u64 || counted_ticks != timebase.ticks {
        return Err(MediaContainerRefusal::Timebase);
    }
    if let Some(ctts) = tables.iter().find(|c| c.kind == b"ctts") {
        one(&tables, b"ctts")?;
        let count = table_count(ctts.payload, 8)?;
        let mut samples = 0u64;
        for i in 0..count {
            samples = samples
                .checked_add(u64::from(be32(ctts.payload, 8 + i * 8)?))
                .ok_or(MediaContainerRefusal::Bounds)?;
            // Nonzero offsets need presentation/edit-list policy, not a desired-duration guess.
            if be32(ctts.payload, 12 + i * 8)? != 0 {
                return Err(MediaContainerRefusal::Unsupported);
            }
        }
        if samples != frames as u64 {
            return Err(MediaContainerRefusal::Timebase);
        }
    }
    if let Some(stss) = tables.iter().find(|c| c.kind == b"stss") {
        one(&tables, b"stss")?;
        let count = table_count(stss.payload, 4)?;
        let mut previous = 0;
        for i in 0..count {
            let sample = be32(stss.payload, 8 + i * 4)?;
            if sample <= previous || sample as usize > frames {
                return Err(MediaContainerRefusal::SampleRange);
            }
            previous = sample;
        }
    }
    let offsets_box = match (
        tables.iter().find(|c| c.kind == b"stco"),
        tables.iter().find(|c| c.kind == b"co64"),
    ) {
        (Some(value), None) => {
            one(&tables, b"stco")?;
            value
        }
        (None, Some(value)) => {
            one(&tables, b"co64")?;
            value
        }
        _ => return Err(MediaContainerRefusal::Malformed),
    };
    let offset_width = if offsets_box.kind == b"stco" { 4 } else { 8 };
    let chunks = table_count(offsets_box.payload, offset_width)?;
    let stsc = one(&tables, b"stsc")?.payload;
    let mapping_count = table_count(stsc, 12)?;
    let mut mappings = Vec::with_capacity(mapping_count);
    for i in 0..mapping_count {
        let first = be32(stsc, 8 + i * 12)? as usize;
        let per_chunk = be32(stsc, 12 + i * 12)? as usize;
        if first == 0
            || first > chunks
            || per_chunk == 0
            || per_chunk > frames
            || be32(stsc, 16 + i * 12)? != 1
            || mappings
                .last()
                .is_some_and(|(previous, _)| *previous >= first)
        {
            return Err(MediaContainerRefusal::SampleRange);
        }
        mappings.push((first, per_chunk));
    }
    if mappings.first().map(|(first, _)| *first) != Some(1) {
        return Err(MediaContainerRefusal::SampleRange);
    }
    let mut sample_index = 0usize;
    let mut mapping_index = 0usize;
    let mut previous_end = media.start as u64;
    for i in 0..chunks {
        let offset = if offset_width == 4 {
            u64::from(be32(offsets_box.payload, 8 + i * 4)?)
        } else {
            be64(offsets_box.payload, 8 + i * 8)?
        };
        // Sorted first-chunk entries are consumed once, bounding table work linearly.
        while mappings
            .get(mapping_index + 1)
            .is_some_and(|(first, _)| *first <= i + 1)
        {
            mapping_index += 1;
        }
        let per_chunk = mappings[mapping_index].1;
        let end_index = sample_index
            .checked_add(per_chunk)
            .ok_or(MediaContainerRefusal::Bounds)?;
        let chunk_sizes = sizes
            .get(sample_index..end_index)
            .ok_or(MediaContainerRefusal::SampleRange)?;
        let length = chunk_sizes.iter().try_fold(0u64, |sum, size| {
            sum.checked_add(*size).ok_or(MediaContainerRefusal::Bounds)
        })?;
        let end = offset
            .checked_add(length)
            .ok_or(MediaContainerRefusal::Bounds)?;
        if offset < previous_end || offset < media.start as u64 || end > media.end as u64 {
            return Err(MediaContainerRefusal::SampleRange);
        }
        previous_end = end;
        sample_index = end_index;
    }
    if sample_index != frames {
        return Err(MediaContainerRefusal::SampleRange);
    }
    Ok(EncodedMediaFacts::Mp4 {
        width,
        height,
        frames: frames as u64,
        timebase,
    })
}

fn table_count(data: &[u8], width: usize) -> Result<usize, MediaContainerRefusal> {
    full_zero(data)?;
    let count = usize::try_from(be32(data, 4)?).map_err(|_| MediaContainerRefusal::Bounds)?;
    if count == 0 || count > 65_536 || data.len() != 8 + count * width {
        return Err(MediaContainerRefusal::Bounds);
    }
    Ok(count)
}

fn validate_avcc(data: &[u8]) -> Result<(), MediaContainerRefusal> {
    if data.len() < 7
        || data[0] != 1
        || data[4] & 0xfc != 0xfc
        || data[4] & 3 != 3
        || data[5] & 0xe0 != 0xe0
        || data[5] & 0x1f == 0
    {
        return Err(MediaContainerRefusal::Unsupported);
    }
    let mut position = 6usize;
    for _ in 0..data[5] & 0x1f {
        let length = u16::from_be_bytes(
            bytes(data, position, 2)?
                .try_into()
                .map_err(|_| MediaContainerRefusal::Malformed)?,
        ) as usize;
        position += 2;
        if length == 0 || bytes(data, position, length)?[0] & 0x1f != 7 {
            return Err(MediaContainerRefusal::Malformed);
        }
        position += length;
    }
    let count = bytes(data, position, 1)?[0];
    position += 1;
    if count == 0 {
        return Err(MediaContainerRefusal::Malformed);
    }
    for _ in 0..count {
        let length = u16::from_be_bytes(
            bytes(data, position, 2)?
                .try_into()
                .map_err(|_| MediaContainerRefusal::Malformed)?,
        ) as usize;
        position += 2;
        if length == 0 || bytes(data, position, length)?[0] & 0x1f != 8 {
            return Err(MediaContainerRefusal::Malformed);
        }
        position += length;
    }
    if position != data.len() {
        return Err(MediaContainerRefusal::Unsupported);
    }
    Ok(())
}
