use crate::error::Error;

/// VP8X feature-flag bits for the metadata chunks `--wipe` removes.
const FLAG_ICC: u8 = 0x20;
const FLAG_EXIF: u8 = 0x08;
const FLAG_XMP: u8 = 0x04;
const WIPE_FLAGS: u8 = FLAG_ICC | FLAG_EXIF | FLAG_XMP;

/// Metadata chunks removed by `--wipe`.
const WIPE_CHUNKS: [[u8; 4]; 3] = [*b"EXIF", *b"ICCP", *b"XMP "];

/// Image-data chunks covered by post-write verification. `VP8X` is
/// deliberately excluded: it is a container header whose feature flags
/// legitimately change on metadata writes.
const IMAGE_CHUNKS: [[u8; 4]; 4] = [*b"VP8 ", *b"VP8L", *b"ALPH", *b"ANMF"];

pub struct Chunk {
    pub fourcc: [u8; 4],
    pub data: Vec<u8>,
}

/// Parse the RIFF chunk stream, validating its structure. Bytes past the
/// declared RIFF size are ignored.
pub fn parse(bytes: &[u8]) -> Result<Vec<Chunk>, Error> {
    if bytes.len() < 12 || bytes[..4] != *b"RIFF" || bytes[8..12] != *b"WEBP" {
        return Err(Error::runtime("corrupt WebP file: bad header"));
    }
    let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = 8usize.checked_add(declared).ok_or_else(|| Error::runtime("corrupt WebP file: declared size overflows"))?;
    if end > bytes.len() {
        return Err(Error::runtime("corrupt WebP file: truncated file"));
    }
    let mut chunks = Vec::new();
    let mut pos = 12;
    while pos < end {
        let header = bytes.get(pos..pos + 8).ok_or_else(|| Error::runtime("corrupt WebP file: truncated chunk header"))?;
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&header[..4]);
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data_start = pos + 8;
        let data_end = data_start.checked_add(length).ok_or_else(|| Error::runtime("corrupt WebP file: chunk length overflows"))?;
        if data_end > end {
            return Err(Error::runtime("corrupt WebP file: truncated chunk data"));
        }
        chunks.push(Chunk { fourcc, data: bytes[data_start..data_end].to_vec() });
        pos = data_end + (length % 2);
    }
    Ok(chunks)
}

/// Serialize chunks back into a file, recomputing the RIFF size.
pub fn encode(chunks: &[Chunk]) -> Vec<u8> {
    let mut out = Vec::from(&b"RIFF....WEBP"[..]);
    for chunk in chunks {
        out.extend_from_slice(&chunk.fourcc);
        out.extend_from_slice(&(chunk.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&chunk.data);
        if chunk.data.len() % 2 == 1 {
            out.push(0);
        }
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}

/// Remove the `EXIF` chunk and clear its `VP8X` feature flag, preserving
/// everything else. Used to pre-strip the file before a `little_exif` write:
/// its own WebP clear corrupts the 8 bytes preceding the EXIF chunk, while
/// its insert path is sound — so it must only ever see EXIF-less input.
pub fn strip_exif(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut chunks = parse(bytes)?;
    chunks.retain(|chunk| chunk.fourcc != *b"EXIF");
    if let Some(vp8x) = chunks.iter_mut().find(|chunk| chunk.fourcc == *b"VP8X") {
        let Some(flags) = vp8x.data.first_mut() else {
            return Err(Error::runtime("corrupt WebP file: empty VP8X chunk"));
        };
        *flags &= !FLAG_EXIF;
    }
    Ok(encode(&chunks))
}

/// Promote a simple (`VP8`/`VP8L`-first) file to the extended format by
/// inserting a `VP8X` chunk with the canvas size parsed from the image
/// bitstream. Extended files pass through unchanged. `little_exif` cannot
/// promote lossy `VP8` itself, so this runs before every WebP EXIF write.
pub fn promote(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut chunks = parse(bytes)?;
    if chunks.first().is_some_and(|chunk| chunk.fourcc == *b"VP8X") {
        return Ok(bytes.to_vec());
    }
    let Some(first) = chunks.first() else {
        return Err(Error::runtime("corrupt WebP file: no chunks"));
    };
    let (width, height) = match &first.fourcc {
        b"VP8 " => lossy_dimensions(&first.data)?,
        b"VP8L" => lossless_dimensions(&first.data)?,
        _ => return Err(Error::runtime("corrupt WebP file: expected a VP8/VP8L chunk")),
    };
    let mut vp8x = vec![0u8; 10];
    let canvas = |size: u32| [(size - 1) as u8, ((size - 1) >> 8) as u8, ((size - 1) >> 16) as u8];
    vp8x[4..7].copy_from_slice(&canvas(width));
    vp8x[7..10].copy_from_slice(&canvas(height));
    chunks.insert(0, Chunk { fourcc: *b"VP8X", data: vp8x });
    Ok(encode(&chunks))
}

/// Parse canvas dimensions from a lossy `VP8` bitstream: 3-byte frame tag,
/// `9D 012A` start code, then 14-bit width/height.
fn lossy_dimensions(payload: &[u8]) -> Result<(u32, u32), Error> {
    if payload.len() < 10 || payload[3..6] != [0x9D, 0x01, 0x2A] {
        return Err(Error::runtime("corrupt WebP file: bad VP8 frame header"));
    }
    let width = u16::from_le_bytes([payload[6], payload[7]]) as u32 & 0x3FFF;
    let height = u16::from_le_bytes([payload[8], payload[9]]) as u32 & 0x3FFF;
    if width == 0 || height == 0 {
        return Err(Error::runtime("corrupt WebP file: bad VP8 dimensions"));
    }
    Ok((width, height))
}

/// Parse canvas dimensions from a lossless `VP8L` bitstream: `0x2F` signature
/// byte, then 14-bit width-1/height-1.
fn lossless_dimensions(payload: &[u8]) -> Result<(u32, u32), Error> {
    if payload.len() < 5 || payload[0] != 0x2F {
        return Err(Error::runtime("corrupt WebP file: bad VP8L header"));
    }
    let bits = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
    Ok((((bits & 0x3FFF) + 1), (((bits >> 14) & 0x3FFF) + 1)))
}

/// Remove the `EXIF`/`ICCP`/`XMP` chunks and clear their `VP8X` feature flags.
/// All other chunks are preserved byte-for-byte, in order.
pub fn wipe(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut chunks = parse(bytes)?;
    chunks.retain(|chunk| !WIPE_CHUNKS.contains(&chunk.fourcc));
    if let Some(vp8x) = chunks.iter_mut().find(|chunk| chunk.fourcc == *b"VP8X") {
        let Some(flags) = vp8x.data.first_mut() else {
            return Err(Error::runtime("corrupt WebP file: empty VP8X chunk"));
        };
        *flags &= !WIPE_FLAGS;
    }
    Ok(encode(&chunks))
}

/// The image-data bytes covered by post-write verification: concatenated
/// `VP8`/`VP8L`/`ALPH`/`ANMF` payloads.
pub fn image_data(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for chunk in parse(bytes)? {
        if IMAGE_CHUNKS.contains(&chunk.fourcc) {
            out.extend_from_slice(&chunk.data);
        }
    }
    Ok(out)
}
