use crc::{CRC_32_ISO_HDLC, Crc};
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};

const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

pub fn inject_metadata(path: &str, out_path: Option<&str>, tags: &BTreeMap<String, String>) -> Result<(), String> {
    let out_path_str = match out_path {
        Some(p) => p.to_string(),
        None => format!("{}.tmp", path),
    };

    // Copy original to out_path_str so we can modify it
    std::fs::copy(path, &out_path_str).map_err(|e| format!("Failed to copy file: {}", e))?;

    // Identify format by sniffing magic bytes
    let mut sig = [0u8; 8];
    {
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        let n = f.read(&mut sig).map_err(|e| e.to_string())?;
        if n < 8 {
            return Err("File too small".into());
        }
    }

    let is_jpeg = sig[0..2] == [0xFF, 0xD8];
    let is_png = sig == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let is_webp = sig[0..4] == *b"RIFF";

    if !is_jpeg && !is_png && !is_webp {
        std::fs::remove_file(&out_path_str).ok();
        return Err("Injecting metadata is currently unsupported for this format (only JPEG, PNG, and WebP are supported).".into());
    }

    // Split tags into known EXIF and unknown
    let mut unknown = BTreeMap::new();
    let mut known_exif_tags: Vec<ExifTag> = Vec::new();

    for (k, v) in tags {
        match parse_known_exif_tag(k, v) {
            Some(tag) => known_exif_tags.push(tag),
            None => {
                unknown.insert(k.clone(), v.clone());
            }
        }
    }

    // For JPEG/WebP: pack unknown tags into UserComment as JSON
    if (is_jpeg || is_webp)
        && !unknown.is_empty()
        && let Ok(json) = serde_json::to_string(&unknown)
    {
        known_exif_tags.push(ExifTag::UserComment(json.into_bytes()));
    }

    // Write EXIF
    if !known_exif_tags.is_empty() {
        if is_webp {
            // For WebP, little_exif gives us the full RIFF chunk bytes which we can inject
            let mut exif = Metadata::new();
            for tag in known_exif_tags {
                exif.set_tag(tag);
            }
            let chunk_bytes = exif.as_u8_vec(little_exif::filetype::FileExtension::WEBP).map_err(|e| format!("Failed to build WebP EXIF: {:?}", e))?;

            let mut inp = File::open(path).map_err(|e| e.to_string())?;
            // We read 8 bytes of sig, need to reset to offset 8 for WebP RIFF parser
            // Actually our webp parser expects 4 bytes already read ("RIFF"), so offset 4
            inp.seek(std::io::SeekFrom::Start(4)).map_err(|e| e.to_string())?;

            let mut out = BufWriter::new(File::create(&out_path_str).map_err(|e| e.to_string())?);

            crate::webp::inject_metadata(&mut inp, &mut out, &chunk_bytes)?;

            // Drop handles explicitly
            drop(out);
            drop(inp);
        } else {
            // For JPEG/PNG we use little_exif's built-in file writing
            let out_p = std::path::Path::new(&out_path_str);
            let mut exif = Metadata::new_from_path(out_p).unwrap_or_else(|_| Metadata::new());

            for tag in known_exif_tags {
                exif.set_tag(tag);
            }

            exif.write_to_file(out_p).map_err(|e| {
                std::fs::remove_file(&out_path_str).ok();
                format!("Failed to write EXIF: {:?}", e)
            })?;
        }
    }

    // If PNG and we have unknown tags, inject tEXt chunks
    if is_png && !unknown.is_empty() {
        let temp_png = format!("{}.png.tmp", out_path_str);
        match inject_png_text(&out_path_str, &temp_png, &unknown) {
            Ok(_) => {
                std::fs::rename(&temp_png, &out_path_str).map_err(|e| format!("Failed to swap png temp file: {}", e))?;
            }
            Err(e) => {
                std::fs::remove_file(&temp_png).ok();
                std::fs::remove_file(&out_path_str).ok();
                return Err(e);
            }
        }
    }

    // Finalize: if in-place, rename temp over original
    if out_path.is_none() {
        std::fs::rename(&out_path_str, path).map_err(|e| format!("Failed to overwrite original file: {}", e))?;
    }

    Ok(())
}

fn parse_known_exif_tag(key: &str, value: &str) -> Option<ExifTag> {
    let v = value.to_string();
    match key {
        "ImageDescription" => Some(ExifTag::ImageDescription(v)),
        "Make" => Some(ExifTag::Make(v)),
        "Model" => Some(ExifTag::Model(v)),
        "Software" => Some(ExifTag::Software(v)),
        "Artist" => Some(ExifTag::Artist(v)),
        "Copyright" => Some(ExifTag::Copyright(v)),
        "DateTimeOriginal" => Some(ExifTag::DateTimeOriginal(v)),
        "UserComment" => Some(ExifTag::UserComment(v.into_bytes())),
        _ => None,
    }
}

fn inject_png_text(in_path: &str, out_path: &str, tags: &BTreeMap<String, String>) -> Result<(), String> {
    let mut inp = File::open(in_path).map_err(|e| e.to_string())?;
    let mut out = BufWriter::new(File::create(out_path).map_err(|e| e.to_string())?);

    // Copy PNG signature
    let mut sig = [0u8; 8];
    inp.read_exact(&mut sig).map_err(|e| e.to_string())?;
    out.write_all(&sig).map_err(|e| e.to_string())?;

    // Keys that have already been written (either as new after IHDR or as replacement)
    let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut new_injected = false;

    loop {
        let mut len_buf = [0u8; 4];
        if inp.read_exact(&mut len_buf).is_err() {
            break;
        }
        let length = u32::from_be_bytes(len_buf) as usize;

        let mut type_buf = [0u8; 4];
        inp.read_exact(&mut type_buf).map_err(|e| e.to_string())?;

        // After IHDR: inject any new tags that don't exist in the file yet
        if !new_injected && &type_buf != b"IHDR" {
            for (k, v) in tags {
                if !written.contains(k) {
                    let k_trunc = if k.len() > 79 { &k[..79] } else { k };
                    write_text_chunk(&mut out, k_trunc, v).map_err(|e| e.to_string())?;
                    written.insert(k.clone());
                }
            }
            new_injected = true;
        }

        // Read chunk data + CRC into a buffer so we can inspect / skip
        let mut chunk_data = vec![0u8; length];
        inp.read_exact(&mut chunk_data).map_err(|e| e.to_string())?;
        let mut crc_buf = [0u8; 4];
        inp.read_exact(&mut crc_buf).map_err(|e| e.to_string())?;

        if &type_buf == b"tEXt"
            && let Some(nul_pos) = chunk_data.iter().position(|&b| b == 0)
            && let Ok(chunk_key) = std::str::from_utf8(&chunk_data[..nul_pos])
            && let Some(new_value) = tags.get(chunk_key)
        {
            // Replace this chunk with the new value
            let k_trunc = if chunk_key.len() > 79 { &chunk_key[..79] } else { chunk_key };
            write_text_chunk(&mut out, k_trunc, new_value).map_err(|e| e.to_string())?;
            written.insert(chunk_key.to_string());
            if &type_buf == b"IEND" {
                break;
            }
            continue;
        }

        // Pass through unchanged chunk
        out.write_all(&len_buf).map_err(|e| e.to_string())?;
        out.write_all(&type_buf).map_err(|e| e.to_string())?;
        out.write_all(&chunk_data).map_err(|e| e.to_string())?;
        out.write_all(&crc_buf).map_err(|e| e.to_string())?;

        if &type_buf == b"IEND" {
            break;
        }
    }

    Ok(())
}

fn write_text_chunk(out: &mut impl Write, key: &str, value: &str) -> std::io::Result<()> {
    // tEXt data: key bytes + 0x00 separator + value bytes
    let mut data = Vec::new();
    data.extend_from_slice(key.as_bytes());
    data.push(0);
    data.extend_from_slice(value.as_bytes());

    let length = data.len() as u32;
    out.write_all(&length.to_be_bytes())?;

    // CRC covers chunk type + chunk data
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(b"tEXt");
    crc_input.extend_from_slice(&data);

    out.write_all(b"tEXt")?;
    out.write_all(&data)?;
    out.write_all(&CRC32.checksum(&crc_input).to_be_bytes())?;

    Ok(())
}
