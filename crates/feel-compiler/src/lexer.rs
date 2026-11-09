//! FEEL tokenizer (DMN 1.5 clause 10.3.1.2 rules 25-37, 61-65 and 10.3.1.4).
//!
//! The lexer produces *tokens*; it does not decide which tokens form a name.
//! Names in FEEL are sequences of tokens (10.3.1.4) and are assembled by the
//! parser using the names in scope (10.3.1.6).

use crate::diag::{Diagnostic, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A maximal run of name characters (rules 26-29), e.g. `Income`, `if?`, `?`.
    Word(String),
    /// Numeric literal text (rule 35 without sign), e.g. `1`, `1.5`, `.5`, `1e3`.
    Number(String),
    /// A decoded string literal (rule 33, 64).
    Str(String),
    Sym(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Whether whitespace or a comment preceded this token.
    pub ws_before: bool,
}

impl Token {
    pub fn is_sym(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Sym(x) if *x == s)
    }
    pub fn is_word(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Word(x) if x == s)
    }
    /// Source text of this token as it participates in a name.
    pub fn name_text(&self) -> String {
        match &self.tok {
            Tok::Word(w) => w.clone(),
            Tok::Number(n) => n.clone(),
            Tok::Sym(s) => s.to_string(),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Eof => String::new(),
        }
    }
}

/// Grammar rule 61 white space (vertical space is rule 62).
pub fn is_feel_whitespace(c: char) -> bool {
    matches!(c,
        '\u{0009}' | '\u{0020}' | '\u{0085}' | '\u{00A0}' | '\u{1680}' | '\u{180E}' |
        '\u{2000}'..='\u{200B}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' |
        '\u{3000}' | '\u{FEFF}' | '\u{000A}'..='\u{000D}')
}

/// Grammar rule 28 (name start char).
pub fn is_name_start_char(c: char) -> bool {
    matches!(c,
        '?' | 'A'..='Z' | '_' | 'a'..='z' |
        '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' |
        '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' |
        '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' |
        '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// Grammar rule 29 (name part char).
pub fn is_name_part_char(c: char) -> bool {
    is_name_start_char(c)
        || c.is_ascii_digit()
        || c == '\u{B7}'
        || ('\u{0300}'..='\u{036F}').contains(&c)
        || ('\u{203F}'..='\u{2040}').contains(&c)
}

const SYMS: &[&str] = &[
    "..", "**", "!=", "<=", ">=", "->", ".", "*", "+", "-", "/", "=", "<", ">", "(", ")", "[",
    "]", "{", "}", ",", ":", "@", "'", "\u{2019}", "!", "#", "$", "%", "&", ";", "\\", "^",
    "|", "~", "`",
];

pub fn tokenize(src: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut ws = false;
    while i < src.len() {
        let c = src[i..].chars().next().unwrap();
        if is_feel_whitespace(c) {
            i += c.len_utf8();
            ws = true;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < src.len() && !src[i..].starts_with('\n') {
                i += src[i..].chars().next().unwrap().len_utf8();
            }
            ws = true;
            continue;
        }
        if src[i..].starts_with("/*") {
            let start = i;
            match src[i + 2..].find("*/") {
                Some(e) => i = i + 2 + e + 2,
                None => {
                    return Err(Diagnostic::error(
                        "FEEL-S003",
                        "unterminated block comment",
                        Span::new(start, src.len()),
                    ))
                }
            }
            ws = true;
            continue;
        }
        let start = i;
        if c == '"' {
            let (s, end) = lex_string(src, i)?;
            out.push(Token { tok: Tok::Str(s), span: Span::new(start, end), ws_before: ws });
            i = end;
            ws = false;
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.' && i + 1 < src.len() && bytes[i + 1].is_ascii_digit() && !prev_allows_path(&out, ws))
        {
            let end = lex_number(src, i);
            out.push(Token {
                tok: Tok::Number(src[i..end].to_string()),
                span: Span::new(start, end),
                ws_before: ws,
            });
            i = end;
            ws = false;
            continue;
        }
        if is_name_start_char(c) {
            let mut j = i + c.len_utf8();
            while j < src.len() {
                let d = src[j..].chars().next().unwrap();
                if is_name_part_char(d) {
                    j += d.len_utf8();
                } else {
                    break;
                }
            }
            out.push(Token { tok: Tok::Word(src[i..j].to_string()), span: Span::new(i, j), ws_before: ws });
            i = j;
            ws = false;
            continue;
        }
        let mut matched = false;
        for s in SYMS {
            if src[i..].starts_with(s) {
                out.push(Token { tok: Tok::Sym(s), span: Span::new(i, i + s.len()), ws_before: ws });
                i += s.len();
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(Diagnostic::error(
                "FEEL-S001",
                format!("unexpected character {:?}", c),
                Span::new(i, i + c.len_utf8()),
            ));
        }
        ws = false;
    }
    out.push(Token { tok: Tok::Eof, span: Span::new(src.len(), src.len()), ws_before: ws });
    Ok(out)
}

/// `.5` is a number unless it directly follows something that can be the
/// subject of a path expression (e.g. `x.5` is not meaningful anyway, but
/// `[1..5]` must lex as `1`, `..`, `5`, which is handled because `..` is
/// matched as a symbol before a number starting with `.` could be).
fn prev_allows_path(out: &[Token], ws: bool) -> bool {
    if ws {
        return false;
    }
    match out.last() {
        Some(t) => matches!(t.tok, Tok::Word(_) | Tok::Str(_)) || t.is_sym(")") || t.is_sym("]") || t.is_sym("}"),
        None => false,
    }
}

fn lex_number(src: &str, start: usize) -> usize {
    let b = src.as_bytes();
    let mut i = start;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            // exponent must not run into a name (e.g. `1each`)
            if j >= b.len() || !src[j..].chars().next().map(is_name_part_char).unwrap_or(false) {
                i = j;
            }
        }
    }
    i
}

fn hex_val(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

fn lex_string(src: &str, start: usize) -> Result<(String, usize), Diagnostic> {
    let mut out = String::new();
    let mut i = start + 1;
    let mut pending_high: Option<u32> = None;
    loop {
        if i >= src.len() {
            return Err(Diagnostic::error("FEEL-S002", "unterminated string literal", Span::new(start, src.len())));
        }
        let c = src[i..].chars().next().unwrap();
        if c == '"' {
            if pending_high.is_some() {
                out.push('\u{FFFD}');
            }
            return Ok((out, i + 1));
        }
        if c == '\\' {
            let n = src[i + 1..].chars().next();
            let esc_start = i;
            match n {
                Some('\'') => { out.push('\''); i += 2; }
                Some('"') => { out.push('"'); i += 2; }
                Some('\\') => { out.push('\\'); i += 2; }
                Some('n') => { out.push('\n'); i += 2; }
                Some('r') => { out.push('\r'); i += 2; }
                Some('t') => { out.push('\t'); i += 2; }
                Some('u') | Some('U') => {
                    // \uXXXX (4 hex digits) or \UXXXXXX (6 hex digits)
                    let len = if n == Some('u') { 4 } else { 6 };
                    let hs = src.get(i + 2..i + 2 + len).unwrap_or("");
                    let cp = if hs.len() == len && hs.chars().all(|c| c.is_ascii_hexdigit()) {
                        hex_val(hs)
                    } else {
                        None
                    };
                    let Some(cp) = cp else {
                        return Err(Diagnostic::error(
                            "FEEL-S004",
                            "invalid unicode escape in string literal",
                            Span::new(esc_start, (i + 2 + len).min(src.len())),
                        ));
                    };
                    i += 2 + len;
                    if (0xD800..0xDC00).contains(&cp) {
                        if pending_high.is_some() {
                            out.push('\u{FFFD}');
                        }
                        pending_high = Some(cp);
                        continue;
                    } else if (0xDC00..0xE000).contains(&cp) {
                        if let Some(h) = pending_high.take() {
                            let full = 0x10000 + ((h - 0xD800) << 10) + (cp - 0xDC00);
                            out.push(char::from_u32(full).unwrap_or('\u{FFFD}'));
                        } else {
                            out.push('\u{FFFD}');
                        }
                        continue;
                    }
                    match char::from_u32(cp) {
                        Some(ch) => out.push(ch),
                        None => {
                            return Err(Diagnostic::error(
                                "FEEL-S004",
                                "invalid code point in string literal",
                                Span::new(esc_start, i),
                            ))
                        }
                    }
                }
                _ => {
                    return Err(Diagnostic::error(
                        "FEEL-S005",
                        "invalid escape sequence in string literal",
                        Span::new(i, (i + 2).min(src.len())),
                    ))
                }
            }
            if pending_high.take().is_some() {
                // a lone high surrogate followed by a non-surrogate
                let last = out.pop();
                out.push('\u{FFFD}');
                if let Some(l) = last {
                    out.push(l);
                }
            }
            continue;
        }
        if pending_high.take().is_some() {
            out.push('\u{FFFD}');
        }
        out.push(c);
        i += c.len_utf8();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn toks(s: &str) -> Vec<Tok> {
        tokenize(s).unwrap().into_iter().map(|t| t.tok).collect()
    }
    #[test]
    fn basic() {
        assert_eq!(
            toks("a.b + 1..2"),
            vec![
                Tok::Word("a".into()),
                Tok::Sym("."),
                Tok::Word("b".into()),
                Tok::Sym("+"),
                Tok::Number("1".into()),
                Tok::Sym(".."),
                Tok::Number("2".into()),
                Tok::Eof
            ]
        );
        assert_eq!(toks(".5"), vec![Tok::Number(".5".into()), Tok::Eof]);
        assert_eq!(toks("1.2e3"), vec![Tok::Number("1.2e3".into()), Tok::Eof]);
        assert_eq!(toks("\"a\\u0041\\U01F40E\""), vec![Tok::Str("aA\u{1F40E}".into()), Tok::Eof]);
        assert_eq!(toks("\"\\ud83d\\udc0e\""), vec![Tok::Str("\u{1F40E}".into()), Tok::Eof]);
        assert_eq!(toks("what if? // c\n /* x */ y"), vec![Tok::Word("what".into()), Tok::Word("if?".into()), Tok::Word("y".into()), Tok::Eof]);
    }
}
