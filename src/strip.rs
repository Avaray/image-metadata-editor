use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};

pub fn strip_metadata(path: &str, out_path: Option<&str>) -> Result<(), String> {
    // If no output path specified, we overwrite by writing to a temp file first, then moving.
    let out_path_str = match out_path {
        Some(p) => p.to_string(),
        None => format!("{}.tmp", path),
    };

    let mut inp = File::open(path).map_err(|e| e.to_string())?;

    // Read signature to identify format
    let mut sig = [0u8; 8];
    let n = inp.read(&mut sig).map_err(|e| e.to_string())?;
    if n < 8 {
        return Err("File too small".into());
    }

    let mut out = BufWriter::new(File::create(&out_path_str).map_err(|e| e.to_string())?);

    let res = if sig[0..2] == [0xFF, 0xD8] {
        strip_jpeg(&mut inp, &mut out, &sig[0..2])
    } else if sig == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        strip_png(&mut inp, &mut out, &sig)
    } else if sig[0..4] == *b"RIFF" {
        crate::webp::strip_metadata(&mut inp, &mut out)
    } else {
        Err("Stripping metadata is currently unsupported for this format (only JPEG, PNG, and WebP are supported).".into())
    };

    // Drop file handles explicitly
    drop(out);
    drop(inp);

    // If an error occurred, remove the partial temp file
    if res.is_err() && out_path.is_none() {
        let _ = std::fs::remove_file(&out_path_str);
    }

    res?;

    // If inplace (no output path), rename temp file to original
    if out_path.is_none() {
        std::fs::rename(&out_path_str, path).map_err(|e| format!("Failed to overwrite original file: {}", e))?;
    }

    Ok(())
}

fn strip_jpeg(inp: &mut File, out: &mut BufWriter<File>, sig: &[u8]) -> Result<(), String> {
    out.write_all(sig).map_err(|e| e.to_string())?;
    // We already read 0xFF 0xD8 (first 2 bytes), but wait, the signature read was 8 bytes!
    // So the pointer is at offset 8. We need to reset to offset 2.
    inp.seek(SeekFrom::Start(2)).map_err(|e| e.to_string())?;

    loop {
        // Find next 0xFF
        let mut byte = [0u8; 1];
        if inp.read(&mut byte).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if byte[0] != 0xFF {
            out.write_all(&byte).map_err(|e| e.to_string())?;
            continue;
        }

        let mut marker = [0u8; 1];
        if inp.read(&mut marker).map_err(|e| e.to_string())? == 0 {
            out.write_all(&[0xFF]).map_err(|e| e.to_string())?;
            break;
        }

        if marker[0] == 0xFF {
            out.write_all(&[0xFF]).map_err(|e| e.to_string())?;
            inp.seek(SeekFrom::Current(-1)).map_err(|e| e.to_string())?;
            continue;
        }

        if marker[0] == 0x00 || marker[0] == 0x01 || (marker[0] >= 0xD0 && marker[0] <= 0xD9) {
            out.write_all(&[0xFF, marker[0]]).map_err(|e| e.to_string())?;
            if marker[0] == 0xD9 {
                break; // EOI
            }
            continue;
        }

        let mut len_buf = [0u8; 2];
        if inp.read_exact(&mut len_buf).is_err() {
            out.write_all(&[0xFF, marker[0]]).map_err(|e| e.to_string())?;
            break;
        }
        let length = u16::from_be_bytes(len_buf) as usize;

        // Strip COM (0xFE) and APP1-APP15 (0xE1-0xEF), keeping APP2 (0xE2) and APP14 (0xEE)
        let strip = marker[0] == 0xFE || (marker[0] >= 0xE1 && marker[0] <= 0xEF && marker[0] != 0xE2 && marker[0] != 0xEE);

        if strip {
            if length >= 2 {
                inp.seek(SeekFrom::Current((length - 2) as i64)).map_err(|e| e.to_string())?;
            }
        } else {
            out.write_all(&[0xFF, marker[0], len_buf[0], len_buf[1]]).map_err(|e| e.to_string())?;
            if length >= 2 {
                let mut data = vec![0; length - 2];
                inp.read_exact(&mut data).map_err(|e| e.to_string())?;
                out.write_all(&data).map_err(|e| e.to_string())?;
            }
        }

        if marker[0] == 0xDA {
            // SOS - Start of Scan
            io::copy(inp, out).map_err(|e| e.to_string())?;
            break;
        }
    }

    Ok(())
}

fn strip_png(inp: &mut File, out: &mut BufWriter<File>, sig: &[u8]) -> Result<(), String> {
    out.write_all(sig).map_err(|e| e.to_string())?;
    // We read 8 bytes, which is the entire PNG signature. We are correctly at offset 8.

    loop {
        let mut len_buf = [0u8; 4];
        if inp.read_exact(&mut len_buf).is_err() {
            break;
        }
        let length = u32::from_be_bytes(len_buf) as usize;

        let mut type_buf = [0u8; 4];
        inp.read_exact(&mut type_buf).map_err(|e| e.to_string())?;

        let strip = matches!(&type_buf, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME");

        if strip {
            // Skip data (length) + CRC (4)
            inp.seek(SeekFrom::Current((length + 4) as i64)).map_err(|e| e.to_string())?;
        } else {
            out.write_all(&len_buf).map_err(|e| e.to_string())?;
            out.write_all(&type_buf).map_err(|e| e.to_string())?;

            // Read data and CRC without allocating massive memory chunks
            let mut remaining = length + 4;
            let mut buf = [0u8; 8192];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, buf.len());
                inp.read_exact(&mut buf[..to_read]).map_err(|e| e.to_string())?;
                out.write_all(&buf[..to_read]).map_err(|e| e.to_string())?;
                remaining -= to_read;
            }
        }

        if &type_buf == b"IEND" {
            break;
        }
    }

    Ok(())
}
