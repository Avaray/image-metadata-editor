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
    let pending_exif = Some(exif_chunk);

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
            // Skip existing EXIF
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
        } else {
            // Keep chunk
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

    if !has_vp8x {
        return Err("Injecting EXIF into simple (non-extended) WebP files is currently unsupported. File must contain a VP8X chunk.".into());
    }

    // Append EXIF chunk
    if let Some(exif_data) = pending_exif {
        out.write_all(exif_data).map_err(|e| e.to_string())?;
        total_written += exif_data.len() as u32;

        // Pad if needed
        if exif_data.len() % 2 != 0 {
            out.write_all(&[0]).map_err(|e| e.to_string())?;
            total_written += 1;
        }
    }

    out.flush().map_err(|e| e.to_string())?;
    let out_file = out.get_mut();
    out_file.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    out_file.write_all(&total_written.to_le_bytes()).map_err(|e| e.to_string())?;

    Ok(())
}
