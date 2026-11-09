//! The shared FEEL runtime.
//!
//! This crate implements FEEL values, operators, the standard builtin
//! catalog, the FEEL-JSON interchange format and the versioned `feel_rt_v1`
//! ABI used by modules emitted by `feel-compiler`. It is compiled
//! * natively, where `feel-host` binds the ABI to wasmi host functions and
//!   the compiler uses it for constant folding, and
//! * to `wasm32-unknown-unknown` via `feel-runtime-wasm`, producing
//!   `runtime.wasm` for browsers and other Wasm hosts.

pub mod abi;
pub mod builtins;
pub mod json;
pub mod ops;
pub mod rangelit;
pub mod regex_xpath;
pub mod types;
pub mod value;

pub use builtins::{Cx, NoCallbacks};
pub use value::*;

pub use feel_decimal::Decimal;
pub use feel_temporal as temporal;

/// Runtime crate version (part of compiled-module cache keys).
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Load a module's constant pool (a FEEL-JSON array) into the fixed region:
/// constant `i` gets handle `RESERVED + i`. Must be called once, on a fresh
/// runtime, before any evaluation.
pub fn load_constants(rt: &mut Runtime, json_text: &str) -> Result<u32, String> {
    if rt.vals.len() != RESERVED as usize {
        return Err("constants already loaded".to_string());
    }
    let saved = rt.limits.clone();
    rt.limits.max_values = u32::MAX;
    rt.limits.max_collection = u32::MAX;
    rt.limits.max_string = u32::MAX;
    // decode the array, reserving the top-level positions first
    let n = count_top_level(json_text)?;
    for _ in 0..n {
        rt.vals.push(Value::Null);
    }
    let arr = json::decode(rt, json_text).map_err(|e| e.to_string())?;
    let items = rt
        .list_items(arr)
        .ok_or("constant pool must be a JSON array")?;
    if items.len() != n as usize {
        return Err("constant pool length mismatch".to_string());
    }
    for (i, hd) in items.iter().enumerate() {
        let v = rt.get(*hd).clone();
        rt.vals[RESERVED as usize + i] = v;
    }
    rt.limits = saved;
    rt.freeze();
    Ok(n)
}

fn count_top_level(s: &str) -> Result<u32, String> {
    // count elements of a JSON array without materialising it
    let b = s.trim().as_bytes();
    if b.first() != Some(&b'[') {
        return Err("constant pool must be a JSON array".to_string());
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut count = 0u32;
    let mut saw_value = false;
    for &c in b {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                if depth == 1 {
                    saw_value = true;
                }
            }
            b'[' | b'{' => {
                if depth == 1 {
                    saw_value = true;
                }
                depth += 1;
            }
            b']' | b'}' => depth -= 1,
            b',' if depth == 1 => count += 1,
            c if depth == 1 && !c.is_ascii_whitespace() => saw_value = true,
            _ => {}
        }
    }
    Ok(if saw_value { count + 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constants_and_json_roundtrip() {
        let mut rt = Runtime::new();
        let n = load_constants(
            &mut rt,
            r#"["abc", 1.10, [1, "x"], {"a": {"@date": "2020-01-02"}}]"#,
        )
        .unwrap();
        assert_eq!(n, 4);
        assert_eq!(&*rt.as_str(RESERVED).unwrap(), "abc");
        assert_eq!(json::encode(&rt, RESERVED + 1), "1.10");
        assert_eq!(json::encode(&rt, RESERVED + 2), "[1,\"x\"]");
        assert_eq!(
            json::encode(&rt, RESERVED + 3),
            "{\"a\":{\"@date\":\"2020-01-02\"}}"
        );
        rt.begin_eval();
        let h = json::decode(&mut rt, "123456789012345678901234567890.123").unwrap();
        assert_eq!(json::encode(&rt, h), "123456789012345678901234567890.123");
    }
}
