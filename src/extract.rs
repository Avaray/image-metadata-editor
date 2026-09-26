use crate::model::Output;
use nom_exif::{MediaParser, MediaSource};
use std::fs::File;

pub fn extract(path: &str, parser: &mut MediaParser) -> Result<Output, Box<dyn std::error::Error>> {
    let mut out = Output::new();

    let ms = MediaSource::<File>::open(path)?;

    match ms.kind() {
        nom_exif::MediaKind::Image => {
            if let Ok(meta) = parser.parse_image_metadata(ms) {
                if let Some(exif) = meta.exif {
                    for entry in exif {
                        if let Some(val) = entry.value() {
                            let dir_name = format!("{:?}", entry.ifd_kind());
                            let mut tag_name = entry.tag().to_string();
                            if tag_name.starts_with("Unknown(0x") {
                                if tag_name == "Unknown(0x010e)" {
                                    tag_name = "ImageDescription".to_string();
                                } else if tag_name == "Unknown(0x010f)" {
                                    tag_name = "Make".to_string();
                                } else if tag_name == "Unknown(0x0110)" {
                                    tag_name = "Model".to_string();
                                } else if tag_name == "Unknown(0x0131)" {
                                    tag_name = "Software".to_string();
                                } else if tag_name == "Unknown(0x013b)" {
                                    tag_name = "Artist".to_string();
                                } else if tag_name == "Unknown(0x8298)" {
                                    tag_name = "Copyright".to_string();
                                } else if tag_name == "Unknown(0x9003)" {
                                    tag_name = "DateTimeOriginal".to_string();
                                } else if tag_name == "Unknown(0x9286)" {
                                    tag_name = "UserComment".to_string();
                                }
                            }

                            let val_str = val.to_string();
                            let decoded = if val_str.starts_with("0x") { decode_exif_hex_string(&val_str) } else { val_str };

                            out.entry(dir_name).or_default().insert(tag_name, decoded);
                        }
                    }
                }

                if let Some(nom_exif::ImageFormatMetadata::Png(chunks)) = meta.format {
                    for (k, v) in chunks.iter() {
                        out.entry("PngText".to_string()).or_default().insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        nom_exif::MediaKind::Track => {
            if let Ok(track) = parser.parse_track(ms) {
                for (tag, value) in track.iter() {
                    out.entry("Track".to_string()).or_default().insert(tag.to_string(), value.to_string());
                }
            }
        }
    }

    Ok(out)
}

fn decode_exif_hex_string(val: &str) -> String {
    let hex_str = &val[2..];
    if let Ok(bytes) = hex::decode(hex_str) {
        if bytes.len() >= 8 {
            let prefix = &bytes[0..8];

            if prefix == b"UNICODE\0" {
                let utf16_bytes = &bytes[8..];
                let mut be = true;
                if utf16_bytes.len() >= 2 {
                    if utf16_bytes[0] == 0xFF && utf16_bytes[1] == 0xFE {
                        be = false;
                    } else if utf16_bytes[0] == 0xFE && utf16_bytes[1] == 0xFF {
                        be = true;
                    } else if utf16_bytes[0] != 0 && utf16_bytes[1] == 0 {
                        be = false;
                    }
                }

                let mut u16s = Vec::with_capacity(utf16_bytes.len() / 2);
                for chunk in utf16_bytes.chunks_exact(2) {
                    if be {
                        u16s.push(u16::from_be_bytes([chunk[0], chunk[1]]));
                    } else {
                        u16s.push(u16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                let decoded = String::from_utf16_lossy(&u16s);
                return decoded.trim_start_matches('\u{feff}').to_string();
            } else if prefix == b"ASCII\0\0\0" {
                return String::from_utf8_lossy(&bytes[8..]).trim_end_matches('\0').to_string();
            }
        }

        // Fallback: try raw UTF-8 in case it's just raw bytes
        if let Ok(s) = String::from_utf8(bytes.clone()) {
            if s.chars().all(|c| !c.is_control() || c.is_ascii_whitespace()) {
                return s;
            }
        }
    }

    val.to_string()
}
