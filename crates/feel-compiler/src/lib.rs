//! FEEL (DMN 1.5) → WebAssembly compiler.
//!
//! Pipeline: [`parse`] (source-spanned AST with scope-aware multi-word names)
//! → optional [`check`] (schema-aware types and diagnostics) → lowering to a
//! typed IR (name resolution, closure conversion) → optional optimization →
//! Wasm emission. The emitted module contains code specialized to the
//! expression and imports only primitive helpers from the shared runtime
//! (`feel_rt_v1`); it never embeds or calls an interpreter.
//!
//! ```
//! use feel_compiler::{compile, CompileOptions};
//! let out = compile("if x > 10 then \"big\" else \"small\"", &CompileOptions::default());
//! assert!(out.wasm.is_some());
//! ```

pub mod ast;
pub mod check;
pub mod codegen;
pub mod deps;
pub mod diag;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod opt;
pub mod parser;
pub mod types;

use ast::Program;
use check::CheckMode;
use diag::{Diagnostic, Severity, Span};
use parser::{NameSet, ParseOptions, Parser};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use types::Schema;

pub use deps::Dependencies;
pub use feel_runtime::abi::{ABI_MODULE, ABI_VERSION};

pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Entry {
    /// An ordinary FEEL expression (grammar rule 1).
    #[default]
    Expression,
    /// Unary tests (grammar rule 15) applied to a supplied candidate `?`.
    UnaryTests,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Dialect {
    /// OMG DMN 1.5 FEEL (the conformance baseline).
    #[default]
    #[serde(rename = "feel-1.5")]
    Feel15,
}

/// Opt-in extensions (outside the DMN 1.5 conformance claim).
pub const EXTENSIONS: &[(&str, u32)] = &[("has key", feel_runtime::EXT_HAS_KEY)];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CompileOptions {
    pub entry: Entry,
    pub dialect: Dialect,
    pub schema: Schema,
    pub check: CheckMode,
    pub optimize: bool,
    pub extensions: Vec<String>,
    /// Validate inputs against declared schema types at the runtime boundary.
    pub validate_inputs: bool,
    /// Additional names to recognise as (multi-word) names when parsing, e.g.
    /// DRG element names supplied by a DMN integration layer.
    pub known_names: Vec<String>,
    /// Additional context field / property names recognised after `.`.
    pub known_fields: Vec<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            entry: Entry::Expression,
            dialect: Dialect::Feel15,
            schema: Schema::default(),
            check: CheckMode::Permissive,
            optimize: true,
            extensions: Vec::new(),
            validate_inputs: true,
            known_names: Vec::new(),
            known_fields: Vec::new(),
        }
    }
}

impl CompileOptions {
    pub fn extension_flags(&self) -> u32 {
        let mut f = 0;
        for e in &self.extensions {
            if let Some((_, b)) = EXTENSIONS.iter().find(|(n, _)| n == e) {
                f |= b;
            }
        }
        f
    }
    /// Stable cache key: source, schema, dialect, options, compiler and
    /// runtime/ABI versions.
    pub fn cache_key(&self, source: &str) -> String {
        let mut h = Sha256::new();
        h.update(b"feel-wasm-cache-v1\0");
        h.update(COMPILER_VERSION.as_bytes());
        h.update(b"\0");
        h.update(feel_runtime::RUNTIME_VERSION.as_bytes());
        h.update(format!("\0abi{}\0", ABI_VERSION).as_bytes());
        h.update(serde_json::to_string(self).unwrap_or_default().as_bytes());
        h.update(b"\0");
        h.update(source.as_bytes());
        let d = h.finalize();
        d.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Lowering environment derived from the options.
pub struct Env {
    pub inputs: Vec<InputDecl>,
    pub host_functions: Vec<HostFnDecl>,
    pub auto_inputs: bool,
    pub extensions: u32,
    pub candidate_type: Option<String>,
    pub aliases: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct InputDecl {
    pub name: String,
    /// FEEL runtime type string used for boundary validation
    pub ty: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HostFnDecl {
    pub name: String,
    pub params: Vec<(String, Option<String>)>,
}

impl Env {
    /// Resolve type aliases (item definitions) to structural type strings.
    pub fn resolve_type_name(&self, s: &str) -> String {
        if let Some((_, d)) = self.aliases.iter().find(|(a, _)| a == s) {
            return match types::parse_schema_type(d, &self.aliases) {
                Ok(t) => t.to_runtime_string(),
                Err(_) => d.clone(),
            };
        }
        if types::named(s).is_some() || s.contains('<') {
            match types::parse_schema_type(s, &self.aliases) {
                Ok(t) => t.to_runtime_string(),
                Err(_) => s.to_string(),
            }
        } else {
            s.to_string()
        }
    }

    pub fn from_options(o: &CompileOptions) -> Result<Env, Diagnostic> {
        let aliases = o.schema.types.clone();
        let mut inputs = Vec::new();
        for i in &o.schema.inputs {
            let ty = match &i.ty {
                Some(t) => Some(
                    types::parse_schema_type(t, &aliases)
                        .map_err(|e| Diagnostic::error("FEEL-B501", format!("schema: input '{}': {e}", i.name), Span::default()))?
                        .to_runtime_string(),
                ),
                None => None,
            };
            inputs.push(InputDecl { name: i.name.clone(), ty: if o.validate_inputs { ty } else { None } });
        }
        let mut host_functions = Vec::new();
        for f in &o.schema.functions {
            let mut ps = Vec::new();
            for p in &f.params {
                let t = match &p.ty {
                    Some(t) => Some(
                        types::parse_schema_type(t, &aliases)
                            .map_err(|e| Diagnostic::error("FEEL-B501", format!("schema: function '{}': {e}", f.name), Span::default()))?
                            .to_runtime_string(),
                    ),
                    None => None,
                };
                ps.push((p.name.clone(), t));
            }
            host_functions.push(HostFnDecl { name: f.name.clone(), params: ps });
        }
        let candidate_type = match &o.schema.candidate {
            Some(t) if o.validate_inputs => types::parse_schema_type(t, &aliases).ok().map(|t| t.to_runtime_string()),
            _ => None,
        };
        Ok(Env { inputs, host_functions, auto_inputs: !o.schema.closed, extensions: o.extension_flags(), candidate_type, aliases })
    }
}

fn builtin_names(ext: u32) -> NameSet {
    let mut s = NameSet::new();
    for b in feel_runtime::builtins::BUILTINS {
        if b.extension == 0 || ext & b.extension != 0 {
            s.insert(b.name);
        }
    }
    s
}

#[derive(Clone, Debug, Serialize)]
pub struct ParseOutput {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

fn parse_options(o: &CompileOptions) -> ParseOptions {
    let mut po = ParseOptions::default();
    for i in &o.schema.inputs {
        po.names.insert(&i.name);
    }
    for f in &o.schema.functions {
        po.names.insert(&f.name);
    }
    for n in &o.known_names {
        po.names.insert(n);
    }
    for n in &o.known_fields {
        po.fields.insert(n);
    }
    for (a, d) in &o.schema.types {
        po.type_names.insert(a);
        if let Ok(t) = types::parse_schema_type(d, &o.schema.types) {
            collect_fields(&t, &mut po.fields);
        }
    }
    for i in &o.schema.inputs {
        if let Some(t) = &i.ty {
            if let Ok(t) = types::parse_schema_type(t, &o.schema.types) {
                collect_fields(&t, &mut po.fields);
            }
        }
    }
    po
}

fn collect_fields(t: &types::Ty, out: &mut NameSet) {
    match t {
        types::Ty::Context(fs, _) => {
            for f in fs {
                out.insert(&f.name);
                collect_fields(&f.ty, out);
            }
        }
        types::Ty::List(x) | types::Ty::Range(x) | types::Ty::Nullable(x) => collect_fields(x, out),
        _ => {}
    }
}

/// Parse FEEL source as an expression or unary tests.
pub fn parse(source: &str, opts: &CompileOptions) -> ParseOutput {
    let po = parse_options(opts);
    let builtins = builtin_names(opts.extension_flags());
    let mut diags = Vec::new();
    let program = match Parser::new(source, &po, &builtins) {
        Err(d) => {
            diags.push(d);
            None
        }
        Ok(mut p) => {
            let r = match opts.entry {
                Entry::Expression => p.parse_expression_program().map(Program::Expression),
                Entry::UnaryTests => {
                    let span = Span::new(0, source.len());
                    p.parse_unary_tests_program().map(|t| Program::UnaryTests(t, span))
                }
            };
            diags.extend(p.diagnostics.drain(..));
            match r {
                Ok(prog) => Some(prog),
                Err(d) => {
                    diags.push(d);
                    None
                }
            }
        }
    };
    for d in diags.iter_mut() {
        d.locate(source);
    }
    ParseOutput { program: if diags.iter().any(|d| d.is_error()) { None } else { program }, diagnostics: diags }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutput {
    pub ok: bool,
    pub result_type: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse and type-check (CEL-style `check`); never emits code.
pub fn check(source: &str, opts: &CompileOptions) -> CheckOutput {
    let p = parse(source, opts);
    let mut diags = p.diagnostics;
    let Some(prog) = p.program else {
        return CheckOutput { ok: false, result_type: None, diagnostics: diags };
    };
    let mode = if opts.check == CheckMode::Off { CheckMode::Permissive } else { opts.check };
    let rt = match check::env_from_schema(&opts.schema, mode, opts.extension_flags()) {
        Ok(env) => {
            let out = check::check_program(&prog, &env);
            diags.extend(out.diagnostics);
            Some(out.result.to_string())
        }
        Err(d) => {
            diags.push(d);
            None
        }
    };
    for d in diags.iter_mut() {
        d.locate(source);
    }
    CheckOutput { ok: !diags.iter().any(|d| d.is_error()), result_type: rt, diagnostics: diags }
}

/// Module metadata, also embedded in the `feel.meta` custom section.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub format: String,
    pub abi_version: u32,
    pub abi_module: String,
    pub compiler_version: String,
    pub runtime_version: String,
    pub dialect: String,
    pub entry: Entry,
    pub extensions: Vec<String>,
    pub extension_flags: u32,
    /// input slots in order (`?` is the unary-test candidate)
    pub inputs: Vec<SlotInfo>,
    pub candidate_slot: Option<u32>,
    pub result_type: Option<String>,
    pub const_count: u32,
    pub function_count: u32,
    pub imports: Vec<String>,
    pub sites: Vec<lower::SiteInfo>,
    pub dependencies: Dependencies,
    pub optimized: bool,
    pub folded_constants: u32,
    pub fused_filters: u32,
    pub source_sha256: String,
    pub cache_key: String,
    pub validate_inputs: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub name: String,
    pub slot: u32,
    #[serde(rename = "type")]
    pub ty: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileOutput {
    #[serde(skip)]
    pub wasm: Option<Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
    pub metadata: Option<Metadata>,
    /// constant pool (FEEL-JSON array), also in the `feel.consts` section
    pub constants: Option<String>,
}

impl CompileOutput {
    pub fn ok(&self) -> bool {
        self.wasm.is_some()
    }
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error)
    }
}

/// Lower to IR only (for inspection and tests).
pub fn lower(source: &str, opts: &CompileOptions) -> Result<(ir::ProgramIr, lower::ConstPool, Vec<String>), Vec<Diagnostic>> {
    let p = parse(source, opts);
    let Some(prog) = p.program else { return Err(p.diagnostics) };
    let env = Env::from_options(opts).map_err(|d| vec![d])?;
    let mut l = lower::Lowerer::new(&env);
    let mut ir = l.lower_program(&prog);
    if !l.diags.is_empty() {
        return Err(l.diags);
    }
    if opts.optimize {
        opt::optimize(&mut ir, &mut l.consts);
    }
    Ok((ir, l.consts, l.slots))
}

/// Parse, (optionally) check, lower, (optionally) optimize and emit Wasm.
pub fn compile(source: &str, opts: &CompileOptions) -> CompileOutput {
    let fail = |mut diagnostics: Vec<Diagnostic>| {
        for d in diagnostics.iter_mut() {
            d.locate(source);
        }
        CompileOutput { wasm: None, diagnostics, metadata: None, constants: None }
    };
    let p = parse(source, opts);
    let diags = p.diagnostics;
    let Some(prog) = p.program else { return fail(diags) };
    compile_program(&prog, source, diags, opts)
}

/// Compile an already-parsed (or programmatically constructed) program.
/// `source` is only used for diagnostics positions and the source hash.
pub fn compile_program(prog: &Program, source: &str, mut diags: Vec<Diagnostic>, opts: &CompileOptions) -> CompileOutput {
    let fail = |mut diagnostics: Vec<Diagnostic>| {
        for d in diagnostics.iter_mut() {
            d.locate(source);
        }
        CompileOutput { wasm: None, diagnostics, metadata: None, constants: None }
    };
    let mut result_type = None;
    if opts.check != CheckMode::Off {
        match check::env_from_schema(&opts.schema, opts.check, opts.extension_flags()) {
            Ok(env) => {
                let out = check::check_program(prog, &env);
                result_type = Some(out.result.to_string());
                diags.extend(out.diagnostics);
            }
            Err(d) => diags.push(d),
        }
        if diags.iter().any(|d| d.is_error()) {
            return fail(diags);
        }
    }
    let env = match Env::from_options(opts) {
        Ok(e) => e,
        Err(d) => {
            diags.push(d);
            return fail(diags);
        }
    };
    let mut l = lower::Lowerer::new(&env);
    let mut ir = l.lower_program(prog);
    if !l.diags.is_empty() {
        diags.extend(l.diags.drain(..));
        return fail(diags);
    }
    let (folded, fused) = if opts.optimize { opt::optimize(&mut ir, &mut l.consts) } else { (0, 0) };
    let deps = deps::extract(&ir, &l.slots, &l.consts.items);
    let consts_json = l.consts.to_json();
    let inputs: Vec<SlotInfo> =
        l.slots.iter().enumerate().map(|(i, n)| SlotInfo { name: n.clone(), slot: i as u32, ty: l.slot_types[i].clone() }).collect();
    let candidate_slot = l.slots.iter().position(|s| s == "?").map(|i| i as u32);
    let mut meta = Metadata {
        format: "feel-wasm-module".into(),
        abi_version: ABI_VERSION,
        abi_module: ABI_MODULE.into(),
        compiler_version: COMPILER_VERSION.into(),
        runtime_version: feel_runtime::RUNTIME_VERSION.into(),
        dialect: "feel-1.5".into(),
        entry: opts.entry,
        extensions: opts.extensions.clone(),
        extension_flags: opts.extension_flags(),
        inputs,
        candidate_slot,
        result_type,
        const_count: l.consts.items.len() as u32,
        function_count: ir.funcs.len() as u32,
        imports: Vec::new(),
        sites: l.sites.clone(),
        dependencies: deps,
        optimized: opts.optimize,
        folded_constants: folded,
        fused_filters: fused,
        source_sha256: {
            let d = Sha256::digest(source.as_bytes());
            d.iter().map(|b| format!("{b:02x}")).collect()
        },
        cache_key: opts.cache_key(source),
        validate_inputs: opts.validate_inputs,
    };
    // the import list is only known after emission; emit twice is avoided by
    // computing it from a first emission
    let first = codegen::emit_module(&codegen::EmitInput { prog: &ir, consts_json: &consts_json, meta_json: "{}" });
    meta.imports = first.imports.clone();
    let meta_json = serde_json::to_string(&meta).unwrap();
    let emitted = codegen::emit_module(&codegen::EmitInput { prog: &ir, consts_json: &consts_json, meta_json: &meta_json });
    for d in diags.iter_mut() {
        d.locate(source);
    }
    CompileOutput { wasm: Some(emitted.bytes), diagnostics: diags, metadata: Some(meta), constants: Some(consts_json) }
}

/// Read the metadata and constant pool from a compiled module's custom
/// sections, validating the ABI version.
pub fn read_module_sections(bytes: &[u8]) -> Result<(Metadata, String), String> {
    let mut meta = None;
    let mut consts = None;
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| format!("invalid wasm: {e}"))?;
        if let wasmparser::Payload::CustomSection(c) = payload {
            match c.name() {
                "feel.meta" => meta = Some(String::from_utf8_lossy(c.data()).to_string()),
                "feel.consts" => consts = Some(String::from_utf8_lossy(c.data()).to_string()),
                _ => {}
            }
        }
    }
    let meta = meta.ok_or("not a FEEL module: missing feel.meta section")?;
    let consts = consts.ok_or("not a FEEL module: missing feel.consts section")?;
    let m: Metadata = serde_json::from_str(&meta).map_err(|e| format!("invalid feel.meta: {e}"))?;
    if m.abi_version != ABI_VERSION || m.abi_module != ABI_MODULE {
        return Err(format!(
            "incompatible module: compiled for runtime ABI {} v{}, this runtime provides {} v{}",
            m.abi_module, m.abi_version, ABI_MODULE, ABI_VERSION
        ));
    }
    Ok((m, consts))
}

/// Validate a module with wasmparser (all proposals used: none beyond MVP +
/// i64/multi-value-free core).
pub fn validate_module(bytes: &[u8]) -> Result<(), String> {
    let mut v = wasmparser::Validator::new();
    v.validate_all(bytes).map(|_| ()).map_err(|e| e.to_string())
}
