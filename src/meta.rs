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

/// Structural pointer tags, hidden from reads: sub-IFD offsets plus the
/// strip/thumbnail layout tags. Their values are encoder-managed byte offsets
/// that shift on every rewrite (positional noise, not image metadata), and
/// the encoder re-adds them whenever their IFD exists — so a cleared section
/// could never read back empty while they stay visible.
const HIDDEN_OFFSETS: [(u16, ExifTagGroup); 3] = [(0x8769, ExifTagGroup::GENERIC), (0x8825, ExifTagGroup::GENERIC), (0xA005, ExifTagGroup::EXIF)];
const HIDDEN_LAYOUT: [u16; 4] = [0x0111, 0x0117, 0x0201, 0x0202];

/// Metadata read from a single image file: standard EXIF tags plus custom keys.
/// `codes` maps every exposed tag name back to its `(code, group)` identity
/// for `--set` deletes; `has_exif` records whether an EXIF segment was found
/// at all (even one with no recognized tags).
pub struct Metadata {
    pub exif: Map<String, Value>,
    pub custom: Option<Map<String, Value>>,
    pub codes: std::collections::HashMap<String, (u16, ExifTagGroup)>,
    pub has_exif: bool,
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
    let mut codes = std::collections::HashMap::new();
    let mut user_comment: Option<Vec<u8>> = None;

    // PNG EXIF is extracted at the chunk level first (`eXIf` or an
    // ImageMagick raw-profile `tEXt`/`zTXt` chunk — the latter is where
    // `little_exif` writes, and `nom-exif` cannot see it), then parsed as
    // TIFF. Other formats go to `nom-exif` directly.
    let chunks = if format == ImageFormat::Png { Some(png::parse(bytes)?) } else { None };
    let tiff_payload = chunks.as_deref().and_then(png::exif_payload);
    let parsed = match (&chunks, tiff_payload) {
        (Some(_), Some(tiff)) => parse_exif(&tiff)?,
        (Some(_), None) => None,
        (None, _) => parse_exif(bytes)?,
    };
    let has_exif = parsed.is_some();

    if let Some(parsed) = parsed {
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
            if HIDDEN_OFFSETS.contains(&(code, group)) || HIDDEN_LAYOUT.contains(&code) {
                continue;
            }
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
            exif.insert(name.clone(), format_value(entry.value()));
            codes.insert(name, (code, group));
        }
    }

    let custom = match format {
        ImageFormat::Png => read_png_custom(chunks.as_deref().unwrap_or(&[])),
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

    Ok(Metadata { exif, custom, codes, has_exif })
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
fn read_png_custom(chunks: &[png::Chunk]) -> Option<Map<String, Value>> {
    let payload = png::find_text(chunks, PNG_CUSTOM_KEYWORD)?;
    let hex_text = std::str::from_utf8(payload).unwrap_or("");
    hex::decode(hex_text.trim()).ok().and_then(|json_bytes| decode_custom_json(&json_bytes))
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
