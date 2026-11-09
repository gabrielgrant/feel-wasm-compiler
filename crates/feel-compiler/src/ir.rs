//! The typed intermediate representation lowered to WebAssembly.
//!
//! Every IR node produces either a *value handle* or a *tri-state boolean*
//! (0 false, 1 true, 2 null); [`Ir::is_tri`] tells which. Names are resolved:
//! locals, input slots, captured variables, builtins, host functions and the
//! dynamic item scope of filters are all explicit.

use crate::ast::{BinOp, CmpOp};

pub type Local = u32;
pub type ConstId = u32;
pub type Site = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum Ir {
    Null,
    True,
    False,
    Const(ConstId),
    Local(Local),
    Slot(u32),
    /// An implicit name inside a filter: the entry `key` of the current item
    /// if it is a context with that entry, otherwise `fallback`.
    ItemField {
        item: Local,
        key: ConstId,
        fallback: Box<Ir>,
    },
    Path(Box<Ir>, ConstId, Site),
    Bin(BinOp, Box<Ir>, Box<Ir>, Site),
    Neg(Box<Ir>, Site),
    /// tri
    Cmp(CmpOp, Box<Ir>, Box<Ir>, Site),
    /// tri: `e = null` / `e != null` (Table 49.a), `negate` for `!=`
    IsNull(Box<Ir>, bool),
    /// tri
    And(Box<Ir>, Box<Ir>),
    /// tri
    Or(Box<Ir>, Box<Ir>),
    /// tri: builtin `not(x)`
    Not(Box<Ir>, Site),
    If(Box<Ir>, Box<Ir>, Box<Ir>),
    Let(Local, Box<Ir>, Box<Ir>),
    List(Vec<Ir>),
    /// entries are evaluated in order into their locals, then the context is built
    Context(Vec<(ConstId, Local, Ir)>),
    Range {
        start: Option<Box<Ir>>,
        end: Option<Box<Ir>>,
        start_incl: bool,
        end_incl: bool,
        site: Site,
    },
    /// `= e` / `!= e` used as a range value
    PointRange {
        value: Box<Ir>,
        ne: bool,
        site: Site,
    },
    /// tri: `cand in (tests)` — `cand` is a local holding the candidate
    In {
        cand: Local,
        tests: Vec<TestIr>,
        negated: bool,
    },
    /// tri
    InstanceOf(Box<Ir>, ConstId),
    Filter {
        list: Box<Ir>,
        item: Local,
        pred: Box<Ir>,
        site: Site,
    },
    /// Fused `list[pred].key` (optimization; see `opt.rs`).
    FilterPath {
        list: Box<Ir>,
        item: Local,
        pred: Box<Ir>,
        key: ConstId,
        site: Site,
    },
    Index(Box<Ir>, Box<Ir>, Site),
    For {
        iters: Vec<(Local, IterIr)>,
        partial: Local,
        body: Box<Ir>,
        site: Site,
    },
    /// tri
    Quant {
        every: bool,
        iters: Vec<(Local, Ir)>,
        body: Box<Ir>,
        site: Site,
    },
    CallBuiltin {
        id: u16,
        args: Vec<Ir>,
        site: Site,
    },
    CallBuiltinNamed {
        id: u16,
        args: Vec<(ConstId, Ir)>,
        site: Site,
    },
    CallValue {
        f: Box<Ir>,
        args: Vec<Ir>,
        site: Site,
    },
    CallValueNamed {
        f: Box<Ir>,
        args: Vec<(ConstId, Ir)>,
        site: Site,
    },
    Closure {
        func: u32,
        captures: Vec<Ir>,
        sig: ConstId,
        self_local: Option<Local>,
    },
    BuiltinFn(u16),
    HostFn {
        name: ConstId,
        sig: ConstId,
    },
    ExternalFn {
        mapping: Box<Ir>,
        sig: ConstId,
    },
    Coerce(Box<Ir>, ConstId, Site),
    /// A snapshot of the `partial` list of the enclosing `for`.
    Partial(Local),
}

#[derive(Clone, Debug, PartialEq)]
pub enum IterIr {
    Expr(Ir),
    Range(Ir, Ir),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TestIr {
    Cmp(CmpOp, Ir, Site),
    Interval {
        start: Ir,
        end: Ir,
        start_incl: bool,
        end_incl: bool,
        site: Site,
    },
    /// a general positive unary test expression evaluated with `?` bound
    Value {
        value: Ir,
        uses_q: bool,
        site: Site,
    },
}

impl Ir {
    pub fn is_tri(&self) -> bool {
        matches!(
            self,
            Ir::Cmp(..)
                | Ir::IsNull(..)
                | Ir::And(..)
                | Ir::Or(..)
                | Ir::Not(..)
                | Ir::In { .. }
                | Ir::InstanceOf(..)
                | Ir::Quant { .. }
        )
    }
}

/// A user function body compiled to its own Wasm function with signature
/// `(env: i32, args: i32) -> i32`.
#[derive(Clone, Debug)]
pub struct FuncIr {
    pub name: String,
    pub params: Vec<Local>,
    /// locals that receive captured values (in env order)
    pub captures: Vec<Local>,
    pub body: Ir,
    pub nlocals: u32,
}

#[derive(Clone, Debug)]
pub struct ProgramIr {
    pub main: Ir,
    pub main_locals: u32,
    pub funcs: Vec<FuncIr>,
}
