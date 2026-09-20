use crate::model::Output;
use nom_exif::{MediaParser, MediaSource};
use std::collections::BTreeMap;
use std::fs::File;

pub fn extract<'a>(
    path: &'a str,
    parser: &mut MediaParser,
) -> Result<Output<'a>, Box<dyn std::error::Error>> {
    let mut out = Output {
        file: path,
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
