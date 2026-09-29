use crate::error::Error;

/// Markers without a length field: TEM, RST0-RST7, SOI, EOI.
fn is_standalone(code: u8) -> bool {
    code == 0x01 || (0xD0..=0xD9).contains(&code)
}

fn check_signature(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8 { Ok(()) } else { Err(Error::runtime("corrupt JPEG file: bad signature")) }
}

/// Read one marker starting at `pos` (which must point at `0xFF`, possibly
/// preceded by fill bytes). Returns `(code, marker_start, header_end)` where
/// `header_end` is past the marker for standalone markers and past the length
/// field otherwise.
fn read_marker(bytes: &[u8], mut pos: usize) -> Result<(u8, usize, usize), Error> {
    if bytes.get(pos) != Some(&0xFF) {
        return Err(Error::runtime("corrupt JPEG file: expected marker"));
    }
    while bytes.get(pos) == Some(&0xFF) {
        pos += 1;
    }
    let code = *bytes.get(pos).ok_or_else(|| Error::runtime("corrupt JPEG file: truncated marker"))?;
    if code == 0x00 {
        return Err(Error::runtime("corrupt JPEG file: unexpected stuffed byte outside scan data"));
    }
    let marker_start = pos - 1;
    let header_end = pos + 1;
    if is_standalone(code) {
        return Ok((code, marker_start, header_end));
    }
    let len_bytes = bytes.get(header_end..header_end + 2).ok_or_else(|| Error::runtime("corrupt JPEG file: truncated segment header"))?;
    let length = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
    if length < 2 {
        return Err(Error::runtime("corrupt JPEG file: invalid segment length"));
    }
    let segment_end = header_end.checked_add(length).ok_or_else(|| Error::runtime("corrupt JPEG file: segment length overflows"))?;
    if segment_end > bytes.len() {
        return Err(Error::runtime("corrupt JPEG file: truncated segment"));
    }
    Ok((code, marker_start, segment_end))
}

/// Find `(scan_start, eoi_start)`: the SOS marker and the EOI marker ending
/// the scan data. Byte-stuffed `FF 00` pairs inside the scan are data, not markers.
pub fn scan_range(bytes: &[u8]) -> Result<(usize, usize), Error> {
    check_signature(bytes)?;
    let mut pos = 2;
    let sos = loop {
        let (code, marker_start, header_end) = read_marker(bytes, pos)?;
        if code == 0xDA {
            break marker_start;
        }
        if code == 0xD9 {
            return Err(Error::runtime("corrupt JPEG file: EOI before SOS"));
        }
        pos = header_end;
    };
    // Skip the SOS segment header to reach raw scan data.
    let (_, _, scan_start) = read_marker(bytes, sos)?;
    let mut i = scan_start;
    while i + 1 < bytes.len() {
        if bytes[i] == 0xFF {
            match bytes[i + 1] {
                0x00 => i += 2,
                0xD9 => return Ok((sos, i)),
                _ => i += 1,
            }
        } else {
            i += 1;
        }
    }
    Err(Error::runtime("corrupt JPEG file: missing EOI"))
}

/// Remove every `APPn`/`COM` segment plus any bytes after `EOI`. All other
/// segments and the scan data are preserved byte-for-byte.
pub fn wipe(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    check_signature(bytes)?;
    let mut out = vec![0xFF, 0xD8];
    let mut pos = 2;
    loop {
        let (code, marker_start, header_end) = read_marker(bytes, pos)?;
        if code == 0xDA {
            let (sos, eoi) = scan_range(bytes)?;
            debug_assert_eq!(sos, marker_start);
            out.extend_from_slice(&bytes[sos..eoi + 2]);
            return Ok(out);
        }
        if code == 0xD9 {
            out.extend_from_slice(&bytes[marker_start..header_end]);
            return Ok(out);
        }
        let is_metadata = (0xE0..=0xEF).contains(&code) || code == 0xFE;
        if !is_metadata {
            out.extend_from_slice(&bytes[marker_start..header_end]);
        }
        pos = header_end;
    }
}

/// The image-data bytes covered by post-write verification: everything from
/// the first SOS marker up to (not including) EOI.
pub fn image_data(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let (sos, eoi) = scan_range(bytes)?;
    Ok(bytes[sos..eoi].to_vec())
}
