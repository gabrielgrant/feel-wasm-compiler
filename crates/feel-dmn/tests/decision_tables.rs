//! Decision-table integration: input expressions produce candidate values,
//! unary-test modules consume them, output expressions produce results; the
//! hit policy is applied by the DMN layer. Each table is evaluated two ways —
//! host-orchestrated (one module per cell) and lowered (one module per table)
//! — and both must agree with the expected results.

use feel_dmn::host_dt::HostDecisionTable;
use feel_dmn::lower::{Lowerer, ModelSet};
use feel_dmn::model::{
    Boxed, Decision, DecisionTable, Definitions, DtInput, DtOutput, DtRule, InputData,
};
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};

fn table(
    hit: &str,
    agg: Option<&str>,
    inputs: &[&str],
    outputs: &[(&str, Option<&str>)],
    rules: &[(&[&str], &[&str])],
) -> DecisionTable {
    DecisionTable {
        hit_policy: hit.into(),
        aggregation: agg.map(|s| s.into()),
        inputs: inputs
            .iter()
            .map(|e| DtInput {
                expr: e.to_string(),
                type_ref: None,
                input_values: None,
            })
            .collect(),
        outputs: outputs
            .iter()
            .map(|(n, v)| DtOutput {
                name: Some(n.to_string()),
                type_ref: None,
                output_values: v.map(|s| s.to_string()),
                default: None,
            })
            .collect(),
        rules: rules
            .iter()
            .map(|(i, o)| DtRule {
                inputs: i.iter().map(|s| s.to_string()).collect(),
                outputs: o.iter().map(|s| s.to_string()).collect(),
            })
            .collect(),
    }
}

/// Evaluate with the lowered (single-module) path.
fn lowered(dt: &DecisionTable, input_names: &[&str], inputs: &str) -> String {
    let defs = Definitions {
        name: "t".into(),
        namespace: "urn:t".into(),
        inputs: input_names
            .iter()
            .map(|n| InputData {
                id: n.to_string(),
                name: n.to_string(),
                type_ref: None,
            })
            .collect(),
        decisions: vec![Decision {
            id: "d".into(),
            name: "d".into(),
            expr: Some(Boxed::DecisionTable(Box::new(dt.clone()))),
            ..Default::default()
        }],
        ..Default::default()
    };
    let set = ModelSet {
        main: defs,
        imports: vec![],
        all: vec![],
    };
    let mut l = Lowerer::new(&set);
    let prog = feel_compiler::ast::Program::Expression(feel_dmn::lower::path(l.lower_model(), "d"));
    let opts = l.options(&[], feel_compiler::Entry::Expression);
    let out = feel_compiler::compile_program(&prog, "", vec![], &opts);
    let engine = Engine::new();
    let p = engine.load(&out.wasm.unwrap()).unwrap();
    let mut i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    i.evaluate(
        inputs,
        &EvalOptions {
            clock: ClockSpec::None,
        },
    )
    .result
    .unwrap()
}

fn both(dt: &DecisionTable, names: &[&str], cases: &[(&str, &str)]) {
    let engine = Engine::new();
    let known: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let mut host = HostDecisionTable::compile(&engine, dt, &known).unwrap();
    assert!(host.modules_compiled > 0);
    for (inputs, expected) in cases {
        let h = host.evaluate(inputs).unwrap();
        let l = lowered(dt, names, inputs);
        assert_eq!(&h, expected, "host-orchestrated, inputs {inputs}");
        assert_eq!(&l, expected, "lowered, inputs {inputs}");
    }
}

#[test]
fn unique_with_wildcard_disjunction_intervals_and_nulls() {
    let dt = table(
        "UNIQUE",
        None,
        &["Applicant Age", "Risk Category"],
        &[("Decision", None)],
        &[
            (&["< 18", "-"], &["\"DECLINE\""]),
            (&["[18..65)", "\"LOW\", \"MEDIUM\""], &["\"ACCEPT\""]),
            (&["[18..65)", "\"HIGH\""], &["\"REFER\""]),
            (&[">= 65", "not(\"HIGH\")"], &["\"ACCEPT\""]),
            (&[">= 65", "\"HIGH\""], &["\"DECLINE\""]),
        ],
    );
    both(
        &dt,
        &["Applicant Age", "Risk Category"],
        &[
            (
                r#"{"Applicant Age": 17, "Risk Category": "HIGH"}"#,
                r#""DECLINE""#,
            ),
            (
                r#"{"Applicant Age": 18, "Risk Category": "MEDIUM"}"#,
                r#""ACCEPT""#,
            ),
            (
                r#"{"Applicant Age": 64.99, "Risk Category": "HIGH"}"#,
                r#""REFER""#,
            ),
            (
                r#"{"Applicant Age": 65, "Risk Category": "LOW"}"#,
                r#""ACCEPT""#,
            ),
            // `null = "HIGH"` is false (Table 49.a), so not("HIGH") matches a null candidate
            (
                r#"{"Applicant Age": 70, "Risk Category": null}"#,
                r#""ACCEPT""#,
            ),
            (r#"{"Applicant Age": null, "Risk Category": "LOW"}"#, "null"),
            (
                r#"{"Applicant Age": 30, "Risk Category": "UNKNOWN"}"#,
                "null",
            ),
        ],
    );
}

#[test]
fn overlapping_rules_under_each_hit_policy() {
    let rules: &[(&[&str], &[&str])] = &[
        (&[">= 10"], &["\"b\""]),
        (&[">= 5"], &["\"a\""]),
        (&["-"], &["\"c\""]),
    ];
    let v = Some("\"a\", \"b\", \"c\"");
    let cases = |u: &str, f: &str, p: &str, r: &str, o: &str, c: &str| {
        vec![
            ("UNIQUE", u),
            ("FIRST", f),
            ("PRIORITY", p),
            ("RULE ORDER", r),
            ("OUTPUT ORDER", o),
            ("COLLECT", c),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect::<Vec<_>>()
    };
    for (inputs, expect) in [
        (
            r#"{"x": 12}"#,
            cases(
                "null",
                r#""b""#,
                r#""a""#,
                r#"["b","a","c"]"#,
                r#"["a","b","c"]"#,
                r#"["b","a","c"]"#,
            ),
        ),
        (
            r#"{"x": 1}"#,
            cases(
                r#""c""#, r#""c""#, r#""c""#, r#"["c"]"#, r#"["c"]"#, r#"["c"]"#,
            ),
        ),
    ] {
        for (hp, e) in expect {
            let dt = table(&hp, None, &["x"], &[("o", v)], rules);
            both(&dt, &["x"], &[(inputs, &e)]);
        }
    }
    // ANY: overlapping rules must agree
    let any = table(
        "ANY",
        None,
        &["x"],
        &[("o", None)],
        &[
            (&[">= 5"], &["1"]),
            (&[">= 0"], &["1"]),
            (&["> 100"], &["2"]),
        ],
    );
    both(
        &any,
        &["x"],
        &[(r#"{"x": 6}"#, "1"), (r#"{"x": 200}"#, "null")],
    );
}

#[test]
fn collect_aggregations_and_compound_outputs() {
    let rules: &[(&[&str], &[&str])] = &[
        (&["> 0"], &["10"]),
        (&["> 5"], &["2.5"]),
        (&["> 100"], &["1"]),
    ];
    for (agg, e) in [
        ("SUM", "12.5"),
        ("COUNT", "2"),
        ("MIN", "2.5"),
        ("MAX", "10"),
    ] {
        let dt = table("COLLECT", Some(agg), &["x"], &[("o", None)], rules);
        both(&dt, &["x"], &[(r#"{"x": 6}"#, e)]);
    }
    let dt = table(
        "FIRST",
        None,
        &["x"],
        &[("rate", None), ("band", None)],
        &[
            (&["< 5"], &["0.05", "\"low\""]),
            (&["-"], &["x / 100", "\"high\""]),
        ],
    );
    both(
        &dt,
        &["x"],
        &[
            (r#"{"x": 1}"#, r#"{"rate":0.05,"band":"low"}"#),
            (r#"{"x": 7}"#, r#"{"rate":0.07,"band":"high"}"#),
        ],
    );
}
