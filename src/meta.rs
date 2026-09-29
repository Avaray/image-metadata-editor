use little_exif::exif_tag::ExifTag;
use little_exif::ifd::ExifTagGroup;
use nom_exif::{EntryValue, Exif, IfdIndex, IfdKind, MediaParser, MediaSource};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::format::ImageFormat;
use crate::png;

/// Keyword of the PNG `tEXt` chunk carrying the `custom` section (see `06-data-schemas.md`).
const PNG_CUSTOM_KEYWORD: &str = "ime:custom";

/// EXIF tag code of `UserComment`, the on-disk carrier of the `custom`
/// section on JPEG/WebP. It never appears under `exif` in output.
const USER_COMMENT_CODE: u16 = 0x9286;

/// Character-code prefix of a `UserComment` payload holding JSON (`06-data-schemas.md`).
const USER_COMMENT_ASCII_PREFIX: &[u8] = b"ASCII\0\0\0";

/// Metadata read from a single image file: standard EXIF tags plus custom keys.
pub struct Metadata {
    pub exif: Map<String, Value>,
    pub custom: Option<Map<String, Value>>,
}

impl Metadata {
    /// Render the `{"exif": ..., "custom": ...}` shape from `06-data-schemas.md`,
    /// omitting sections that are empty.
    pub fn to_value(&self) -> Value {
        let mut root = Map::new();
        if !self.exif.is_empty() {
            root.insert("exif".to_string(), Value::Object(self.exif.clone()));
        }
        if let Some(custom) = &self.custom
            && !custom.is_empty()
        {
            root.insert("custom".to_string(), Value::Object(custom.clone()));
        }
        Value::Object(root)
    }
}

/// Read all metadata from one image. EXIF comes from `nom-exif`; the `custom`
/// section comes from the format-specific carrier (PNG `tEXt` chunk, or the
/// EXIF `UserComment` tag on JPEG/WebP).
pub fn read_metadata(bytes: &[u8], format: ImageFormat) -> Result<Metadata, Error> {
    let mut exif = Map::new();
    let mut user_comment: Option<Vec<u8>> = None;

    if let Some(parsed) = parse_exif(bytes)? {
        for entry in parsed.entries() {
            if entry.ifd() == IfdIndex::THUMBNAIL {
                continue;
            }
            let group = match entry.ifd_kind() {
                IfdKind::Tiff => ExifTagGroup::GENERIC,
                IfdKind::Exif => ExifTagGroup::EXIF,
                IfdKind::Gps => ExifTagGroup::GPS,
                IfdKind::Interop => ExifTagGroup::INTEROP,
                _ => continue,
            };
            let code = entry.tag().code();
            if code == USER_COMMENT_CODE && group == ExifTagGroup::EXIF {
                if format != ImageFormat::Png
                    && let EntryValue::Undefined(payload) = entry.value()
                {
                    user_comment = Some(payload.clone());
                }
                continue;
            }
            let Ok(tag) = ExifTag::from_u16(code, &group) else {
                continue;
            };
            let name = canonical_name(&tag);
            if exif.contains_key(&name) {
                continue;
            }
            exif.insert(name, format_value(entry.value()));
        }
    }

    let custom = match format {
        ImageFormat::Png => read_png_custom(bytes)?,
        ImageFormat::Jpeg | ImageFormat::Webp => user_comment.as_deref().and_then(decode_user_comment),
    };

    for value in exif.values_mut() {
        traverse(value);
    }
    let mut custom = custom;
    if let Some(map) = custom.as_mut() {
        for value in map.values_mut() {
            traverse(value);
        }
    }

    Ok(Metadata { exif, custom })
}

/// The canonical variant name of a `little_exif` tag (e.g. `Make`, `GPSLatitude`).
fn canonical_name(tag: &ExifTag) -> String {
    let debug = format!("{tag:?}");
    match debug.split('(').next() {
        Some(name) => name.to_string(),
        None => debug,
    }
}

/// Parse the EXIF segment of already-magic-checked image bytes. A file with no
/// EXIF data yields `Ok(None)`; a file whose container cannot be parsed is a
/// runtime error.
fn parse_exif(bytes: &[u8]) -> Result<Option<Exif>, Error> {
    let source = MediaSource::from_memory(bytes.to_vec()).map_err(|err| Error::runtime(format!("cannot read metadata: {err}")))?;
    let mut parser = MediaParser::new();
    match parser.parse_exif(source) {
        Ok(iter) => Ok(Some(Exif::from(iter))),
        Err(nom_exif::Error::ExifNotFound) => Ok(None),
        Err(err) => Err(Error::runtime(format!("cannot read metadata: {err}"))),
    }
}

/// Format one EXIF value as JSON, following the value table in `06-data-schemas.md`:
/// strings as-is, integers as numbers, rationals as `"numerator/denominator"`,
/// dates in native `"YYYY:MM:DD HH:MM:SS"` form, rational arrays (e.g. GPS
/// triplets) as arrays of `"numerator/denominator"` strings, and binary
/// (`UNDEFINED`) values as lower-case hex. Multi-component integer arrays map
/// to JSON arrays of numbers; floats map to JSON numbers.
fn format_value(value: &EntryValue) -> Value {
    match value {
        EntryValue::Text(text) => Value::String(text.clone()),
        EntryValue::U8(n) => Value::from(*n),
        EntryValue::U16(n) => Value::from(*n),
        EntryValue::U32(n) => Value::from(*n),
        EntryValue::U64(n) => Value::from(*n),
        EntryValue::I8(n) => Value::from(*n),
        EntryValue::I16(n) => Value::from(*n),
        EntryValue::I32(n) => Value::from(*n),
        EntryValue::I64(n) => Value::from(*n),
        EntryValue::F32(n) => number_or_string(f64::from(*n)),
        EntryValue::F64(n) => number_or_string(*n),
        EntryValue::URational(r) => Value::String(format!("{}/{}", r.numerator(), r.denominator())),
        EntryValue::IRational(r) => Value::String(format!("{}/{}", r.numerator(), r.denominator())),
        EntryValue::URationalArray(items) => items.iter().map(|r| Value::String(format!("{}/{}", r.numerator(), r.denominator()))).collect(),
        EntryValue::IRationalArray(items) => items.iter().map(|r| Value::String(format!("{}/{}", r.numerator(), r.denominator()))).collect(),
        EntryValue::U8Array(items) => items.iter().map(|n| Value::from(*n)).collect(),
        EntryValue::U16Array(items) => items.iter().map(|n| Value::from(*n)).collect(),
        EntryValue::U32Array(items) => items.iter().map(|n| Value::from(*n)).collect(),
        EntryValue::Undefined(bytes) => Value::String(hex::encode(bytes)),
        EntryValue::DateTime(dt) => Value::String(dt.format("%Y:%m:%d %H:%M:%S").to_string()),
        EntryValue::NaiveDateTime(dt) => Value::String(dt.format("%Y:%m:%d %H:%M:%S").to_string()),
        _ => Value::String(format!("{value:?}")),
    }
}

fn number_or_string(n: f64) -> Value {
    match serde_json::Number::from_f64(n) {
        Some(number) => Value::Number(number),
        None => Value::String(n.to_string()),
    }
}

/// Read the `custom` section from the PNG `tEXt` chunk: hex-encoded compact JSON.
fn read_png_custom(bytes: &[u8]) -> Result<Option<Map<String, Value>>, Error> {
    let Some(payload) = png::find_text_chunk(bytes, PNG_CUSTOM_KEYWORD)? else {
        return Ok(None);
    };
    let hex_text = std::str::from_utf8(&payload).unwrap_or("");
    match hex::decode(hex_text.trim()) {
        Ok(json_bytes) => Ok(decode_custom_json(&json_bytes)),
        Err(_) => Ok(None),
    }
}

/// Read the `custom` section from a JPEG/WebP `UserComment` payload: compact
/// JSON behind the standard 8-byte `"ASCII\0\0\0"` prefix.
fn decode_user_comment(payload: &[u8]) -> Option<Map<String, Value>> {
    decode_custom_json(payload.strip_prefix(USER_COMMENT_ASCII_PREFIX)?)
}

fn decode_custom_json(json_bytes: &[u8]) -> Option<Map<String, Value>> {
    let text = std::str::from_utf8(json_bytes).ok()?;
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Smart JSON traversal (`03-business-logic.md`, output-only): any string
/// value holding a JSON object or array is embedded as nested JSON, applied
/// recursively. Strings holding JSON scalars stay strings.
fn traverse(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text)
                && (parsed.is_object() || parsed.is_array())
            {
                *value = parsed;
                traverse(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                traverse(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                traverse(item);
            }
        }
        _ => {}
    }
}
