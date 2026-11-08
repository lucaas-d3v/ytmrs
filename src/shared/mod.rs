pub mod player_state;
pub mod store_schema;
pub mod types;

use serde::Serializer;

/// Serializes an `f64` the way `JSON.stringify` would for a JS number:
/// integral values as integers, non finite values as `null`.
pub fn ser_num<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if !v.is_finite() {
        s.serialize_none()
    } else if v.fract() == 0.0 && v.abs() < 9.0e15 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

pub fn ser_opt_num<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => ser_num(v, s),
        None => s.serialize_none(),
    }
}

/// Mirrors JS `Number(value)` semantics loosely for values coming from YTM.
pub fn js_number(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
        serde_json::Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        serde_json::Value::Null => 0.0,
        _ => f64::NAN,
    }
}

/// JS `parseInt` for strings like "215".
pub fn js_parse_int(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().map(|f| f.trunc()).unwrap_or(f64::NAN),
        serde_json::Value::String(s) => {
            let s = s.trim();
            let mut end = 0;
            for (i, c) in s.char_indices() {
                if c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')) {
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            s[..end].parse::<f64>().unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    }
}
