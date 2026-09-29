use little_exif::rational::{iR64, uR64};
use serde_json::Value;

use crate::error::Error;

/// Strict validators converting `--set` JSON values into EXIF tag payloads.
/// Every tag accepts exactly the JSON shape a read produces for it
/// (`06-data-schemas.md`); anything else is a runtime error. There is no
/// silent type coercion.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_error(name: &str, expected: &str, value: &Value) -> Error {
    Error::runtime(format!("exif tag '{name}': expected {expected}, got {}", kind_of(value)))
}

/// Normalize a scalar-or-array JSON value into a list of references, enforcing
/// the component count from the tag table when it is fixed.
fn as_list<'a>(name: &str, what: &str, value: &'a Value, count: Option<u32>) -> Result<Vec<&'a Value>, Error> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        single => vec![single],
    };
    if items.is_empty() {
        return Err(Error::runtime(format!("exif tag '{name}': expected {what}, got an empty array")));
    }
    if let Some(expected) = count
        && items.len() as u32 != expected
    {
        return Err(Error::runtime(format!("exif tag '{name}': expected {expected} component(s), got {}", items.len())));
    }
    Ok(items)
}

fn as_u64_item(name: &str, value: &Value) -> Result<u64, Error> {
    match value {
        Value::Number(number) => number.as_u64().ok_or_else(|| type_error(name, "an unsigned integer", value)),
        _ => Err(type_error(name, "an unsigned integer", value)),
    }
}

macro_rules! int_list {
    ($name:ident, $target:ty, $extract:ident, $what:literal) => {
        pub fn $name(tag: &str, value: &Value, count: Option<u32>) -> Result<Vec<$target>, Error> {
            let mut out = Vec::new();
            for item in as_list(tag, $what, value, count)? {
                let raw = $extract(tag, item)?;
                out.push(<$target>::try_from(raw).map_err(|_| Error::runtime(format!("exif tag '{tag}': value {raw} is out of range for {}", $what)))?);
            }
            Ok(out)
        }
    };
}

int_list!(expect_u8_list, u8, as_u64_item, "an unsigned 8-bit integer");
int_list!(expect_u16_list, u16, as_u64_item, "an unsigned 16-bit integer");
int_list!(expect_u32_list, u32, as_u64_item, "an unsigned 32-bit integer");

pub fn expect_text(name: &str, value: &Value, count: Option<u32>) -> Result<String, Error> {
    let text = match value {
        Value::String(text) if text.is_ascii() => text,
        Value::String(_) => return Err(Error::runtime(format!("exif tag '{name}': expected an ASCII string"))),
        _ => return Err(type_error(name, "a string", value)),
    };
    if let Some(expected) = count
        && text.len() as u32 + 1 != expected
    {
        return Err(Error::runtime(format!("exif tag '{name}': expected a string of {} character(s), got {}", expected - 1, text.len())));
    }
    Ok(text.clone())
}

fn parse_rational(name: &str, value: &Value, signed: bool) -> Result<(i64, i64), Error> {
    let text = match value {
        Value::String(text) => text,
        _ => return Err(type_error(name, "a \"numerator/denominator\" string", value)),
    };
    let Some((num, den)) = text.split_once('/') else {
        return Err(Error::runtime(format!("exif tag '{name}': expected a \"numerator/denominator\" string, got {text:?}")));
    };
    let parse_part = |part: &str| -> Option<i64> {
        if part.is_empty() {
            return None;
        }
        let (digits, negative) = match part.strip_prefix('-') {
            Some(rest) if signed => (rest, true),
            Some(_) => return None,
            None => (part, false),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let magnitude: i64 = digits.parse().ok()?;
        Some(if negative { -magnitude } else { magnitude })
    };
    let (Some(n), Some(d)) = (parse_part(num), parse_part(den)) else {
        return Err(Error::runtime(format!("exif tag '{name}': expected a \"numerator/denominator\" string, got {text:?}")));
    };
    if d == 0 {
        return Err(Error::runtime(format!("exif tag '{name}': rational denominator must not be zero")));
    }
    Ok((n, d))
}

pub fn expect_urational_list(name: &str, value: &Value, count: Option<u32>) -> Result<Vec<uR64>, Error> {
    let mut out = Vec::new();
    for item in as_list(name, "a \"numerator/denominator\" string", value, count)? {
        let (n, d) = parse_rational(name, item, false)?;
        out.push(uR64 { nominator: n as u32, denominator: d as u32 });
    }
    Ok(out)
}

pub fn expect_irational_list(name: &str, value: &Value, count: Option<u32>) -> Result<Vec<iR64>, Error> {
    let mut out = Vec::new();
    for item in as_list(name, "a \"numerator/denominator\" string", value, count)? {
        let (n, d) = parse_rational(name, item, true)?;
        let nominator = i32::try_from(n).map_err(|_| Error::runtime(format!("exif tag '{name}': rational numerator {n} is out of range")))?;
        let denominator = i32::try_from(d).map_err(|_| Error::runtime(format!("exif tag '{name}': rational denominator {d} is out of range")))?;
        out.push(iR64 { nominator, denominator });
    }
    Ok(out)
}

pub fn expect_undef(name: &str, value: &Value, count: Option<u32>) -> Result<Vec<u8>, Error> {
    let text = match value {
        Value::String(text) => text,
        _ => return Err(type_error(name, "a lower-case hex string", value)),
    };
    let bytes = hex::decode(text).map_err(|_| Error::runtime(format!("exif tag '{name}': expected a lower-case hex string, got {text:?}")))?;
    if let Some(expected) = count
        && bytes.len() as u32 != expected
    {
        return Err(Error::runtime(format!("exif tag '{name}': expected {expected} byte(s), got {}", bytes.len())));
    }
    Ok(bytes)
}
