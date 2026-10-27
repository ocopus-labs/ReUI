//! Compiles `design/tokens.json` (W3C Design Tokens format) into typed Rust constants.
//!
//! Groups become modules and tokens become `SCREAMING_SNAKE` constants, so
//! `color.label.primary` is `tokens::color::label::PRIMARY`. `$type` is inherited from the
//! nearest group that sets it, and `{path.to.token}` aliases are resolved. Shorthands are
//! accepted next to the spec's object forms: plain numbers for dimensions and durations
//! (DIPs and ms), and `"#RRGGBB"` strings for opaque colours.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Map, Value};

fn main() {
    let tokens_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/tokens.json");
    println!("cargo:rerun-if-changed={}", tokens_path.display());
    let text = std::fs::read_to_string(&tokens_path).unwrap_or_else(|e| panic!("can't read {}: {e}", tokens_path.display()));
    let root: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", tokens_path.display()));

    let mut out = String::from("// Generated from design/tokens.json by build.rs. Do not edit.\n");
    emit_group(&root, &root, &mut Vec::new(), None, &mut out, 0);
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("tokens.rs");
    std::fs::write(dest, out).unwrap();
}

fn emit_group(root: &Value, group: &Value, path: &mut Vec<String>, inherited: Option<&str>, out: &mut String, depth: usize) {
    let obj = group.as_object().expect("token groups are objects");
    let group_type = obj.get("$type").and_then(Value::as_str).or(inherited);
    let indent = "    ".repeat(depth);
    for (key, child) in obj {
        if key.starts_with('$') {
            continue;
        }
        path.push(key.clone());
        let child_obj = child.as_object().unwrap_or_else(|| panic!("{} must be an object", path.join(".")));
        if child_obj.contains_key("$value") {
            let ty = child_obj.get("$type").and_then(Value::as_str).or(group_type).unwrap_or_else(|| panic!("{} has no $type", path.join(".")));
            if let Some(desc) = child_obj.get("$description").and_then(Value::as_str) {
                writeln!(out, "{indent}/// {desc}").unwrap();
            }
            let value = resolve(root, &child_obj["$value"], &path.join("."));
            let (rust_ty, expr) = convert(root, ty, &value, &path.join("."));
            writeln!(out, "{indent}pub const {}: {rust_ty} = {expr};", const_name(key)).unwrap();
        } else {
            writeln!(out, "{indent}pub mod {} {{", mod_name(key)).unwrap();
            writeln!(out, "{indent}    #[allow(unused_imports)]\n{indent}    use crate::*;").unwrap();
            emit_group(root, child, path, group_type, out, depth + 1);
            writeln!(out, "{indent}}}").unwrap();
        }
        path.pop();
    }
}

/// Follows `{a.b.c}` aliases (also inside composite values) to concrete values.
fn resolve(root: &Value, value: &Value, at: &str) -> Value {
    match value {
        Value::String(s) if s.starts_with('{') && s.ends_with('}') => {
            let target = lookup(root, &s[1..s.len() - 1]).unwrap_or_else(|| panic!("{at}: unknown alias {s}"));
            resolve(root, &target["$value"], at)
        }
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), resolve(root, v, at))).collect::<Map<_, _>>()),
        other => other.clone(),
    }
}

fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(root, |node, key| node.get(key))
}

fn convert(root: &Value, ty: &str, value: &Value, at: &str) -> (&'static str, String) {
    match ty {
        "color" => ("Color", color(value, at)),
        "dimension" => ("f32", float(measure(value, "px", at))),
        "number" => ("f32", float(value.as_f64().unwrap_or_else(|| panic!("{at}: expected a number")))),
        "duration" => ("u32", format!("{}", measure(value, "ms", at) as u32)),
        "fontFamily" => ("&str", format!("{:?}", value.as_str().unwrap_or_else(|| panic!("{at}: expected a string")))),
        "fontWeight" => ("u16", format!("{}", value.as_u64().unwrap_or_else(|| panic!("{at}: expected an integer")))),
        "typography" => {
            let field = |k: &str| value.get(k).unwrap_or_else(|| panic!("{at}: typography needs {k}"));
            let family = field("fontFamily").as_str().unwrap_or_else(|| panic!("{at}: fontFamily must be a string"));
            let weight = field("fontWeight").as_u64().unwrap_or_else(|| panic!("{at}: fontWeight must be an integer"));
            let size = measure(field("fontSize"), "px", at);
            let line = value.get("lineHeight").map(|v| measure(v, "px", at)).unwrap_or(size * 1.25);
            ("TextStyle", format!("TextStyle {{ family: {family:?}, weight: {weight}, size: {}, line_height: {} }}", float(size), float(line)))
        }
        "spring" => {
            let damping = value.get("dampingRatio").and_then(Value::as_f64).unwrap_or_else(|| panic!("{at}: spring needs dampingRatio"));
            let period = measure(value.get("period").unwrap_or_else(|| panic!("{at}: spring needs period")), "ms", at);
            ("Spring", format!("Spring {{ damping_ratio: {}, period_ms: {} }}", float(damping), float(period)))
        }
        other => {
            let _ = root;
            panic!("{at}: unsupported $type {other}")
        }
    }
}

/// A number, or `{ "value": n, "unit": u }` with the expected unit.
fn measure(value: &Value, unit: &str, at: &str) -> f64 {
    if let Some(n) = value.as_f64() {
        return n;
    }
    let n = value.get("value").and_then(Value::as_f64).unwrap_or_else(|| panic!("{at}: expected a number or {{value, unit}}"));
    let u = value.get("unit").and_then(Value::as_str).unwrap_or(unit);
    assert_eq!(u, unit, "{at}: expected unit {unit}");
    n
}

/// `"#RRGGBB"`, `"#RRGGBBAA"`, or `{ "hex": "#RRGGBB", "alpha": a }`.
fn color(value: &Value, at: &str) -> String {
    let (hex, alpha) = match value {
        Value::String(s) => (s.as_str(), None),
        Value::Object(map) => (
            map.get("hex").and_then(Value::as_str).unwrap_or_else(|| panic!("{at}: colour object needs hex")),
            map.get("alpha").and_then(Value::as_f64),
        ),
        _ => panic!("{at}: unsupported colour value"),
    };
    let digits = hex.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).unwrap_or_else(|_| panic!("{at}: bad hex {hex}")) as f64 / 255.0;
    let a = match (digits.len(), alpha) {
        (6, a) => a.unwrap_or(1.0),
        (8, None) => channel(6),
        _ => panic!("{at}: bad colour {hex}"),
    };
    format!("Color {{ r: {}, g: {}, b: {}, a: {} }}", float(channel(0)), float(channel(2)), float(channel(4)), float(a))
}

fn float(n: f64) -> String {
    let s = format!("{}", n as f32);
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

fn const_name(key: &str) -> String {
    key.replace('-', "_").to_uppercase()
}

fn mod_name(key: &str) -> String {
    key.replace('-', "_").to_lowercase()
}
