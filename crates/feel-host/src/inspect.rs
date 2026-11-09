//! Human-readable module inspection (imports, exports, function sizes,
//! custom sections) used by `feel inspect` and the "no interpreter import"
//! checks.

/// Describe a module, one line per item.
pub fn describe(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut nfunc = 0;
    for p in wasmi_parser(bytes) {
        out.push(p);
        nfunc += 1;
    }
    let _ = nfunc;
    out
}

fn wasmi_parser(bytes: &[u8]) -> Vec<String> {
    use feel_compiler::read_module_sections;
    let mut v = Vec::new();
    match ModuleSummary::of(bytes) {
        Ok(s) => {
            v.push(format!("size: {} bytes", bytes.len()));
            for i in &s.imports {
                v.push(format!("import {i}"));
            }
            for e in &s.exports {
                v.push(format!("export {e}"));
            }
            v.push(format!("defined functions: {} (code bytes: {:?})", s.code_sizes.len(), s.code_sizes));
            for c in &s.custom {
                v.push(format!("custom section {c}"));
            }
        }
        Err(e) => v.push(format!("error: {e}")),
    }
    if let Err(e) = read_module_sections(bytes) {
        v.push(format!("metadata error: {e}"));
    }
    v
}

/// Structural summary of a module.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModuleSummary {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub code_sizes: Vec<usize>,
    pub custom: Vec<String>,
    pub has_memory: bool,
}

impl ModuleSummary {
    pub fn of(bytes: &[u8]) -> Result<ModuleSummary, String> {
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let mut s = ModuleSummary::default();
        for i in m.imports() {
            s.imports.push(format!("{}.{}", i.module(), i.name()));
            if matches!(i.ty(), wasmi::ExternType::Memory(_)) {
                s.has_memory = true;
            }
        }
        for e in m.exports() {
            s.exports.push(e.name().to_string());
            if matches!(e.ty(), wasmi::ExternType::Memory(_)) {
                s.has_memory = true;
            }
        }
        // code sizes and custom sections via a minimal section walk
        let mut i = 8usize;
        while i < bytes.len() {
            let id = bytes[i];
            i += 1;
            let (len, n) = leb(&bytes[i..]);
            i += n;
            let body = &bytes[i..(i + len).min(bytes.len())];
            match id {
                0 => {
                    let (nl, k) = leb(body);
                    let name = String::from_utf8_lossy(&body[k..k + nl]).to_string();
                    s.custom.push(format!("{name} ({} bytes)", body.len() - k - nl));
                }
                5 => s.has_memory = true,
                10 => {
                    let (count, mut k) = leb(body);
                    for _ in 0..count {
                        let (sz, kk) = leb(&body[k..]);
                        k += kk;
                        s.code_sizes.push(sz);
                        k += sz;
                    }
                }
                _ => {}
            }
            i += len;
        }
        Ok(s)
    }
}

fn leb(b: &[u8]) -> (usize, usize) {
    let mut r = 0usize;
    let mut shift = 0;
    let mut i = 0;
    while i < b.len() {
        let x = b[i];
        r |= ((x & 0x7f) as usize) << shift;
        i += 1;
        if x & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    (r, i)
}
