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

/// One decoded PNG text chunk: its keyword exactly as stored, plus its text.
pub struct TextChunk {
    pub keyword: String,
    pub text: String,
}

/// Enumerate every `tEXt`/`zTXt`/`iTXt` chunk except the raw-profile EXIF
/// carriers (those surface under `exif`, never under `custom`). Undecodable
/// chunks are skipped, not errors: one bad comment must not hide the rest.
pub fn text_chunks(chunks: &[Chunk]) -> Vec<TextChunk> {
    let mut out = Vec::new();
    for chunk in chunks {
        let decoded = match &chunk.kind {
            b"tEXt" => decode_texte(&chunk.data),
            b"zTXt" => decode_ztxt(&chunk.data),
            b"iTXt" => decode_itxt(&chunk.data),
            _ => None,
        };
        if let Some((keyword, text)) = decoded
            && keyword != RAW_PROFILE_EXIF
            && keyword != RAW_PROFILE_APP1
        {
            out.push(TextChunk { keyword, text });
        }
    }
    out
}

/// Split a text chunk into its keyword bytes and the remainder. Keywords are
/// 1-79 bytes per the PNG spec; anything else is malformed.
fn split_keyword(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let nul = data.iter().position(|&b| b == 0)?;
    if nul == 0 || nul > 79 {
        return None;
    }
    Some((&data[..nul], &data[nul + 1..]))
}

/// Decode a Latin-1 byte string (`tEXt`/`zTXt` keywords and text) with a
/// byte-to-char mapping, not lossy UTF-8.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Decode a keyword: UTF-8 when valid (what `iTXt` writers produce in
/// practice), otherwise Latin-1.
fn decode_keyword(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).map(str::to_string).unwrap_or_else(|_| latin1(bytes))
}

fn decode_texte(data: &[u8]) -> Option<(String, String)> {
    let (keyword, text) = split_keyword(data)?;
    Some((decode_keyword(keyword), latin1(text)))
}

fn decode_ztxt(data: &[u8]) -> Option<(String, String)> {
    let (keyword, rest) = split_keyword(data)?;
    if rest.first() != Some(&0) {
        return None;
    }
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&rest[1..]).ok()?;
    Some((decode_keyword(keyword), latin1(&inflated)))
}

fn decode_itxt(data: &[u8]) -> Option<(String, String)> {
    let (keyword, rest) = split_keyword(data)?;
    if rest.len() < 2 || rest[0] > 1 || rest[1] != 0 {
        return None;
    }
    let mut parts = rest[2..].splitn(3, |&b| b == 0);
    parts.next()?;
    parts.next()?;
    let text = parts.next()?;
    let raw = if rest[0] == 1 { miniz_oxide::inflate::decompress_to_vec_zlib(text).ok()? } else { text.to_vec() };
    Some((decode_keyword(keyword), String::from_utf8_lossy(&raw).into_owned()))
}

/// Rewrite the text-chunk set: drop every `tEXt`/`zTXt`/`iTXt` chunk except
/// the raw-profile EXIF carriers, then store each desired entry as `iTXt`
/// (UTF-8, uncompressed), sorted by keyword for deterministic output.
pub fn set_text_chunks(bytes: &[u8], desired: &[(String, String)]) -> Result<Vec<u8>, Error> {
    for (keyword, _) in desired {
        validate_keyword(keyword)?;
    }
    let mut chunks = parse(bytes)?;
    chunks.retain(|chunk| !is_custom_text(chunk));
    let mut sorted: Vec<&(String, String)> = desired.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let position = chunks.iter().position(|chunk| chunk.kind == *b"IEND").unwrap_or(chunks.len());
    for (index, (keyword, text)) in sorted.iter().enumerate() {
        chunks.insert(position + index, Chunk { kind: *b"iTXt", data: itxt_data(keyword, text) });
    }
    Ok(encode(&chunks))
}

/// A text chunk managed as custom storage: any `tEXt`/`zTXt`/`iTXt` chunk
/// that is not a raw-profile EXIF carrier (malformed ones included — with no
/// keyword they cannot be represented, so a rewrite drops them).
fn is_custom_text(chunk: &Chunk) -> bool {
    match &chunk.kind {
        b"tEXt" | b"zTXt" | b"iTXt" => match split_keyword(&chunk.data) {
            Some((keyword, _)) => {
                let keyword = decode_keyword(keyword);
                keyword != RAW_PROFILE_EXIF && keyword != RAW_PROFILE_APP1
            }
            None => true,
        },
        _ => false,
    }
}

fn itxt_data(keyword: &str, text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(keyword.len() + 5 + text.len());
    data.extend_from_slice(keyword.as_bytes());
    data.push(0);
    data.push(0);
    data.push(0);
    data.push(0);
    data.push(0);
    data.extend_from_slice(text.as_bytes());
    data
}

fn validate_keyword(keyword: &str) -> Result<(), Error> {
    if keyword.is_empty() || keyword.len() > 79 {
        return Err(Error::runtime(format!("invalid PNG text keyword '{keyword}': must be 1-79 bytes")));
    }
    if keyword.as_bytes().contains(&0) {
        return Err(Error::runtime("invalid PNG text keyword: must not contain NUL bytes"));
    }
    if keyword == RAW_PROFILE_EXIF || keyword == RAW_PROFILE_APP1 {
        return Err(Error::runtime(format!("PNG text keyword '{keyword}' is reserved for EXIF data")));
    }
    Ok(())
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
