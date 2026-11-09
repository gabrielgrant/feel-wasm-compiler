//! `runtime.wasm`: the FEEL runtime as a standalone WebAssembly module.
//!
//! Exports:
//! * every `feel_rt_v1` ABI function (see `feel_runtime::abi`), imported by
//!   generated expression modules;
//! * host-facing functions (`rt_*`) used by the JavaScript bindings to load
//!   constants, pass inputs/outputs as FEEL-JSON, configure limits and the
//!   clock, and read diagnostics.
//!
//! Imports (`feel_host_v1`): `apply(f, args) -> h` (call back into the
//! generated module's `__feel_apply`) and `call_host(f, args) -> h` (external
//! and host functions).
//!
//! One runtime instance serves exactly one expression-module instance; this
//! is the isolation unit.

#![allow(clippy::missing_safety_doc)]

use core::cell::UnsafeCell;
use feel_runtime::abi;
use feel_runtime::{Cx, Runtime, H};

struct Global(UnsafeCell<Option<Runtime>>);
unsafe impl Sync for Global {}
static RT: Global = Global(UnsafeCell::new(None));

/// Short-lived access to the instance runtime. Never hold the returned
/// reference across a host callback (the `Cx` discipline enforces this).
#[allow(clippy::mut_from_ref)]
fn rt() -> &'static mut Runtime {
    unsafe {
        let slot = &mut *RT.0.get();
        if slot.is_none() {
            *slot = Some(Runtime::new());
        }
        slot.as_mut().unwrap()
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "feel_host_v1")]
extern "C" {
    #[link_name = "apply"]
    fn host_apply(f: i32, args: i32) -> i32;
    #[link_name = "call_host"]
    fn host_call(f: i32, args: i32) -> i32;
}

#[cfg(not(target_arch = "wasm32"))]
unsafe fn host_apply(_f: i32, _args: i32) -> i32 {
    0
}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn host_call(_f: i32, _args: i32) -> i32 {
    0
}

struct WasmCx;
impl Cx for WasmCx {
    fn rt(&mut self) -> &mut Runtime {
        rt()
    }
    fn apply(&mut self, f: H, args: H) -> H {
        unsafe { host_apply(f as i32, args as i32) as u32 }
    }
    fn call_host(&mut self, f: H, args: H) -> H {
        unsafe { host_call(f as i32, args as i32) as u32 }
    }
}

#[inline]
fn check_abort() {
    if rt().aborted() {
        #[cfg(target_arch = "wasm32")]
        core::arch::wasm32::unreachable();
    }
}

macro_rules! export_abi {
    ($($name:ident($($p:ident),*) -> $r:tt;)*) => {
        $( export_abi!(@one $name ($($p),*) $r); )*
    };
    (@one $name:ident ($($p:ident),*) 1) => {
        #[export_name = concat!("f_", stringify!($name))]
        pub extern "C" fn $name($($p: i32),*) -> i32 {
            let v = abi::$name(&mut WasmCx, $($p),*);
            check_abort();
            v
        }
    };
    (@one $name:ident ($($p:ident),*) 0) => {
        #[export_name = concat!("f_", stringify!($name))]
        pub extern "C" fn $name($($p: i32),*) {
            let _ = abi::$name(&mut WasmCx, $($p),*);
            check_abort();
        }
    };
}
feel_runtime::feel_abi_list!(export_abi);

// ------------------------------------------------------------------ host-facing

fn read_str(ptr: i32, len: i32) -> &'static str {
    unsafe {
        let s = core::slice::from_raw_parts(ptr as *const u8, len as usize);
        core::str::from_utf8_unchecked(s)
    }
}

fn set_out(s: String) -> i32 {
    let r = rt();
    r.out = s.into_bytes();
    r.out.as_ptr() as i32
}

#[no_mangle]
pub extern "C" fn rt_abi_version() -> i32 {
    abi::ABI_VERSION as i32
}

/// Allocate `len` bytes for the host to write into.
#[no_mangle]
pub extern "C" fn rt_alloc(len: i32) -> i32 {
    let mut v: Vec<u8> = Vec::with_capacity(len.max(1) as usize);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p as i32
}

#[no_mangle]
pub unsafe extern "C" fn rt_free(ptr: i32, len: i32) {
    drop(Vec::from_raw_parts(ptr as *mut u8, 0, len.max(1) as usize));
}

/// Length of the last output buffer (`rt_encode`, `rt_diags`, ...).
#[no_mangle]
pub extern "C" fn rt_out_len() -> i32 {
    rt().out.len() as i32
}

/// Load the module's constant pool (FEEL-JSON array). Returns the number of
/// constants or -1 (error message in the output buffer).
#[no_mangle]
pub extern "C" fn rt_load_consts(ptr: i32, len: i32) -> i32 {
    match feel_runtime::load_constants(rt(), read_str(ptr, len)) {
        Ok(n) => n as i32,
        Err(e) => {
            set_out(e);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_set_limits(max_values: i32, max_collection: i32, max_string: i32, max_diags: i32, max_depth: i32) {
    let r = rt();
    r.limits.max_values = max_values as u32;
    r.limits.max_collection = max_collection as u32;
    r.limits.max_string = max_string as u32;
    r.limits.max_diags = max_diags as u32;
    r.limits.max_depth = max_depth as u32;
}

#[no_mangle]
pub extern "C" fn rt_set_extensions(flags: i32) {
    rt().extensions = flags as u32;
}

/// Set the clock instant (date and time string); returns 1 on success.
#[no_mangle]
pub extern "C" fn rt_set_clock(ptr: i32, len: i32) -> i32 {
    if len == 0 {
        rt().clock = None;
        return 1;
    }
    abi::set_clock(rt(), read_str(ptr, len)) as i32
}

#[no_mangle]
pub extern "C" fn rt_begin_eval() {
    rt().begin_eval();
}

/// Decode FEEL-JSON into the evaluation region. Returns a handle, or -2 on
/// error (message in the output buffer).
#[no_mangle]
pub extern "C" fn rt_decode(ptr: i32, len: i32) -> i32 {
    match feel_runtime::json::decode(rt(), read_str(ptr, len)) {
        Ok(h) => h as i32,
        Err(e) => {
            set_out(e.to_string());
            -2
        }
    }
}

/// Bind input slots. `names` is a JSON array of slot names, `types` a JSON
/// array of declared types (or null). `candidate` is the unary-test input
/// handle or -1. Returns 1 on success, 0 on a boundary error (message in the
/// output buffer).
#[no_mangle]
pub extern "C" fn rt_bind_inputs(inputs: i32, candidate: i32, names_h: i32, types_h: i32, validate: i32) -> i32 {
    let r = rt();
    let names: Vec<String> =
        r.list_items(names_h as u32).unwrap_or_default().iter().map(|h| r.as_str(*h).map(|s| s.to_string()).unwrap_or_default()).collect();
    let types: Vec<Option<String>> =
        r.list_items(types_h as u32).unwrap_or_default().iter().map(|h| r.as_str(*h).map(|s| s.to_string())).collect();
    let cand = if candidate < 0 { None } else { Some(candidate as u32) };
    match abi::bind_inputs(r, inputs as u32, cand, &names, &types, validate != 0) {
        Ok(()) => 1,
        Err(e) => {
            set_out(e);
            0
        }
    }
}

/// Encode a value as FEEL-JSON into the output buffer; returns its pointer.
#[no_mangle]
pub extern "C" fn rt_encode(h: i32) -> i32 {
    let s = feel_runtime::json::encode(rt(), h as u32);
    set_out(s)
}

#[no_mangle]
pub extern "C" fn rt_diags() -> i32 {
    let s = abi::diags_json(rt());
    set_out(s)
}

/// Abort information as JSON (`null` when the evaluation was not aborted).
#[no_mangle]
pub extern "C" fn rt_abort() -> i32 {
    let s = abi::abort_json(rt());
    set_out(s)
}

/// Number of values currently allocated in the evaluation region.
#[no_mangle]
pub extern "C" fn rt_eval_values() -> i32 {
    rt().eval_values() as i32
}

/// Clear an abort after the host has handled it.
#[no_mangle]
pub extern "C" fn rt_clear_abort() {
    rt().abort = None;
}
