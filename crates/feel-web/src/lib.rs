//! `compiler.wasm`: the FEEL → Wasm compiler as a WebAssembly module.
//!
//! The same `feel-compiler` source and backend as the native CLI, exposed
//! through a minimal C ABI (no wasm-bindgen) so that any JavaScript host can
//! compile FEEL entirely offline:
//!
//! | export | description |
//! |---|---|
//! | `fw_alloc(len) -> ptr`, `fw_free(ptr, len)` | buffers for passing UTF-8 text in |
//! | `fw_compile(src, src_len, opts, opts_len) -> ok` | compile; JSON report + module bytes |
//! | `fw_check(...) -> ok`, `fw_parse(...) -> ok` | check / parse only; JSON report |
//! | `fw_version() -> ok` | JSON version info |
//! | `fw_out_ptr/fw_out_len` | the last JSON report |
//! | `fw_wasm_ptr/fw_wasm_len` | the last emitted module |
//!
//! Options are the JSON form of `feel_compiler::CompileOptions` (camelCase).

use feel_compiler::{CompileOptions, ABI_MODULE, ABI_VERSION, COMPILER_VERSION};
use std::cell::RefCell;

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static WASM: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(s: String) {
    OUT.with(|o| *o.borrow_mut() = s.into_bytes());
}

unsafe fn text<'a>(ptr: i32, len: i32) -> &'a str {
    if len <= 0 {
        return "";
    }
    let b = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    std::str::from_utf8(b).unwrap_or("")
}

fn options(json: &str) -> Result<CompileOptions, String> {
    if json.trim().is_empty() {
        return Ok(CompileOptions::default());
    }
    parse_options(json)
}

/// Parse options JSON (see `CompileOptions::from_json`).
pub fn parse_options(json: &str) -> Result<CompileOptions, String> {
    CompileOptions::from_json(json)
}

#[no_mangle]
pub extern "C" fn fw_alloc(len: i32) -> i32 {
    let mut v: Vec<u8> = Vec::with_capacity(len.max(1) as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p as i32
}

/// # Safety
/// `ptr`/`len` must come from `fw_alloc`.
#[no_mangle]
pub unsafe extern "C" fn fw_free(ptr: i32, len: i32) {
    drop(Vec::from_raw_parts(ptr as *mut u8, 0, len.max(1) as usize));
}

#[no_mangle]
pub extern "C" fn fw_out_ptr() -> i32 {
    OUT.with(|o| o.borrow().as_ptr() as i32)
}
#[no_mangle]
pub extern "C" fn fw_out_len() -> i32 {
    OUT.with(|o| o.borrow().len() as i32)
}
#[no_mangle]
pub extern "C" fn fw_wasm_ptr() -> i32 {
    WASM.with(|o| o.borrow().as_ptr() as i32)
}
#[no_mangle]
pub extern "C" fn fw_wasm_len() -> i32 {
    WASM.with(|o| o.borrow().len() as i32)
}

pub fn version_json() -> String {
    serde_json::json!({
        "compiler": COMPILER_VERSION,
        "runtime": feel_runtime::RUNTIME_VERSION,
        "abiModule": ABI_MODULE,
        "abiVersion": ABI_VERSION,
        "dialect": "feel-1.5",
        "tzdb": feel_runtime::temporal::tzdb_version(),
    })
    .to_string()
}

#[no_mangle]
pub extern "C" fn fw_version() -> i32 {
    set_out(version_json());
    1
}

/// Compile and return `(report JSON, module bytes)`.
pub fn compile_json(src: &str, opts_json: &str) -> (String, Option<Vec<u8>>) {
    let opts = match options(opts_json) {
        Ok(o) => o,
        Err(e) => {
            return (
                serde_json::json!({"ok": false, "error": e}).to_string(),
                None,
            )
        }
    };
    let out = feel_compiler::compile(src, &opts);
    let ok = out.wasm.is_some();
    let report = serde_json::json!({
        "ok": ok,
        "diagnostics": out.diagnostics,
        "metadata": out.metadata,
        "constants": out.constants,
        "cacheKey": opts.cache_key(src),
    });
    (report.to_string(), out.wasm)
}

/// # Safety
/// Pointers must reference `len` readable bytes (from `fw_alloc`).
#[no_mangle]
pub unsafe extern "C" fn fw_compile(src: i32, src_len: i32, opts: i32, opts_len: i32) -> i32 {
    let (report, wasm) = compile_json(text(src, src_len), text(opts, opts_len));
    let ok = wasm.is_some();
    WASM.with(|w| *w.borrow_mut() = wasm.unwrap_or_default());
    set_out(report);
    ok as i32
}

/// # Safety
/// Pointers must reference `len` readable bytes (from `fw_alloc`).
#[no_mangle]
pub unsafe extern "C" fn fw_check(src: i32, src_len: i32, opts: i32, opts_len: i32) -> i32 {
    let opts = match options(text(opts, opts_len)) {
        Ok(o) => o,
        Err(e) => {
            set_out(serde_json::json!({"ok": false, "error": e}).to_string());
            return 0;
        }
    };
    let out = feel_compiler::check(text(src, src_len), &opts);
    let ok = out.ok;
    set_out(serde_json::to_string(&out).unwrap_or_default());
    ok as i32
}

/// # Safety
/// Pointers must reference `len` readable bytes (from `fw_alloc`).
#[no_mangle]
pub unsafe extern "C" fn fw_parse(src: i32, src_len: i32, opts: i32, opts_len: i32) -> i32 {
    let opts = match options(text(opts, opts_len)) {
        Ok(o) => o,
        Err(e) => {
            set_out(serde_json::json!({"ok": false, "error": e}).to_string());
            return 0;
        }
    };
    let out = feel_compiler::parse(text(src, src_len), &opts);
    let ok = out.program.is_some();
    set_out(
        serde_json::json!({"ok": ok, "ast": out.program, "diagnostics": out.diagnostics})
            .to_string(),
    );
    ok as i32
}
