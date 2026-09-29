use serde_json::{Map, Value};

use crate::error::Error;

/// One parsed `--set` payload: the two optional top-level sections
/// (`None` = that section is left untouched).
pub struct SetPayload {
    pub exif: Option<SetSection>,
    pub custom: Option<SetSection>,
}

/// A section value: `Clear` (`null` or any other empty value) deletes
/// everything in the section; `Merge` deep-merges the given object.
pub enum SetSection {
    Clear,
    Merge(Map<String, Value>),
}

/// Parse one `--set` argument as lenient JSON5, validating the
/// `{"exif": ..., "custom": ...}` shape.
pub fn parse_set_payload(text: &str) -> Result<SetPayload, Error> {
    let value: Value = json5::from_str(text).map_err(|err| Error::runtime(format!("invalid --set JSON: {err}")))?;
    let Value::Object(root) = value else {
        return Err(Error::runtime("invalid --set JSON: expected an object with 'exif' and/or 'custom' sections"));
    };
    let mut exif = None;
    let mut custom = None;
    for (key, section_value) in root {
        let slot = match key.as_str() {
            "exif" => &mut exif,
            "custom" => &mut custom,
            _ => return Err(Error::runtime(format!("invalid --set JSON: unknown section '{key}' (expected 'exif' or 'custom')"))),
        };
        *slot = Some(match section_value {
            Value::Null => SetSection::Clear,
            Value::Object(map) if map.is_empty() => SetSection::Clear,
            Value::Object(map) => SetSection::Merge(map),
            Value::String(text) if text.is_empty() => SetSection::Clear,
            Value::Array(items) if items.is_empty() => SetSection::Clear,
            _ => return Err(Error::runtime(format!("invalid --set JSON: section '{key}' must be an object or null"))),
        });
    }
    Ok(SetPayload { exif, custom })
}

/// Deep (recursive) merge of `input` into `base`, in place: only keys
/// explicitly present in the input change anything. An empty input value
/// (`null`, `""`, `{}`, `[]`) deletes the key instead; objects merge
/// recursively when both sides are objects, everything else replaces.
pub fn deep_merge(base: &mut Map<String, Value>, input: &Map<String, Value>) {
    for (key, value) in input {
        if is_empty_value(value) {
            base.remove(key);
            continue;
        }
        if let Value::Object(patch) = value
            && let Some(Value::Object(existing)) = base.get_mut(key)
        {
            deep_merge(existing, patch);
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// Serialize a JSON value deterministically: compact JSON with object keys
/// sorted alphabetically at every nesting level.
pub fn serialize_canonical(value: &Value) -> Result<String, Error> {
    serde_json::to_string(&sorted_value(value)).map_err(|err| Error::runtime(format!("cannot encode custom metadata as JSON: {err}")))
}

/// Escape every non-ASCII character as `\uXXXX` (with surrogate pairs past
/// the BMP), for carriers that must stay ASCII-safe (JPEG/WebP UserComment).
pub fn ascii_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let code = ch as u32;
            if code <= 0xFFFF {
                out.push_str(&format!("\\u{code:04x}"));
            } else {
                let v = code - 0x1_0000;
                out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xD800 + (v >> 10), 0xDC00 + (v & 0x3FF)));
            }
        }
    }
    out
}

fn sorted_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            keys.into_iter().map(|key| (key.clone(), sorted_value(&map[key]))).collect()
        }
        Value::Array(items) => items.iter().map(sorted_value).collect(),
        _ => value.clone(),
    }
}
