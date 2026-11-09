//! Diagnostics with stable codes and source spans.
//!
//! Code ranges:
//! * `FEEL-S0xx` – syntax errors (parser / lexer); compilation fails.
//! * `FEEL-C1xx` – checker findings (errors under the strict policy, warnings
//!   under the permissive policy).
//! * `FEEL-W2xx` – checker warnings/lints (never errors).
//! * `FEEL-E3xx` – evaluation diagnostics (produced at runtime alongside a
//!   FEEL `null` result; see `feel_runtime::diag`).
//! * `FEEL-L4xx` – resource-limit failures (evaluation aborted; *not* null).
//! * `FEEL-B5xx` – runtime-boundary / ABI errors (bad inputs, incompatible
//!   module, schema violations at the boundary).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    /// 1-based line/column of `span.start` (filled in by [`Diagnostic::locate`]).
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub suggestion: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, msg: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic {
            code: code.to_string(),
            severity: Severity::Error,
            message: msg.into(),
            span,
            line: 0,
            column: 0,
            suggestion: None,
        }
    }
    pub fn warning(code: &str, msg: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, msg, span)
        }
    }
    pub fn with_suggestion(mut self, s: impl Into<String>) -> Diagnostic {
        self.suggestion = Some(s.into());
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
    /// Fill in line/column information from the source text.
    pub fn locate(&mut self, src: &str) {
        let (l, c) = line_col(src, self.span.start);
        self.line = l;
        self.column = c;
    }
}

pub fn line_col(src: &str, offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(
            f,
            "{sev}[{}] {}:{}: {}",
            self.code, self.line, self.column, self.message
        )?;
        if let Some(s) = &self.suggestion {
            write!(f, " (suggestion: {s})")?;
        }
        Ok(())
    }
}
