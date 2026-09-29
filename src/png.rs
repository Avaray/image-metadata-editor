use crate::error::Error;

/// PNG file signature; also the magic bytes used for format detection.
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Chunks kept by `--wipe`: exactly the critical chunks from `03-business-logic.md`.
const WIPE_KEEP: [[u8; 4]; 4] = [*b"IHDR", *b"PLTE", *b"IDAT", *b"IEND"];

/// Keywords of the legacy ImageMagick raw-profile text chunks carrying EXIF.
const RAW_PROFILE_EXIF: &str = "Raw profile type exif";
const RAW_PROFILE_APP1: &str = "Raw profile type APP1";

/// One parsed PNG chunk (type + data; length and CRC are recomputed on encode).
pub struct Chunk {
    pub kind: [u8; 4],
    pub data: Vec<u8>,
}

/// Parse the chunk stream, validating its structure. Stops at `IEND`; chunk
/// CRCs are not validated. Bytes after `IEND` are not part of the datastream
/// and are ignored.
pub fn parse(bytes: &[u8]) -> Result<Vec<Chunk>, Error> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(Error::runtime("corrupt PNG file: bad signature"));
    }
    let mut chunks = Vec::new();
    let mut pos = PNG_SIGNATURE.len();
    while pos < bytes.len() {
        let header = bytes.get(pos..pos + 8).ok_or_else(|| Error::runtime("corrupt PNG file: truncated chunk header"))?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&header[4..8]);
        let data_start = pos + 8;
        let data_end = data_start.checked_add(length).ok_or_else(|| Error::runtime("corrupt PNG file: chunk length overflows"))?;
        let chunk_end = data_end.checked_add(4).ok_or_else(|| Error::runtime("corrupt PNG file: chunk length overflows"))?;
        if chunk_end > bytes.len() {
            return Err(Error::runtime("corrupt PNG file: truncated chunk data"));
        }
        chunks.push(Chunk { kind, data: bytes[data_start..data_end].to_vec() });
        if kind == *b"IEND" {
            break;
        }
        pos = chunk_end;
    }
    Ok(chunks)
}

/// Serialize chunks back into a file, recomputing lengths and CRC32 checksums.
pub fn encode(chunks: &[Chunk]) -> Vec<u8> {
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut out = Vec::from(PNG_SIGNATURE);
    for chunk in chunks {
        out.extend_from_slice(&(chunk.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&chunk.kind);
        out.extend_from_slice(&chunk.data);
        let mut digest = crc.digest();
        digest.update(&chunk.kind);
        digest.update(&chunk.data);
        out.extend_from_slice(&digest.finalize().to_be_bytes());
    }
    out
}

/// Payload of the first `tEXt` chunk with the given Latin-1 keyword, if any.
pub fn find_text<'a>(chunks: &'a [Chunk], keyword: &str) -> Option<&'a [u8]> {
    chunks.iter().filter(|chunk| chunk.kind == *b"tEXt").filter_map(|chunk| text_payload(&chunk.data, keyword)).next()
}

fn text_payload<'a>(data: &'a [u8], keyword: &str) -> Option<&'a [u8]> {
    let nul = data.iter().position(|&b| b == 0)?;
    if data[..nul] == *keyword.as_bytes() { Some(&data[nul + 1..]) } else { None }
}

/// Insert or replace a `tEXt` chunk, placed right before `IEND`. All other
/// chunks are preserved byte-for-byte.
pub fn upsert_text(bytes: &[u8], keyword: &str, text: &[u8]) -> Result<Vec<u8>, Error> {
    let mut chunks = parse(bytes)?;
    chunks.retain(|chunk| !(chunk.kind == *b"tEXt" && text_payload(&chunk.data, keyword).is_some()));
    let mut data = Vec::with_capacity(keyword.len() + 1 + text.len());
    data.extend_from_slice(keyword.as_bytes());
    data.push(0);
    data.extend_from_slice(text);
    let position = chunks.iter().position(|chunk| chunk.kind == *b"IEND").unwrap_or(chunks.len());
    chunks.insert(position, Chunk { kind: *b"tEXt", data });
    Ok(encode(&chunks))
}

/// Remove every `tEXt` chunk with the given keyword. All other chunks are
/// preserved byte-for-byte.
pub fn remove_text(bytes: &[u8], keyword: &str) -> Result<Vec<u8>, Error> {
    let mut chunks = parse(bytes)?;
    chunks.retain(|chunk| !(chunk.kind == *b"tEXt" && text_payload(&chunk.data, keyword).is_some()));
    Ok(encode(&chunks))
}

/// Extract the raw EXIF (TIFF) payload: the `eXIf` chunk, or a legacy
/// ImageMagick raw-profile `tEXt`/`zTXt` chunk. Priority mirrors `nom-exif`:
/// `eXIf` wins over `APP1` profiles, which win over `exif` profiles.
/// Unparseable sources are skipped, not errors.
pub fn exif_payload(chunks: &[Chunk]) -> Option<Vec<u8>> {
    let mut fallback: Option<(u8, Vec<u8>)> = None;
    for chunk in chunks {
        match &chunk.kind {
            b"eXIf" => {
                if is_tiff(&chunk.data) {
                    return Some(chunk.data.clone());
                }
            }
            b"tEXt" => {
                if let Some((priority, payload)) = raw_profile_payload(&chunk.data, false) {
                    take_candidate(&mut fallback, priority, payload);
                }
            }
            b"zTXt" => {
                if let Some((priority, payload)) = raw_profile_payload(&chunk.data, true) {
                    take_candidate(&mut fallback, priority, payload);
                }
            }
            _ => {}
        }
    }
    fallback.map(|(_, payload)| payload)
}

fn take_candidate(fallback: &mut Option<(u8, Vec<u8>)>, priority: u8, payload: Vec<u8>) {
    let replace = fallback.as_ref().is_none_or(|(current, _)| priority > *current);
    if replace {
        *fallback = Some((priority, payload));
    }
}

fn raw_profile_payload(data: &[u8], compressed: bool) -> Option<(u8, Vec<u8>)> {
    let nul = data.iter().position(|&b| b == 0)?;
    let keyword = std::str::from_utf8(&data[..nul]).ok()?;
    let priority = match keyword {
        RAW_PROFILE_APP1 => 2,
        RAW_PROFILE_EXIF => 1,
        _ => return None,
    };
    let body = &data[nul + 1..];
    if compressed {
        if body.first() != Some(&0) {
            return None;
        }
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&body[1..]).ok()?;
        return decode_raw_profile(&inflated).map(|payload| (priority, payload));
    }
    decode_raw_profile(body).map(|payload| (priority, payload))
}

/// Decode an ImageMagick raw-profile value: three header lines
/// (`\n`, type, decimal length), then whitespace-tolerant hex.
fn decode_raw_profile(data: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.split('\n');
    lines.next()?;
    lines.next()?;
    lines.next()?;
    let mut payload = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for byte in lines.collect::<String>().bytes() {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b' ' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        match high.take() {
            None => high = Some(nibble),
            Some(h) => payload.push((h << 4) | nibble),
        }
    }
    if high.is_some() {
        return None;
    }
    // Writers disagree on whether the hex includes the APP1 `Exif\0\0`
    // header (`little_exif` does, ImageMagick does not): strip it when present.
    if let Some(stripped) = payload.strip_prefix(b"Exif\0\0") {
        payload = stripped.to_vec();
    }
    if is_tiff(&payload) { Some(payload) } else { None }
}

fn is_tiff(payload: &[u8]) -> bool {
    payload.len() >= 8 && (payload.starts_with(b"II*\0") || payload.starts_with(b"MM\0*"))
}

/// Remove every chunk but `IHDR`/`PLTE`/`IDAT`/`IEND`, preserving the order of
/// what remains.
pub fn wipe(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let chunks = parse(bytes)?;
    if chunks.first().is_none_or(|chunk| chunk.kind != *b"IHDR") {
        return Err(Error::runtime("corrupt PNG file: missing IHDR"));
    }
    if chunks.iter().all(|chunk| chunk.kind != *b"IEND") {
        return Err(Error::runtime("corrupt PNG file: missing IEND"));
    }
    let kept: Vec<Chunk> = chunks.into_iter().filter(|chunk| WIPE_KEEP.contains(&chunk.kind)).collect();
    Ok(encode(&kept))
}

/// The image-data bytes covered by post-write verification: concatenated
/// `IDAT` payloads.
pub fn image_data(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for chunk in parse(bytes)? {
        if chunk.kind == *b"IDAT" {
            out.extend_from_slice(&chunk.data);
        }
    }
    Ok(out)
}
