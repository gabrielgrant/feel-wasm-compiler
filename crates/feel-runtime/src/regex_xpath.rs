//! XPath/XQuery F&O regular expressions (clause 7.6 of XQuery 1.0 and XPath
//! 2.0 Functions and Operators) mapped onto `fancy-regex`.
//!
//! Supported flags: `s` (dot matches newline), `m` (multi-line anchors),
//! `i` (case-insensitive), `x` (strip unescaped white space from the
//! pattern), `q` (pattern is a literal string). Unknown flags or invalid
//! patterns yield `None` (FEEL null), as XPath errors map to null in FEEL.

use fancy_regex::Regex;
use std::rc::Rc;

pub fn compile(pattern: &str, flags: &str) -> Option<Regex> {
    let mut s = false;
    let mut m = false;
    let mut i = false;
    let mut x = false;
    let mut q = false;
    for c in flags.chars() {
        match c {
            's' => s = true,
            'm' => m = true,
            'i' => i = true,
            'x' => x = true,
            'q' => q = true,
            _ => return None,
        }
    }
    let mut pat = String::new();
    if q {
        pat = fancy_regex::escape(pattern).into_owned();
    } else {
        let src: String = if x {
            // remove white space not inside character classes
            let mut out = String::new();
            let mut in_class = false;
            let mut esc = false;
            for ch in pattern.chars() {
                // XPath: with the x flag all white space is removed, even
                // after a backslash (so "\ s" means "\s")
                if ch.is_whitespace() && !in_class {
                    continue;
                }
                if esc {
                    out.push(ch);
                    esc = false;
                    continue;
                }
                match ch {
                    '\\' => {
                        esc = true;
                        out.push(ch)
                    }
                    '[' => {
                        in_class = true;
                        out.push(ch)
                    }
                    ']' => {
                        in_class = false;
                        out.push(ch)
                    }
                    c if c.is_whitespace() && !in_class => {}
                    c => out.push(c),
                }
            }
            out
        } else {
            pattern.to_string()
        };
        translate(&src, &mut pat, s)?;
    }
    let mut prefix = String::new();
    if s || i || m {
        prefix.push_str("(?");
        if s {
            prefix.push('s');
        }
        if i {
            prefix.push('i');
        }
        if m {
            prefix.push('m');
        }
        prefix.push(')');
    }
    Regex::new(&format!("{prefix}{pat}")).ok()
}

/// Translate XPath regex syntax into fancy-regex syntax.
/// Unicode block names used by XPath `\p{IsBlock}` (not supported by the
/// regex engine), mapped to code point ranges.
fn block_range(name: &str) -> Option<&'static str> {
    Some(match name {
        "BasicLatin" => "\\x{0}-\\x{7F}",
        "Latin-1Supplement" => "\\x{80}-\\x{FF}",
        "LatinExtended-A" => "\\x{100}-\\x{17F}",
        "LatinExtended-B" => "\\x{180}-\\x{24F}",
        "IPAExtensions" => "\\x{250}-\\x{2AF}",
        "Greek" | "GreekandCoptic" => "\\x{370}-\\x{3FF}",
        "Cyrillic" => "\\x{400}-\\x{4FF}",
        "Hebrew" => "\\x{590}-\\x{5FF}",
        "Arabic" => "\\x{600}-\\x{6FF}",
        "GeneralPunctuation" => "\\x{2000}-\\x{206F}",
        "CJKUnifiedIdeographs" => "\\x{4E00}-\\x{9FFF}",
        _ => return None,
    })
}

fn translate(src: &str, out: &mut String, dot_all: bool) -> Option<()> {
    let chars: Vec<char> = src.chars().collect();
    let mut k = 0;
    let mut depth_class = 0;
    while k < chars.len() {
        let c = chars[k];
        match c {
            '\\' => {
                let n = *chars.get(k + 1)?;
                match n {
                    // XPath multi-char escapes \i \c (XML name chars) approximated
                    'i' => out.push_str(if depth_class > 0 { "_:A-Za-z" } else { "[_:A-Za-z]" }),
                    'I' => out.push_str("[^_:A-Za-z]"),
                    'c' => out.push_str(if depth_class > 0 { "-._:A-Za-z0-9" } else { "[-._:A-Za-z0-9]" }),
                    'C' => out.push_str("[^-._:A-Za-z0-9]"),
                    'p' | 'P' => {
                        // \p{IsBlock} → Unicode block names are not supported by
                        // fancy-regex; pass general categories through
                        out.push('\\');
                        out.push(n);
                        let mut j = k + 2;
                        if chars.get(j) == Some(&'{') {
                            let close = chars[j..].iter().position(|&c| c == '}')? + j;
                            let name: String = chars[j + 1..close].iter().collect();
                            j = close + 1;
                            if let Some(b) = name.strip_prefix("Is") {
                                let r = block_range(b)?;
                                out.pop();
                                out.pop();
                                if depth_class > 0 {
                                    if n == 'P' {
                                        return None;
                                    }
                                    out.push_str(r);
                                } else {
                                    out.push_str(if n == 'P' { "[^" } else { "[" });
                                    out.push_str(r);
                                    out.push(']');
                                }
                                k = j;
                                continue;
                            }
                            out.push('{');
                            out.push_str(&name);
                            out.push('}');
                        }
                        k = j;
                        continue;
                    }
                    _ => {
                        out.push('\\');
                        out.push(n);
                    }
                }
                k += 2;
                continue;
            }
            '[' => {
                depth_class += 1;
                out.push('[');
                // XPath character class subtraction `[a-z-[aeiou]]`
                k += 1;
                continue;
            }
            ']' => {
                if depth_class > 0 {
                    depth_class -= 1;
                }
                out.push(']');
            }
            '-' if depth_class > 0 && chars.get(k + 1) == Some(&'[') => {
                // subtraction: emit as intersection with a negated class
                out.push_str("&&[^");
                k += 2;
                depth_class += 1;
                continue;
            }
            // XPath '.' matches any character except \n and \r
            '.' if depth_class == 0 && !dot_all => out.push_str("[^\\n\\r]"),
            c => out.push(c),
        }
        k += 1;
    }
    Some(())
}

/// Expand an XPath replacement string (`$n`, `\$`, `\\`) for one match.
pub fn expand_replacement(rep: &str, caps: &fancy_regex::Captures<'_, str>, ngroups: usize) -> Option<String> {
    let mut out = String::new();
    let b: Vec<char> = rep.chars().collect();
    let mut k = 0;
    while k < b.len() {
        match b[k] {
            '\\' => {
                let n = *b.get(k + 1)?;
                if n == '$' || n == '\\' {
                    out.push(n);
                    k += 2;
                } else {
                    return None;
                }
            }
            '$' => {
                let mut j = k + 1;
                if j >= b.len() || !b[j].is_ascii_digit() {
                    return None;
                }
                // longest group number that exists (XPath rule)
                let mut num: usize = (b[j] as u8 - b'0') as usize;
                j += 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    let next = num * 10 + (b[j] as u8 - b'0') as usize;
                    if next > ngroups {
                        break;
                    }
                    num = next;
                    j += 1;
                }
                if num <= ngroups {
                    if let Some(m) = caps.get(num) {
                        out.push_str(m.as_str());
                    }
                }
                k = j;
            }
            c => {
                out.push(c);
                k += 1;
            }
        }
    }
    Some(out)
}

pub type CachedRegex = Rc<Regex>;
