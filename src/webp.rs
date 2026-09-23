use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

pub fn strip_metadata(inp: &mut File, out: &mut BufWriter<File>) -> Result<(), String> {
    // Check if next 4 bytes are WEBP
    let mut webp_sig = [0u8; 4];
    inp.read_exact(&mut webp_sig).map_err(|e| e.to_string())?;
    if &webp_sig != b"WEBP" {
        return Err("Not a valid WebP file".into());
    }

    out.write_all(b"RIFF").map_err(|e| e.to_string())?;
    // Write dummy size, we'll update it later
    out.write_all(&[0, 0, 0, 0]).map_err(|e| e.to_string())?;
    out.write_all(b"WEBP").map_err(|e| e.to_string())?;

    let mut total_written = 4u32; // for "WEBP"

    loop {
        let mut chunk_header = [0u8; 8];
        match inp.read_exact(&mut chunk_header) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        }

        let tag = &chunk_header[0..4];
        let size = u32::from_le_bytes([chunk_header[4], chunk_header[5], chunk_header[6], chunk_header[7]]);
        let padded_size = if size % 2 != 0 { size + 1 } else { size };

        if tag == b"EXIF" || tag == b"XMP " {
            // Strip it
            inp.seek(SeekFrom::Current(padded_size as i64)).map_err(|e| e.to_string())?;
        } else if tag == b"VP8X" {
            // Read VP8X, clear bit 3 (EXIF) and bit 2 (XMP)
            out.write_all(&chunk_header).map_err(|e| e.to_string())?;
            let mut vp8x_data = vec![0u8; padded_size as usize];
            inp.read_exact(&mut vp8x_data).map_err(|e| e.to_string())?;

            // VP8X flags are in the first byte
            // bit 3 (0x08) = EXIF, bit 2 (0x04) = XMP
            vp8x_data[0] &= !0x08;
            vp8x_data[0] &= !0x04;

            out.write_all(&vp8x_data).map_err(|e| e.to_string())?;
            total_written += 8 + padded_size;
        } else {
            // Keep the chunk
            out.write_all(&chunk_header).map_err(|e| e.to_string())?;
            let mut remaining = padded_size;
            let mut buf = [0u8; 8192];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining as usize, buf.len());
                inp.read_exact(&mut buf[..to_read]).map_err(|e| e.to_string())?;
                out.write_all(&buf[..to_read]).map_err(|e| e.to_string())?;
                remaining -= to_read as u32;
            }
            total_written += 8 + padded_size;
        }
    }

    // Update RIFF size
    out.flush().map_err(|e| e.to_string())?;
    let out_file = out.get_mut();
    out_file.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    out_file.write_all(&total_written.to_le_bytes()).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn inject_metadata(inp: &mut File, out: &mut BufWriter<File>, exif_chunk: &[u8]) -> Result<(), String> {
    let mut webp_sig = [0u8; 4];
    inp.read_exact(&mut webp_sig).map_err(|e| e.to_string())?;
    if &webp_sig != b"WEBP" {
        return Err("Not a valid WebP file".into());
    }

    out.write_all(b"RIFF").map_err(|e| e.to_string())?;
    out.write_all(&[0, 0, 0, 0]).map_err(|e| e.to_string())?;
    out.write_all(b"WEBP").map_err(|e| e.to_string())?;

    let mut total_written = 4u32;
    let mut has_vp8x = false;

    loop {
        let mut chunk_header = [0u8; 8];
        match inp.read_exact(&mut chunk_header) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        }

        let tag = &chunk_header[0..4];
        let size = u32::from_le_bytes([chunk_header[4], chunk_header[5], chunk_header[6], chunk_header[7]]);
        let padded_size = if size % 2 != 0 { size + 1 } else { size };

        if tag == b"EXIF" {
            // Skip existing EXIF — we will append a fresh one at the end
            inp.seek(SeekFrom::Current(padded_size as i64)).map_err(|e| e.to_string())?;
        } else if tag == b"VP8X" {
            has_vp8x = true;
            out.write_all(&chunk_header).map_err(|e| e.to_string())?;
            let mut vp8x_data = vec![0u8; padded_size as usize];
            inp.read_exact(&mut vp8x_data).map_err(|e| e.to_string())?;

            // Set EXIF flag (bit 3)
            vp8x_data[0] |= 0x08;

            out.write_all(&vp8x_data).map_err(|e| e.to_string())?;
            total_written += 8 + padded_size;
        } else if !has_vp8x && (tag == b"VP8 " || tag == b"VP8L") {
            // Simple WebP (no VP8X chunk yet). We read the image data first so we can
            // extract canvas dimensions, then prepend a VP8X chunk before writing the
            // image chunk. This converts the file to the Extended WebP format in-place.
            let mut img_data = vec![0u8; padded_size as usize];
            inp.read_exact(&mut img_data).map_err(|e| e.to_string())?;

            // Parse canvas dimensions from the bitstream:
            //   VP8 : 3-byte frame tag at [0..3], then 3-byte start code at [3..6],
            //         then width  (bits 0–13 of LE u16 at [6..8]),
            //              height (bits 0–13 of LE u16 at [8..10]).
            //   VP8L: signature byte 0x2F at [0], then 28 packed bits:
            //         bits  0–13 = width  - 1
            //         bits 14–27 = height - 1
            let (canvas_w, canvas_h) = if tag == b"VP8 " && img_data.len() >= 10 {
                let w = (u16::from_le_bytes([img_data[6], img_data[7]]) & 0x3FFF) as u32;
                let h = (u16::from_le_bytes([img_data[8], img_data[9]]) & 0x3FFF) as u32;
                (w, h)
            } else if tag == b"VP8L" && img_data.len() >= 5 {
                let bits = u32::from_le_bytes([img_data[1], img_data[2], img_data[3], img_data[4]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                (w, h)
            } else {
                (0u32, 0u32)
            };

            // Build VP8X payload: 4-byte flags + 3-byte width-1 + 3-byte height-1 = 10 bytes
            let w_minus1 = canvas_w.saturating_sub(1);
            let h_minus1 = canvas_h.saturating_sub(1);
            let mut vp8x_payload = [0u8; 10];
            vp8x_payload[0] = 0x08; // EXIF flag (bit 3)
            vp8x_payload[4] = (w_minus1 & 0xFF) as u8;
            vp8x_payload[5] = ((w_minus1 >> 8) & 0xFF) as u8;
            vp8x_payload[6] = ((w_minus1 >> 16) & 0xFF) as u8;
            vp8x_payload[7] = (h_minus1 & 0xFF) as u8;
            vp8x_payload[8] = ((h_minus1 >> 8) & 0xFF) as u8;
            vp8x_payload[9] = ((h_minus1 >> 16) & 0xFF) as u8;

            // Write VP8X chunk (8-byte header + 10-byte payload)
            out.write_all(b"VP8X").map_err(|e| e.to_string())?;
            out.write_all(&10u32.to_le_bytes()).map_err(|e| e.to_string())?;
            out.write_all(&vp8x_payload).map_err(|e| e.to_string())?;
            total_written += 18;
            has_vp8x = true;

            // Write the original image chunk unchanged
            out.write_all(&chunk_header).map_err(|e| e.to_string())?;
            out.write_all(&img_data).map_err(|e| e.to_string())?;
            total_written += 8 + padded_size;
        } else {
            // Keep chunk unchanged
            out.write_all(&chunk_header).map_err(|e| e.to_string())?;
            let mut remaining = padded_size;
            let mut buf = [0u8; 8192];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining as usize, buf.len());
                inp.read_exact(&mut buf[..to_read]).map_err(|e| e.to_string())?;
                out.write_all(&buf[..to_read]).map_err(|e| e.to_string())?;
                remaining -= to_read as u32;
            }
            total_written += 8 + padded_size;
        }
    }

    // Append the EXIF chunk at the end
    out.write_all(exif_chunk).map_err(|e| e.to_string())?;
    total_written += exif_chunk.len() as u32;
    if exif_chunk.len() % 2 != 0 {
        out.write_all(&[0]).map_err(|e| e.to_string())?;
        total_written += 1;
    }

    // Patch the RIFF size field
    out.flush().map_err(|e| e.to_string())?;
    let out_file = out.get_mut();
    out_file.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    out_file.write_all(&total_written.to_le_bytes()).map_err(|e| e.to_string())?;

    Ok(())
}
