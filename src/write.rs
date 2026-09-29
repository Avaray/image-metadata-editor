use std::path::Path;

use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::exif_tags;
use crate::format::ImageFormat;
use crate::jpeg;
use crate::merge;
use crate::meta;
use crate::png;
use crate::webp;

/// Tag code of `UserComment`, the on-disk carrier of `custom` on JPEG/WebP.
const USER_COMMENT_CODE: u16 = 0x9286;

/// Character-code prefix of a `UserComment` payload holding JSON.
const USER_COMMENT_ASCII_PREFIX: &[u8] = b"ASCII\0\0\0";

/// The result of a `--set`/`--wipe` computation, before it is written
/// anywhere. `changed == false` means the result is byte-identical to the
/// source, so an in-place write can be skipped.
pub struct WriteOutcome {
    pub bytes: Vec<u8>,
    pub changed: bool,
}

/// Compute the `--set` result: deep-merge the pre-parsed payloads in order
/// onto the file's current metadata, and encode the difference.
pub fn apply_set(source: &[u8], format: ImageFormat, payloads: &[merge::SetPayload]) -> Result<WriteOutcome, Error> {
    let current = meta::read_metadata(source, format)?;
    let mut merged_exif = current.exif.clone();
    let mut merged_custom = current.custom.clone();
    for payload in payloads {
        match &payload.exif {
            None => {}
            Some(merge::SetSection::Clear) => merged_exif.clear(),
            Some(merge::SetSection::Merge(map)) => merge::deep_merge(&mut merged_exif, map),
        }
        match &payload.custom {
            None => {}
            Some(merge::SetSection::Clear) => merged_custom = Some(Map::new()),
            Some(merge::SetSection::Merge(map)) => merge::deep_merge(merged_custom.get_or_insert_with(Map::new), map),
        }
    }

    // Minimal diff: only added/changed keys are (re)written, so untouched tags
    // pass through the encoder exactly as they were.
    let mut deletes: Vec<(u16, ExifTagGroup)> = Vec::new();
    for key in current.exif.keys() {
        if !merged_exif.contains_key(key) {
            let Some((code, group)) = current.codes.get(key) else { continue };
            deletes.push((*code, *group));
        }
    }
    let mut sets: Vec<ExifTag> = Vec::new();
    for (key, value) in &merged_exif {
        if current.exif.get(key) == Some(value) {
            continue;
        }
        sets.push(build_tag(key, value)?);
    }
    let custom_changed = merged_custom != current.custom;

    if sets.is_empty() && deletes.is_empty() && !custom_changed {
        return Ok(WriteOutcome { bytes: source.to_vec(), changed: false });
    }

    let mut result = source.to_vec();
    match format {
        ImageFormat::Png => {
            if !sets.is_empty() || !deletes.is_empty() {
                run_little_exif(&mut result, format, sets, deletes, current.has_exif)?;
            }
            if custom_changed {
                result = apply_png_custom(&result, merged_custom.as_ref())?;
            }
        }
        ImageFormat::Jpeg | ImageFormat::Webp => {
            if custom_changed {
                apply_jpeg_custom(&mut sets, &mut deletes, merged_custom.as_ref())?;
            }
            run_little_exif(&mut result, format, sets, deletes, current.has_exif)?;
        }
    }
    Ok(WriteOutcome { bytes: result, changed: true })
}

/// Compute the `--wipe` result: remove every standard EXIF tag, every custom
/// key, the ICC profile, and any other ancillary chunk or segment.
pub fn apply_wipe(source: &[u8], format: ImageFormat) -> Result<WriteOutcome, Error> {
    let result = match format {
        ImageFormat::Png => png::wipe(source)?,
        ImageFormat::Jpeg => jpeg::wipe(source)?,
        ImageFormat::Webp => webp::wipe(source)?,
    };
    let changed = result != source;
    Ok(WriteOutcome { bytes: result, changed })
}

fn build_tag(name: &str, value: &Value) -> Result<ExifTag, Error> {
    if name == "UserComment" {
        return Err(Error::runtime("exif tag 'UserComment' is reserved for custom keys; put free-text notes under 'custom'"));
    }
    let Some(spec) = exif_tags::lookup(name) else {
        return Err(Error::runtime(format!("unknown exif tag '{name}' (arbitrary keys belong under 'custom')")));
    };
    if !spec.settable {
        return Err(Error::runtime(format!("exif tag '{name}' cannot be set explicitly")));
    }
    exif_tags::build(name, value)
}

/// Rewrite the PNG text-chunk set from the merged `custom` section (which
/// may only hold `PngText`), or drop every text chunk when it is empty.
fn apply_png_custom(result: &[u8], merged: Option<&Map<String, Value>>) -> Result<Vec<u8>, Error> {
    let mut desired: Vec<(String, String)> = Vec::new();
    if let Some(custom) = merged {
        for key in custom.keys() {
            if key != "PngText" {
                return Err(Error::runtime(format!("unknown custom key '{key}' for PNG files (only 'PngText' is supported)")));
            }
        }
        if let Some(png_text) = custom.get("PngText") {
            let Value::Object(map) = png_text else {
                return Err(Error::runtime("custom 'PngText' must be an object of keyword/value pairs"));
            };
            for (keyword, value) in map {
                desired.push((keyword.clone(), custom_text(value)?));
            }
        }
    }
    png::set_text_chunks(result, &desired)
}

/// The on-disk text of one custom value: strings as-is, anything else as
/// canonical compact JSON.
fn custom_text(value: &Value) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => merge::serialize_canonical(value),
    }
}

/// Translate a custom-section change into a `UserComment` set/delete so it can
/// ride the same `little_exif` pipeline as standard tags. Only `UserComment`
/// may appear under `custom` here; structured values serialize to canonical
/// JSON (ASCII-escaped), plain strings are written as-is.
fn apply_jpeg_custom(sets: &mut Vec<ExifTag>, deletes: &mut Vec<(u16, ExifTagGroup)>, merged: Option<&Map<String, Value>>) -> Result<(), Error> {
    let value = match merged {
        None => None,
        Some(custom) => {
            for key in custom.keys() {
                if key != "UserComment" {
                    return Err(Error::runtime(format!("unknown custom key '{key}' for JPEG/WebP files (only 'UserComment' is supported)")));
                }
            }
            custom.get("UserComment")
        }
    };
    let Some(value) = value else {
        deletes.push((USER_COMMENT_CODE, ExifTagGroup::EXIF));
        return Ok(());
    };
    let text = match value {
        Value::String(text) => {
            if !text.is_ascii() {
                return Err(Error::runtime("custom 'UserComment' string must be ASCII (use \\uXXXX escapes for other characters)"));
            }
            if text.as_bytes().contains(&0) {
                return Err(Error::runtime("custom 'UserComment' string must not contain NUL bytes"));
            }
            text.clone()
        }
        _ => merge::ascii_escape(&merge::serialize_canonical(value)?),
    };
    let mut payload = Vec::from(USER_COMMENT_ASCII_PREFIX);
    payload.extend_from_slice(text.as_bytes());
    sets.push(ExifTag::UserComment(payload));
    Ok(())
}

/// Apply tag sets/deletes through `little_exif`, preserving every tag that is
/// not mentioned. Files without an EXIF segment start from empty metadata;
/// files whose existing EXIF cannot be decoded are refused rather than
/// clobbered.
fn run_little_exif(buffer: &mut Vec<u8>, format: ImageFormat, sets: Vec<ExifTag>, deletes: Vec<(u16, ExifTagGroup)>, has_exif: bool) -> Result<(), Error> {
    let file_type = match format {
        ImageFormat::Png => FileExtension::PNG { as_zTXt_chunk: true },
        ImageFormat::Jpeg => FileExtension::JPEG,
        ImageFormat::Webp => FileExtension::WEBP,
    };
    let mut metadata = match Metadata::new_from_vec(buffer, file_type) {
        Ok(metadata) => metadata,
        Err(_) if !has_exif => Metadata::new(),
        Err(err) => return Err(Error::runtime(format!("cannot decode existing EXIF metadata: {err}"))),
    };
    if format == ImageFormat::Webp {
        // Pre-strip our own way (see `webp::strip_exif`) and promote simple
        // files ourselves, so `little_exif` only ever runs its sound insert
        // path on an extended, EXIF-less buffer.
        *buffer = webp::promote(&webp::strip_exif(buffer)?)?;
    }
    for (code, group) in deletes {
        metadata.remove_tag_by_hex_group(code, group);
    }
    for tag in sets {
        metadata.set_tag(tag);
    }
    metadata.write_to_vec(buffer, file_type).map_err(|err| Error::runtime(format!("cannot encode EXIF metadata: {err}")))
}

/// Post-write verification (`03-business-logic.md`): the image-data bytes of
/// the result must equal the source's. Runs before the result becomes visible
/// anywhere; a mismatch leaves every destination untouched.
pub fn verify(source: &[u8], result: &[u8], format: ImageFormat) -> Result<(), Error> {
    let before = image_data(source, format)?;
    let after = image_data(result, format).map_err(|_| Error::runtime("post-write verification failed: cannot locate image data in the write result"))?;
    if before != after {
        return Err(Error::runtime("post-write verification failed: image data changed"));
    }
    Ok(())
}

fn image_data(bytes: &[u8], format: ImageFormat) -> Result<Vec<u8>, Error> {
    let data = match format {
        ImageFormat::Png => png::image_data(bytes)?,
        ImageFormat::Jpeg => jpeg::image_data(bytes)?,
        ImageFormat::Webp => webp::image_data(bytes)?,
    };
    if data.is_empty() {
        return Err(Error::runtime("post-write verification failed: image data missing"));
    }
    Ok(data)
}

/// Write bytes to `dest` atomically: a temp file in the same directory,
/// then a rename over the destination. The destination is never left
/// partially written.
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = dest.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let pid = std::process::id();
    for attempt in 0..100u32 {
        let temp = parent.join(format!(".ime-{pid}-{attempt}.tmp"));
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(_) => continue,
            // Name taken (or another transient failure): try the next name.
        };
        let outcome = (|| -> Result<(), Error> {
            use std::io::Write as _;
            file.write_all(bytes).map_err(|err| Error::runtime(format!("cannot write to '{}': {err}", dest.display())))?;
            drop(file);
            replace(&temp, dest)?;
            Ok(())
        })();
        if outcome.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return outcome;
    }
    Err(Error::runtime(format!("cannot write to '{}': no free temporary name", dest.display())))
}

/// Move `temp` over `dest`. Unix `rename` replaces atomically; Windows needs
/// the destination removed first for the same silent-overwrite behavior.
fn replace(temp: &Path, dest: &Path) -> Result<(), Error> {
    #[cfg(windows)]
    if dest.exists() {
        std::fs::remove_file(dest).map_err(|err| Error::runtime(format!("cannot write to '{}': {err}", dest.display())))?;
    }
    std::fs::rename(temp, dest).map_err(|err| Error::runtime(format!("cannot write to '{}': {err}", dest.display())))
}

/// Write bytes to stdout in one go, mapping failures (including a closed
/// pipe) to a runtime error instead of panicking.
pub fn write_stdout(bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(bytes).map_err(|err| Error::runtime(format!("cannot write to stdout: {err}")))?;
    handle.flush().map_err(|err| Error::runtime(format!("cannot write to stdout: {err}")))
}
