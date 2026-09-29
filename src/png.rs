use crate::error::Error;

/// PNG file signature; also the magic bytes used for format detection.
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Find the payload of the first `tEXt` chunk with the given Latin-1 keyword.
/// Returns `Ok(None)` when the file has no such chunk. Chunk CRCs are not
/// validated on read; a structurally broken chunk stream (length overruns,
/// truncation) is reported as a corrupt file.
pub fn find_text_chunk(bytes: &[u8], keyword: &str) -> Result<Option<Vec<u8>>, Error> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(Error::runtime("corrupt PNG file: bad signature"));
    }
    let mut pos = PNG_SIGNATURE.len();
    while pos < bytes.len() {
        let header = bytes.get(pos..pos + 8).ok_or_else(|| Error::runtime("corrupt PNG file: truncated chunk header"))?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let chunk_type = &header[4..8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(length).ok_or_else(|| Error::runtime("corrupt PNG file: chunk length overflows"))?;
        let chunk_end = data_end.checked_add(4).ok_or_else(|| Error::runtime("corrupt PNG file: chunk length overflows"))?;
        if chunk_end > bytes.len() {
            return Err(Error::runtime("corrupt PNG file: truncated chunk data"));
        }
        if chunk_type == b"tEXt" {
            let data = &bytes[data_start..data_end];
            if let Some(nul) = data.iter().position(|&b| b == 0) {
                let (key, rest) = (&data[..nul], &data[nul + 1..]);
                if key == keyword.as_bytes() {
                    return Ok(Some(rest.to_vec()));
                }
            }
        }
        if chunk_type == b"IEND" {
            break;
        }
        pos = chunk_end;
    }
    Ok(None)
}
