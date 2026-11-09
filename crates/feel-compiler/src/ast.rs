//! Source-spanned FEEL abstract syntax tree (DMN 1.5 clause 10.3.1.2).

use crate::diag::Span;
use serde::Serialize;

pub type NodeId = u32;

#[derive(Clone, Debug, Serialize)]
pub struct Expr {
    pub id: NodeId,
    pub span: Span,
    pub kind: ExprKind,
}

/// A FEEL name: a sequence of tokens (10.3.1.4). `text` is the canonical
/// string form (tokens joined as written, with runs of white space collapsed
/// to a single space); `key` is the whitespace-insensitive token key used for
/// matching.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
pub struct Name {
    pub text: String,
    pub tokens: Vec<String>,
}

impl Name {
    pub fn simple(s: &str) -> Name {
        Name { text: s.to_string(), tokens: vec![s.to_string()] }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Quantifier {
    Some,
    Every,
}

#[derive(Clone, Debug, Serialize)]
pub enum ExprKind {
    Null,
    Bool(bool),
    /// Numeric literal text, exactly as written (rule 35).
    Number(String),
    String(String),
    /// `@"..."` (rule 65): the string payload.
    At(String),
    Name(Name),
    /// `e.name` (rule 43).
    Path(Box<Expr>, Name),
    /// `e1[e2]` (rule 50).
    Filter(Box<Expr>, Box<Expr>),
    /// `e(args)` (rule 38).
    Call(Box<Expr>, Args),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Compare(CmpOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Between(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `e in test` / `e in (test, test, ...)` (rule 49 c, d).
    In(Box<Expr>, Vec<UnaryTest>),
    InstanceOf(Box<Expr>, TypeExpr),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    For(Vec<(Name, Span, IterCtx)>, Box<Expr>),
    Quantified(Quantifier, Vec<(Name, Span, Expr)>, Box<Expr>),
    List(Vec<Expr>),
    Context(Vec<ContextEntry>),
    FunctionDef(FunctionDef),
    /// Interval literal `[a..b)` etc. (rule 8) used as a range value.
    Interval(Box<Interval>),
    /// `< e`, `<= e`, `> e`, `>= e` used as a range value (rule 7.a).
    UnaryRange(CmpOp, Box<Expr>),
    /// Synthetic (not FEEL syntax): binding conversion to a declared type
    /// (10.3.2.9.4 "conforms to" conversion), used by the DMN integration
    /// layer for typeRef'd bindings.
    Coerce(Box<Expr>, TypeExpr),
    /// Synthetic: a unary-tests check of `candidate` (decision-table input
    /// entries built by the DMN integration layer).
    Tests(Box<Expr>, Box<UnaryTests>),
}

#[derive(Clone, Debug, Serialize)]
pub struct Interval {
    pub start: Expr,
    pub end: Expr,
    pub start_incl: bool,
    pub end_incl: bool,
}

#[derive(Clone, Debug, Serialize)]
pub enum IterCtx {
    Expr(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextEntry {
    pub key: String,
    pub key_span: Span,
    pub value: Expr,
}

#[derive(Clone, Debug, Serialize)]
pub struct FunctionDef {
    pub params: Vec<Param>,
    pub body: Box<Expr>,
    pub external: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Param {
    pub name: Name,
    pub span: Span,
    pub ty: Option<TypeExpr>,
}

#[derive(Clone, Debug, Serialize)]
pub enum Args {
    Positional(Vec<Expr>),
    Named(Vec<(Name, Span, Expr)>),
}

impl Args {
    pub fn len(&self) -> usize {
        match self {
            Args::Positional(v) => v.len(),
            Args::Named(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Type expressions (rule 52).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub enum TypeExpr {
    Named(String, Span),
    List(Box<TypeExpr>),
    Range(Box<TypeExpr>),
    Context(Vec<(String, TypeExpr)>),
    Function(Vec<TypeExpr>, Box<TypeExpr>),
}

impl std::fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeExpr::Named(n, _) => write!(f, "{n}"),
            TypeExpr::List(t) => write!(f, "list<{t}>"),
            TypeExpr::Range(t) => write!(f, "range<{t}>"),
            TypeExpr::Context(es) => {
                write!(f, "context<")?;
                for (i, (k, t)) in es.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {t}")?;
                }
                write!(f, ">")
            }
            TypeExpr::Function(ps, r) => {
                write!(f, "function<")?;
                for (i, t) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ">->{r}")
            }
        }
    }
}

/// A positive unary test (rule 7, 13).
#[derive(Clone, Debug, Serialize)]
pub enum UnaryTest {
    /// `< e`, `<= e`, `> e`, `>= e`, `= e`, `!= e` (rule 7.a).
    Compare(CmpOp, Expr, Span),
    /// `[a..b]` etc. (rule 7.b / 8).
    Interval(Box<Interval>, Span),
    /// Any other expression (rule 13); `uses_q` records whether it mentions `?`.
    Expr(Expr, bool),
}

impl UnaryTest {
    pub fn span(&self) -> Span {
        match self {
            UnaryTest::Compare(_, _, s) => *s,
            UnaryTest::Interval(_, s) => *s,
            UnaryTest::Expr(e, _) => e.span,
        }
    }
}

/// Unary tests (rule 15).
#[derive(Clone, Debug, Serialize)]
pub enum UnaryTests {
    Positive(Vec<UnaryTest>),
    Negated(Vec<UnaryTest>),
    /// `-`
    Any,
}

/// A parsed compilation unit.
#[derive(Clone, Debug, Serialize)]
pub enum Program {
    Expression(Expr),
    UnaryTests(UnaryTests, Span),
}

impl Expr {
    /// Visit direct children.
    pub fn children(&self) -> Vec<&Expr> {
        let mut v: Vec<&Expr> = Vec::new();
        match &self.kind {
            ExprKind::Null | ExprKind::Bool(_) | ExprKind::Number(_) | ExprKind::String(_) | ExprKind::At(_) | ExprKind::Name(_) => {}
            ExprKind::Path(e, _) | ExprKind::Neg(e) | ExprKind::InstanceOf(e, _) | ExprKind::UnaryRange(_, e) | ExprKind::Coerce(e, _) => v.push(e),
            ExprKind::Tests(e, ts) => {
                v.push(e);
                if let UnaryTests::Positive(ts) | UnaryTests::Negated(ts) = &**ts {
                    for t in ts {
                        t.collect_exprs(&mut v);
                    }
                }
            }
            ExprKind::Filter(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Compare(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                v.push(a);
                v.push(b)
            }
            ExprKind::Call(f, args) => {
                v.push(f);
                match args {
                    Args::Positional(a) => v.extend(a.iter()),
                    Args::Named(a) => v.extend(a.iter().map(|x| &x.2)),
                }
            }
            ExprKind::Between(a, b, c) | ExprKind::If(a, b, c) => {
                v.push(a);
                v.push(b);
                v.push(c)
            }
            ExprKind::In(e, tests) => {
                v.push(e);
                for t in tests {
                    t.collect_exprs(&mut v);
                }
            }
            ExprKind::For(its, body) => {
                for (_, _, ic) in its {
                    match ic {
                        IterCtx::Expr(e) => v.push(e),
                        IterCtx::Range(a, b) => {
                            v.push(a);
                            v.push(b)
                        }
                    }
                }
                v.push(body)
            }
            ExprKind::Quantified(_, its, body) => {
                for (_, _, e) in its {
                    v.push(e)
                }
                v.push(body)
            }
            ExprKind::List(es) => v.extend(es.iter()),
            ExprKind::Context(es) => v.extend(es.iter().map(|e| &e.value)),
            ExprKind::FunctionDef(fd) => v.push(&fd.body),
            ExprKind::Interval(iv) => {
                v.push(&iv.start);
                v.push(&iv.end)
            }
        }
        v
    }

    /// Does this expression (lexically) mention the special name `?`?
    pub fn mentions_q(&self) -> bool {
        if let ExprKind::Name(n) = &self.kind {
            if n.tokens.len() == 1 && n.tokens[0] == "?" {
                return true;
            }
        }
        self.children().iter().any(|c| c.mentions_q())
    }
}

impl UnaryTest {
    pub fn collect_exprs<'a>(&'a self, v: &mut Vec<&'a Expr>) {
        match self {
            UnaryTest::Compare(_, e, _) => v.push(e),
            UnaryTest::Interval(iv, _) => {
                v.push(&iv.start);
                v.push(&iv.end)
            }
            UnaryTest::Expr(e, _) => v.push(e),
        }
    }
}
