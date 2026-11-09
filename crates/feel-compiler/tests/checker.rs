//! Checker (CEL-style parse → check) and dependency-extraction tests.

use feel_compiler::check::CheckMode;
use feel_compiler::{check, compile, CompileOptions, Entry};

fn opts(schema: &str, mode: CheckMode) -> CompileOptions {
    let mut o = CompileOptions::from_json(&format!(r#"{{"schema": {schema}}}"#)).unwrap();
    o.check = mode;
    o
}

const SCHEMA: &str = r#"{
  "inputs": {
    "applicant": "context<name: string, age: number, email?: string, spouse: context<name: string>?>",
    "loans": "list<context<amount: number, rate: number, kind: string>>",
    "score": "number?",
    "rate": "number"
  },
  "functions": [{"name": "risk band", "params": [{"name": "score", "type": "number"}], "returns": "string"}],
  "closed": true
}"#;

fn codes(src: &str, mode: CheckMode) -> Vec<String> {
    check(src, &opts(SCHEMA, mode)).diagnostics.iter().map(|d| d.code.clone()).collect()
}

#[test]
fn infers_result_types() {
    let o = opts(SCHEMA, CheckMode::Strict);
    let t = |s: &str| check(s, &o).result_type.unwrap();
    assert_eq!(t("applicant.age + 1"), "number");
    assert_eq!(t("loans[amount > 1000].rate"), "list<number>");
    assert_eq!(t("for l in loans return l.amount * l.rate"), "list<number>");
    assert_eq!(t("applicant.age >= 18 and rate < 0.1"), "boolean?");
    assert_eq!(t("risk band(700)"), "string");
    assert_eq!(t("{a: 1, b: \"x\"}"), "context<a: number, b: string>");
    assert_eq!(t("applicant.email"), "string?");
    assert_eq!(t("@\"2020-01-01\" + @\"P1D\""), "date");
    assert_eq!(t("date(\"2020-01-01\") + @\"P1D\""), "date?");
}

#[test]
fn reports_stable_codes_spans_and_suggestions() {
    let o = opts(SCHEMA, CheckMode::Strict);
    let c = check("applicant.agee + 1", &o);
    assert!(!c.ok);
    let d = &c.diagnostics[0];
    assert_eq!(d.code, "FEEL-C102");
    assert_eq!((d.span.start, d.span.end), (0, 14));
    assert_eq!(d.suggestion.as_deref(), Some("did you mean 'age'?"));
    let c = check("aplicant.age", &o);
    assert_eq!(c.diagnostics[0].code, "FEEL-C101");
    assert_eq!(c.diagnostics[0].suggestion.as_deref(), Some("did you mean 'applicant'?"));
    assert!(codes("applicant.name * 2", CheckMode::Strict).contains(&"FEEL-C103".to_string()));
    assert!(codes("applicant.name < 3", CheckMode::Strict).contains(&"FEEL-C107".to_string()));
    assert!(codes("substring(\"abc\")", CheckMode::Strict).contains(&"FEEL-C104".to_string()));
    assert!(codes("substring(\"abc\", \"x\")", CheckMode::Strict).contains(&"FEEL-C105".to_string()));
    assert!(codes("substring(strin: \"abc\", start position: 1)", CheckMode::Strict).contains(&"FEEL-C104".to_string()));
    assert!(codes("if rate then 1 else 2", CheckMode::Strict).contains(&"FEEL-C106".to_string()));
    assert!(codes("rate(1)", CheckMode::Strict).contains(&"FEEL-C110".to_string()));
    assert!(codes("1 instance of tUnknown", CheckMode::Strict).contains(&"FEEL-C111".to_string()));
    let p = check("-2 ** 2", &o);
    assert_eq!(p.diagnostics[0].code, "FEEL-W201");
}

#[test]
fn strict_vs_permissive_policies() {
    // the same finding is an error under strict and a warning under permissive
    let s = check("applicant.name * 2", &opts(SCHEMA, CheckMode::Strict));
    let p = check("applicant.name * 2", &opts(SCHEMA, CheckMode::Permissive));
    assert!(!s.ok && p.ok);
    // permissive compilation proceeds and keeps FEEL semantics (null)
    let out = compile("applicant.name * 2", &opts(SCHEMA, CheckMode::Permissive));
    assert!(out.wasm.is_some());
    let out = compile("applicant.name * 2", &opts(SCHEMA, CheckMode::Strict));
    assert!(out.wasm.is_none());
    // checking disabled: no findings at all
    assert!(compile("applicant.name * 2", &opts(SCHEMA, CheckMode::Off)).diagnostics.is_empty());
}

#[test]
fn nullable_narrowing_through_guards() {
    let o = opts(SCHEMA, CheckMode::Strict);
    let w = |s: &str| check(s, &o).diagnostics.iter().filter(|d| d.code == "FEEL-W210").count();
    assert_eq!(w("score + 1"), 1, "score is nullable");
    assert_eq!(w("if score != null then score + 1 else 0"), 0);
    assert_eq!(w("score != null and score > 600"), 0);
    assert_eq!(w("if score = null then 0 else score * 2"), 0);
    assert_eq!(w("if applicant.spouse != null then applicant.spouse.name else \"\""), 0);
    assert_eq!(check("if score != null then score + 1 else 0", &o).result_type.as_deref(), Some("number"));
    let d = check("score + 1", &o).diagnostics.into_iter().find(|d| d.code == "FEEL-W210").unwrap();
    assert_eq!(d.suggestion.as_deref(), Some("guard with `score != null`"));
}

#[test]
fn optional_is_distinct_from_nullable() {
    let o = opts(SCHEMA, CheckMode::Strict);
    // optional field: selecting is allowed and yields a nullable type
    assert_eq!(check("applicant.email", &o).result_type.as_deref(), Some("string?"));
    // a non-optional field is non-null
    assert_eq!(check("applicant.name", &o).result_type.as_deref(), Some("string"));
}

#[test]
fn expected_type_and_candidate_type() {
    let mut o = opts(SCHEMA, CheckMode::Strict);
    o.schema.expected = Some("boolean".into());
    assert!(check("applicant.age >= 18", &o).ok);
    let c = check("applicant.age + 1", &o);
    assert_eq!(c.diagnostics[0].code, "FEEL-C108");
    let mut u = opts(r#"{"candidate": "number"}"#, CheckMode::Strict);
    u.entry = Entry::UnaryTests;
    assert!(check("[18..65), > 100", &u).ok);
    let c = check("\"gold\", \"silver\"", &u);
    assert_eq!(c.diagnostics[0].code, "FEEL-C109");
    assert!(check("? * 2 > 10", &u).ok);
}

#[test]
fn checker_never_changes_values() {
    // strict and permissive compiles of the same valid program produce identical modules
    let a = compile("for l in loans return l.amount * l.rate", &opts(SCHEMA, CheckMode::Strict));
    let b = compile("for l in loans return l.amount * l.rate", &opts(SCHEMA, CheckMode::Off));
    let strip = |w: &Vec<u8>| feel_host_free_code(w);
    assert_eq!(strip(a.wasm.as_ref().unwrap()), strip(b.wasm.as_ref().unwrap()));
}

/// The code section of a module (ignoring custom sections).
fn feel_host_free_code(w: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in wasmparser::Parser::new(0).parse_all(w) {
        if let Ok(wasmparser::Payload::CodeSectionEntry(body)) = p {
            let r = body.range();
            out.extend_from_slice(&w[r.start as usize..r.end as usize]);
        }
    }
    out
}

#[test]
fn dependency_extraction() {
    let o = opts(SCHEMA, CheckMode::Permissive);
    let d = compile("applicant.age >= 18 and applicant.name != \"\" and sum(loans.amount) < 1000", &o).metadata.unwrap().dependencies;
    let a = d.inputs.iter().find(|i| i.name == "applicant").unwrap();
    assert!(!a.whole);
    assert_eq!(a.paths, vec![vec!["age".to_string()], vec!["name".to_string()]]);
    let l = d.inputs.iter().find(|i| i.name == "loans").unwrap();
    assert_eq!(l.paths, vec![vec!["amount".to_string()]]);
    assert!(d.exact && !d.clock);
    // dynamic lookups and clock access are reported
    let d = compile("get value(applicant, k) + today()", &CompileOptions::default()).metadata.unwrap().dependencies;
    assert!(d.clock && !d.exact);
    assert!(d.uncertain.iter().any(|u| u.contains("computed key")));
    let d = compile("loans[amount > rate]", &opts(SCHEMA, CheckMode::Permissive)).metadata.unwrap().dependencies;
    assert!(!d.exact, "implicit filter names may shadow inputs");
    let d = compile("f(1)", &CompileOptions::default()).metadata.unwrap().dependencies;
    assert!(d.uncertain.iter().any(|u| u.contains("function value")));
}

#[test]
fn cache_keys() {
    let o = CompileOptions::default();
    let k1 = o.cache_key("a + b");
    assert_eq!(k1, o.cache_key("a + b"));
    assert_ne!(k1, o.cache_key("a + c"));
    let mut o2 = o.clone();
    o2.optimize = false;
    assert_ne!(k1, o2.cache_key("a + b"));
    let o3 = opts(SCHEMA, CheckMode::Permissive);
    assert_ne!(k1, o3.cache_key("a + b"));
    assert_eq!(compile("a + b", &o).metadata.unwrap().cache_key, k1);
}
