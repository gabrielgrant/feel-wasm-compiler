//! FEEL values and the per-instance value arena.
//!
//! Generated code manipulates values exclusively through `i32` *handles*
//! (indices into [`Runtime::vals`]). The arena has two regions:
//!
//! * the **fixed region** `[0, fixed)`: `null`, `true`, `false` and the module's
//!   constant pool, loaded once per instance and never released;
//! * the **evaluation region** `[fixed, len)`: everything allocated while
//!   evaluating; it is truncated by [`Runtime::begin_eval`], so repeated
//!   evaluations do not accumulate memory.
//!
//! Values are immutable once published. Lists and contexts are built through
//! "builder" operations that are only applied to freshly allocated handles
//! (the code generator guarantees no aliasing during construction).

use feel_decimal::Decimal;
use feel_temporal::{Date, DateTime, DayTimeDuration, Time, YearMonthDuration};
use std::rc::Rc;

pub type H = u32;
pub const NULL: H = 0;
pub const TRUE: H = 1;
pub const FALSE: H = 2;
/// Sentinel returned by lookups that found nothing (distinct from `null`).
pub const ABSENT: H = u32::MAX;
/// Number of reserved handles before the constant pool.
pub const RESERVED: u32 = 3;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Num(Decimal),
    Str(Rc<str>),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Dtd(DayTimeDuration),
    Ymd(YearMonthDuration),
    List(Vec<H>),
    Ctx(Ctx),
    Range(Range),
    Func(Func),
}

#[derive(Clone, Debug, Default)]
pub struct Ctx {
    pub keys: Vec<Rc<str>>,
    pub vals: Vec<H>,
}

impl Ctx {
    pub fn get(&self, key: &str) -> Option<H> {
        self.keys.iter().position(|k| &**k == key).map(|i| self.vals[i])
    }
    pub fn put(&mut self, key: Rc<str>, v: H) {
        if let Some(i) = self.keys.iter().position(|k| *k == key) {
            self.vals[i] = v;
        } else {
            self.keys.push(key);
            self.vals.push(v);
        }
    }
    pub fn len(&self) -> usize {
        self.keys.len()
    }
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Range {
    /// `None` = unbounded (the "undefined" endpoint of `< e` style ranges).
    pub start: Option<H>,
    pub end: Option<H>,
    pub start_incl: bool,
    pub end_incl: bool,
    /// `!= e` used as a range value (DMN 1.5 rule 7.a): every value except
    /// `start` (= `end`).
    pub ne: bool,
    /// `= e` used as a range value: exactly `start` (= `end`). Distinct from
    /// the interval `[e..e]` for equality (TCK 0068).
    pub eq: bool,
}

#[derive(Clone, Debug)]
pub enum Func {
    Builtin(u16),
    /// A function defined in the generated module: table index, captured
    /// environment (a list handle) and signature constant (a list of
    /// `[name, type-or-null]` pairs).
    User { index: u32, env: H, sig: H },
    /// `function(..) external {...}`: mapping-information context and signature.
    External { mapping: H, sig: H },
    /// A host function bound by name (see the schema's `functions`).
    Host { name: Rc<str>, sig: H },
}

/// An evaluation diagnostic: produced alongside a FEEL `null` (or other
/// value) result. These never abort evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalDiag {
    pub code: &'static str,
    pub site: u32,
    pub message: String,
}

/// Why an evaluation was aborted (distinct from a FEEL `null` result).
#[derive(Clone, Debug, PartialEq)]
pub enum Abort {
    /// A resource limit was exceeded (`FEEL-L4xx`).
    Limit { code: &'static str, message: String },
    /// The host or ABI was misused (`FEEL-B5xx`).
    Boundary { code: &'static str, message: String },
}

impl Abort {
    pub fn code(&self) -> &'static str {
        match self {
            Abort::Limit { code, .. } | Abort::Boundary { code, .. } => code,
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Abort::Limit { message, .. } | Abort::Boundary { message, .. } => message,
        }
    }
}

/// Explicit evaluation limits for untrusted expressions. Step and call-depth
/// limits are enforced by generated code; the rest by the runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Maximum number of values allocated during one evaluation.
    pub max_values: u32,
    /// Maximum length of any list produced (incl. `for` ranges).
    pub max_collection: u32,
    /// Maximum string length in bytes.
    pub max_string: u32,
    /// Maximum number of loop iterations + function entries (fuel).
    pub max_steps: u64,
    /// Maximum user-function call depth.
    pub max_depth: u32,
    /// Maximum number of evaluation diagnostics retained.
    pub max_diags: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_values: 4_000_000,
            max_collection: 1_000_000,
            max_string: 16 * 1024 * 1024,
            max_steps: 50_000_000,
            max_depth: 512,
            max_diags: 64,
        }
    }
}

/// The clock capability: the instant `now()`/`today()` observe for one
/// evaluation (fixed for the whole evaluation).
#[derive(Clone, Debug)]
pub struct Clock {
    pub now: DateTime,
}

pub struct Runtime {
    pub vals: Vec<Value>,
    pub fixed: u32,
    pub slots: Vec<H>,
    pub diags: Vec<EvalDiag>,
    pub diag_overflow: u32,
    pub limits: Limits,
    pub clock: Option<Clock>,
    pub abort: Option<Abort>,
    /// Buffer used to hand encoded data (JSON) to the host.
    pub out: Vec<u8>,
    pub(crate) regex_cache: Vec<(String, String, Rc<fancy_regex::Regex>)>,
    pub(crate) type_cache: Vec<(H, Rc<crate::types::RtType>)>,
    pub extensions: u32,
}

/// Extension flags (opt-in, outside the DMN 1.5 conformance claim).
pub const EXT_HAS_KEY: u32 = 1;

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Runtime {
        Runtime {
            vals: vec![Value::Null, Value::Bool(true), Value::Bool(false)],
            fixed: RESERVED,
            slots: Vec::new(),
            diags: Vec::new(),
            diag_overflow: 0,
            limits: Limits::default(),
            clock: None,
            abort: None,
            out: Vec::new(),
            regex_cache: Vec::new(),
            type_cache: Vec::new(),
            extensions: 0,
        }
    }

    /// Start a fresh evaluation: release the evaluation region and reset
    /// per-evaluation state. Constants are kept.
    pub fn begin_eval(&mut self) {
        self.vals.truncate(self.fixed as usize);
        // release excess capacity after a large evaluation so that memory is
        // not retained without bound
        if self.vals.capacity() > 4 * (self.fixed as usize) + 65536 {
            self.vals.shrink_to(2 * self.fixed as usize + 16384);
        }
        self.slots.clear();
        self.diags.clear();
        self.diag_overflow = 0;
        self.abort = None;
        if self.out.capacity() > 1 << 20 {
            self.out = Vec::new();
        }
        self.type_cache.retain(|(h, _)| *h < self.fixed);
    }

    /// Mark everything currently allocated as part of the fixed region
    /// (used after loading the constant pool).
    pub fn freeze(&mut self) {
        self.fixed = self.vals.len() as u32;
    }

    pub fn eval_values(&self) -> u32 {
        self.vals.len() as u32 - self.fixed
    }

    #[inline]
    pub fn get(&self, h: H) -> &Value {
        match self.vals.get(h as usize) {
            Some(v) => v,
            None => &Value::Null,
        }
    }

    #[inline]
    pub fn alloc(&mut self, v: Value) -> H {
        match v {
            Value::Null => return NULL,
            Value::Bool(true) => return TRUE,
            Value::Bool(false) => return FALSE,
            _ => {}
        }
        if self.vals.len() as u64 - self.fixed as u64 >= self.limits.max_values as u64 {
            self.limit("FEEL-L401", format!("allocation limit exceeded ({} values)", self.limits.max_values));
            return NULL;
        }
        let h = self.vals.len() as H;
        self.vals.push(v);
        h
    }

    pub fn bool(&self, b: bool) -> H {
        if b {
            TRUE
        } else {
            FALSE
        }
    }

    pub fn num(&mut self, d: Decimal) -> H {
        self.alloc(Value::Num(d))
    }
    pub fn num_opt(&mut self, d: Option<Decimal>) -> H {
        match d {
            Some(d) => self.num(d),
            None => NULL,
        }
    }
    pub fn int(&mut self, i: i64) -> H {
        self.num(Decimal::from_i64(i))
    }
    pub fn str(&mut self, s: &str) -> H {
        if s.len() as u64 > self.limits.max_string as u64 {
            self.limit("FEEL-L403", format!("string length limit exceeded ({} bytes)", self.limits.max_string));
            return NULL;
        }
        self.alloc(Value::Str(Rc::from(s)))
    }
    pub fn string(&mut self, s: String) -> H {
        if s.len() as u64 > self.limits.max_string as u64 {
            self.limit("FEEL-L403", format!("string length limit exceeded ({} bytes)", self.limits.max_string));
            return NULL;
        }
        self.alloc(Value::Str(Rc::from(s)))
    }
    pub fn list(&mut self, items: Vec<H>) -> H {
        if items.len() as u64 > self.limits.max_collection as u64 {
            self.limit("FEEL-L402", format!("collection size limit exceeded ({} items)", self.limits.max_collection));
            return NULL;
        }
        self.alloc(Value::List(items))
    }
    pub fn ctx(&mut self, c: Ctx) -> H {
        self.alloc(Value::Ctx(c))
    }

    pub fn limit(&mut self, code: &'static str, message: String) {
        if self.abort.is_none() {
            self.abort = Some(Abort::Limit { code, message });
        }
    }
    pub fn boundary(&mut self, code: &'static str, message: String) {
        if self.abort.is_none() {
            self.abort = Some(Abort::Boundary { code, message });
        }
    }
    #[inline]
    pub fn aborted(&self) -> bool {
        self.abort.is_some()
    }

    /// Record an evaluation diagnostic (bounded).
    pub fn diag(&mut self, code: &'static str, site: u32, message: impl Into<String>) {
        if (self.diags.len() as u32) < self.limits.max_diags {
            self.diags.push(EvalDiag { code, site, message: message.into() });
        } else {
            self.diag_overflow += 1;
        }
    }

    // ------------------------------------------------------------ accessors

    pub fn is_null(&self, h: H) -> bool {
        matches!(self.get(h), Value::Null)
    }
    pub fn as_num(&self, h: H) -> Option<Decimal> {
        match self.get(h) {
            Value::Num(d) => Some(*d),
            _ => None,
        }
    }
    pub fn as_str(&self, h: H) -> Option<Rc<str>> {
        match self.get(h) {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
    pub fn as_bool(&self, h: H) -> Option<bool> {
        match self.get(h) {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_list(&self, h: H) -> Option<&[H]> {
        match self.get(h) {
            Value::List(v) => Some(v),
            _ => None,
        }
    }
    pub fn list_items(&self, h: H) -> Option<Vec<H>> {
        self.as_list(h).map(|s| s.to_vec())
    }
    pub fn as_ctx(&self, h: H) -> Option<&Ctx> {
        match self.get(h) {
            Value::Ctx(c) => Some(c),
            _ => None,
        }
    }
    /// Integer value of a number handle (must be integral and fit i64).
    pub fn as_int(&self, h: H) -> Option<i64> {
        self.as_num(h).and_then(|d| d.to_i64())
    }

    /// Kind name for diagnostics.
    pub fn kind_name(&self, h: H) -> &'static str {
        kind_name(self.get(h))
    }

    // ------------------------------------------------------------ builders

    pub fn list_push(&mut self, l: H, v: H) {
        if self.aborted() {
            return;
        }
        let max = self.limits.max_collection as usize;
        let mut over = false;
        if let Some(Value::List(items)) = self.vals.get_mut(l as usize) {
            if items.len() >= max {
                over = true;
            } else {
                items.push(v);
            }
        }
        if over {
            self.limit("FEEL-L402", format!("collection size limit exceeded ({max} items)"));
        }
    }

    pub fn ctx_put(&mut self, c: H, key: H, v: H) {
        let k = match self.get(key) {
            Value::Str(s) => s.clone(),
            _ => return,
        };
        if let Some(Value::Ctx(ctx)) = self.vals.get_mut(c as usize) {
            ctx.put(k, v);
        }
    }
}

pub fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Date(_) => "date",
        Value::Time(_) => "time",
        Value::DateTime(_) => "date and time",
        Value::Dtd(_) => "days and time duration",
        Value::Ymd(_) => "years and months duration",
        Value::List(_) => "list",
        Value::Ctx(_) => "context",
        Value::Range(_) => "range",
        Value::Func(_) => "function",
    }
}
