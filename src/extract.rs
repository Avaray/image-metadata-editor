use crate::model::Output;
use nom_exif::{MediaParser, MediaSource};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

pub fn extract(path: &str, parser: &mut MediaParser) -> Result<Output, Box<dyn std::error::Error>> {
    // Resolve to an absolute path so `File:` always shows the full location,
    // regardless of how the user typed the argument.  `std::path::absolute`
    // (stable since 1.79) does NOT prepend the `\\?\` UNC prefix on Windows
    // and does NOT require the path to already exist.
    let absolute =
        std::path::absolute(Path::new(path)).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let file_str = absolute.display().to_string();

    let mut out = Output {
        file: file_str,
        directories: BTreeMap::new(),
    };

    let ms = MediaSource::<File>::open(path)?;

    match ms.kind() {
        nom_exif::MediaKind::Image => {
            if let Ok(meta) = parser.parse_image_metadata(ms) {
                if let Some(exif) = meta.exif {
                    for entry in exif {
                        if let Some(val) = entry.value() {
                            let dir_name = format!("{:?}", entry.ifd_kind());
                            let tag_name = entry.tag().to_string();
                            out.directories
                                .entry(dir_name)
                                .or_default()
                                .insert(tag_name, val.to_string());
                        }
                    }
                }

                if let Some(nom_exif::ImageFormatMetadata::Png(chunks)) = meta.format {
                    for (k, v) in chunks.iter() {
                        out.directories
                            .entry("PngText".to_string())
                            .or_default()
                            .insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        nom_exif::MediaKind::Track => {
            if let Ok(track) = parser.parse_track(ms) {
                for (tag, value) in track.iter() {
                    out.directories
                        .entry("Track".to_string())
                        .or_default()
                        .insert(tag.to_string(), value.to_string());
                }
            }
        }
    }

    Ok(out)
}
