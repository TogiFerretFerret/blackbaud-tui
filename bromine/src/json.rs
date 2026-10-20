//! Lenient readers for the LMS's JSON: fields come and go, numbers sometimes arrive as strings, and
//! "no value" is the int `-2147483648`.

use serde_json::Value;

pub(crate) const SENTINEL: i64 = -2_147_483_648;

pub(crate) fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

pub(crate) fn int(v: &Value, key: &str) -> Option<i64> {
    let n = match v.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n != SENTINEL).then_some(n)
}

pub(crate) fn float(v: &Value, key: &str) -> Option<f64> {
    let n = match v.get(key)? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n != SENTINEL as f64).then_some(n)
}

/// `true`, or a non-zero number (the API mixes the two).
pub(crate) fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().is_some_and(|i| i != 0 && i != SENTINEL),
        _ => false,
    }
}

pub(crate) fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}
