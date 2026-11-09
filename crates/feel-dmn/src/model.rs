//! A pragmatic reader for DMN 1.1–1.5 XML models (the subset needed to give
//! FEEL its DMN context: DRG elements, item definitions and boxed
//! expressions). Full DMN XML schema validation, DI and decision requirement
//! graph *authoring* semantics are out of scope.

use roxmltree::{Document, Node};

#[derive(Clone, Debug, Default)]
pub struct Definitions {
    pub name: String,
    pub namespace: String,
    pub imports: Vec<Import>,
    pub item_defs: Vec<ItemDef>,
    pub inputs: Vec<InputData>,
    pub decisions: Vec<Decision>,
    pub bkms: Vec<Bkm>,
    pub services: Vec<DecisionService>,
}

#[derive(Clone, Debug, Default)]
pub struct Import {
    pub name: String,
    pub namespace: String,
    pub location: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ItemDef {
    pub name: String,
    pub type_ref: Option<String>,
    pub is_collection: bool,
    pub components: Vec<ItemDef>,
    pub allowed_values: Option<String>,
    pub function_item: Option<FunctionItem>,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionItem {
    pub output_type_ref: Option<String>,
    pub params: Vec<(String, Option<String>)>,
}

#[derive(Clone, Debug, Default)]
pub struct InputData {
    pub id: String,
    pub name: String,
    pub type_ref: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Decision {
    pub id: String,
    pub name: String,
    pub type_ref: Option<String>,
    pub required_decisions: Vec<String>,
    pub required_inputs: Vec<String>,
    pub required_knowledge: Vec<String>,
    pub expr: Option<Boxed>,
}

#[derive(Clone, Debug, Default)]
pub struct Bkm {
    pub id: String,
    pub name: String,
    pub type_ref: Option<String>,
    pub logic: Option<FunctionDefinition>,
    pub required_knowledge: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DecisionService {
    pub id: String,
    pub name: String,
    pub type_ref: Option<String>,
    pub output_decisions: Vec<String>,
    pub encapsulated_decisions: Vec<String>,
    pub input_decisions: Vec<String>,
    pub input_data: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionDefinition {
    pub kind: String,
    pub params: Vec<(String, Option<String>)>,
    pub body: Option<Box<Boxed>>,
}

#[derive(Clone, Debug)]
pub enum Boxed {
    Literal {
        text: String,
        type_ref: Option<String>,
    },
    Context {
        entries: Vec<(Option<String>, Option<String>, Boxed)>,
    },
    DecisionTable(Box<DecisionTable>),
    Invocation {
        callee: Box<Boxed>,
        bindings: Vec<(String, Option<Boxed>)>,
    },
    Relation {
        columns: Vec<String>,
        rows: Vec<Vec<Boxed>>,
    },
    List(Vec<Boxed>),
    Function(FunctionDefinition),
    Conditional {
        cond: Box<Boxed>,
        then: Box<Boxed>,
        els: Box<Boxed>,
    },
    Filter {
        input: Box<Boxed>,
        matcher: Box<Boxed>,
    },
    For {
        var: String,
        input: Box<Boxed>,
        ret: Box<Boxed>,
    },
    Quantified {
        every: bool,
        var: String,
        input: Box<Boxed>,
        satisfies: Box<Boxed>,
    },
    /// unsupported element (name)
    Unsupported(String),
}

#[derive(Clone, Debug, Default)]
pub struct DecisionTable {
    pub hit_policy: String,
    pub aggregation: Option<String>,
    pub inputs: Vec<DtInput>,
    pub outputs: Vec<DtOutput>,
    pub rules: Vec<DtRule>,
}

#[derive(Clone, Debug, Default)]
pub struct DtInput {
    pub expr: String,
    pub type_ref: Option<String>,
    pub input_values: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DtOutput {
    pub name: Option<String>,
    pub type_ref: Option<String>,
    pub output_values: Option<String>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DtRule {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

fn local<'a>(n: &Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}

fn children<'a, 'i>(n: &Node<'a, 'i>, name: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn child<'a, 'i>(n: &Node<'a, 'i>, name: &'static str) -> Option<Node<'a, 'i>> {
    children(n, name).next()
}

fn text_of(n: &Node) -> String {
    match child(n, "text") {
        Some(t) => t.text().unwrap_or("").to_string(),
        None => String::new(),
    }
}

fn href_id(n: &Node) -> String {
    let h = n.attribute("href").unwrap_or("");
    match h.find('#') {
        Some(i) => h[i + 1..].to_string(),
        None => h.to_string(),
    }
}

/// Element id, falling back to the name for elements without an id.
fn id_of(n: &Node) -> String {
    n.attribute("id")
        .or_else(|| n.attribute("name"))
        .unwrap_or("")
        .to_string()
}

fn var_type(n: &Node) -> Option<String> {
    child(n, "variable")
        .and_then(|v| v.attribute("typeRef"))
        .map(|s| s.to_string())
}

pub fn parse_definitions(xml: &str) -> Result<Definitions, String> {
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if local(&root) != "definitions" {
        return Err("not a DMN definitions document".into());
    }
    let mut d = Definitions {
        name: root.attribute("name").unwrap_or("").to_string(),
        namespace: root.attribute("namespace").unwrap_or("").to_string(),
        ..Default::default()
    };
    for c in root.children().filter(|c| c.is_element()) {
        match local(&c) {
            "import" => d.imports.push(Import {
                name: c.attribute("name").unwrap_or("").to_string(),
                namespace: c.attribute("namespace").unwrap_or("").to_string(),
                location: c.attribute("locationURI").map(|s| s.to_string()),
            }),
            "itemDefinition" => d.item_defs.push(item_def(&c)),
            "inputData" => d.inputs.push(InputData {
                id: id_of(&c),
                name: c.attribute("name").unwrap_or("").to_string(),
                type_ref: var_type(&c),
            }),
            "decision" => {
                let mut dec = Decision {
                    id: id_of(&c),
                    name: c.attribute("name").unwrap_or("").to_string(),
                    type_ref: var_type(&c),
                    ..Default::default()
                };
                for r in children(&c, "informationRequirement") {
                    if let Some(x) = child(&r, "requiredDecision") {
                        dec.required_decisions.push(href_id(&x));
                    }
                    if let Some(x) = child(&r, "requiredInput") {
                        dec.required_inputs.push(href_id(&x));
                    }
                }
                for r in children(&c, "knowledgeRequirement") {
                    if let Some(x) = child(&r, "requiredKnowledge") {
                        dec.required_knowledge.push(href_id(&x));
                    }
                }
                dec.expr = c
                    .children()
                    .filter(|x| x.is_element())
                    .find_map(|x| boxed(&x));
                d.decisions.push(dec);
            }
            "businessKnowledgeModel" => {
                let mut b = Bkm {
                    id: id_of(&c),
                    name: c.attribute("name").unwrap_or("").to_string(),
                    type_ref: var_type(&c),
                    ..Default::default()
                };
                if let Some(el) = child(&c, "encapsulatedLogic") {
                    b.logic = Some(function_def(&el));
                }
                for r in children(&c, "knowledgeRequirement") {
                    if let Some(x) = child(&r, "requiredKnowledge") {
                        b.required_knowledge.push(href_id(&x));
                    }
                }
                d.bkms.push(b);
            }
            "decisionService" => {
                let refs = |name: &'static str| {
                    children(&c, name).map(|x| href_id(&x)).collect::<Vec<_>>()
                };
                d.services.push(DecisionService {
                    id: id_of(&c),
                    name: c.attribute("name").unwrap_or("").to_string(),
                    type_ref: var_type(&c),
                    output_decisions: refs("outputDecision"),
                    encapsulated_decisions: refs("encapsulatedDecision"),
                    input_decisions: refs("inputDecision"),
                    input_data: refs("inputData"),
                });
            }
            _ => {}
        }
    }
    Ok(d)
}

fn item_def(n: &Node) -> ItemDef {
    let tr = child(n, "typeRef")
        .and_then(|t| t.text())
        .map(|s| s.trim().to_string());
    let allowed = child(n, "allowedValues")
        .or_else(|| child(n, "typeConstraint"))
        .map(|a| text_of(&a))
        .filter(|s| !s.trim().is_empty());
    let fi = child(n, "functionItem").map(|f| FunctionItem {
        output_type_ref: f.attribute("outputTypeRef").map(|s| s.to_string()),
        params: children(&f, "parameters")
            .map(|p| {
                (
                    p.attribute("name").unwrap_or("").to_string(),
                    p.attribute("typeRef").map(|s| s.to_string()),
                )
            })
            .collect(),
    });
    ItemDef {
        name: n.attribute("name").unwrap_or("").to_string(),
        type_ref: tr,
        is_collection: n.attribute("isCollection") == Some("true"),
        components: children(n, "itemComponent").map(|c| item_def(&c)).collect(),
        allowed_values: allowed,
        function_item: fi,
    }
}

fn function_def(n: &Node) -> FunctionDefinition {
    FunctionDefinition {
        kind: n.attribute("kind").unwrap_or("FEEL").to_string(),
        params: children(n, "formalParameter")
            .map(|p| {
                (
                    p.attribute("name").unwrap_or("").to_string(),
                    p.attribute("typeRef").map(|s| s.to_string()),
                )
            })
            .collect(),
        body: n
            .children()
            .filter(|x| x.is_element() && local(x) != "formalParameter")
            .find_map(|x| boxed(&x))
            .map(Box::new),
    }
}

fn expr_child(n: &Node) -> Option<Boxed> {
    n.children()
        .filter(|x| x.is_element())
        .find_map(|x| boxed(&x))
}

fn boxed(n: &Node) -> Option<Boxed> {
    let b = match local(n) {
        "literalExpression" => Boxed::Literal {
            text: text_of(n),
            type_ref: n.attribute("typeRef").map(|s| s.to_string()),
        },
        "context" => Boxed::Context {
            entries: children(n, "contextEntry")
                .map(|e| {
                    let v = child(&e, "variable");
                    (
                        v.and_then(|v| v.attribute("name")).map(|s| s.to_string()),
                        v.and_then(|v| v.attribute("typeRef"))
                            .map(|s| s.to_string()),
                        expr_child(&e).unwrap_or(Boxed::Literal {
                            text: "null".into(),
                            type_ref: None,
                        }),
                    )
                })
                .collect(),
        },
        "decisionTable" => {
            let mut dt = DecisionTable {
                hit_policy: n.attribute("hitPolicy").unwrap_or("UNIQUE").to_string(),
                aggregation: n.attribute("aggregation").map(|s| s.to_string()),
                ..Default::default()
            };
            for i in children(n, "input") {
                let ie = child(&i, "inputExpression");
                dt.inputs.push(DtInput {
                    expr: ie.map(|x| text_of(&x)).unwrap_or_default(),
                    type_ref: ie
                        .and_then(|x| x.attribute("typeRef"))
                        .map(|s| s.to_string()),
                    input_values: child(&i, "inputValues").map(|x| text_of(&x)),
                });
            }
            for o in children(n, "output") {
                dt.outputs.push(DtOutput {
                    name: o.attribute("name").map(|s| s.to_string()),
                    type_ref: o.attribute("typeRef").map(|s| s.to_string()),
                    output_values: child(&o, "outputValues").map(|x| text_of(&x)),
                    default: child(&o, "defaultOutputEntry").map(|x| text_of(&x)),
                });
            }
            for r in children(n, "rule") {
                dt.rules.push(DtRule {
                    inputs: children(&r, "inputEntry").map(|x| text_of(&x)).collect(),
                    outputs: children(&r, "outputEntry").map(|x| text_of(&x)).collect(),
                });
            }
            Boxed::DecisionTable(Box::new(dt))
        }
        "invocation" => {
            let callee = n
                .children()
                .filter(|x| x.is_element() && local(x) != "binding")
                .find_map(|x| boxed(&x))?;
            Boxed::Invocation {
                callee: Box::new(callee),
                bindings: children(n, "binding")
                    .map(|b| {
                        let p = child(&b, "parameter")
                            .and_then(|p| p.attribute("name"))
                            .unwrap_or("")
                            .to_string();
                        (
                            p,
                            b.children()
                                .filter(|x| x.is_element() && local(x) != "parameter")
                                .find_map(|x| boxed(&x)),
                        )
                    })
                    .collect(),
            }
        }
        "relation" => Boxed::Relation {
            columns: children(n, "column")
                .map(|c| c.attribute("name").unwrap_or("").to_string())
                .collect(),
            rows: children(n, "row")
                .map(|r| {
                    r.children()
                        .filter(|x| x.is_element())
                        .filter_map(|x| boxed(&x))
                        .collect()
                })
                .collect(),
        },
        "list" => Boxed::List(
            n.children()
                .filter(|x| x.is_element())
                .filter_map(|x| boxed(&x))
                .collect(),
        ),
        "functionDefinition" => Boxed::Function(function_def(n)),
        "conditional" => {
            let part =
                |name: &'static str| child(n, name).and_then(|x| expr_child(&x)).map(Box::new);
            Boxed::Conditional {
                cond: part("if")?,
                then: part("then")?,
                els: part("else")?,
            }
        }
        "filter" => {
            let part =
                |name: &'static str| child(n, name).and_then(|x| expr_child(&x)).map(Box::new);
            Boxed::Filter {
                input: part("in")?,
                matcher: part("match")?,
            }
        }
        "for" | "some" | "every" => {
            let var = n.attribute("iteratorVariable").unwrap_or("").to_string();
            let part =
                |name: &'static str| child(n, name).and_then(|x| expr_child(&x)).map(Box::new);
            match local(n) {
                "for" => Boxed::For {
                    var,
                    input: part("in")?,
                    ret: part("return")?,
                },
                q => Boxed::Quantified {
                    every: q == "every",
                    var,
                    input: part("in")?,
                    satisfies: part("satisfies")?,
                },
            }
        }
        "variable"
        | "informationRequirement"
        | "knowledgeRequirement"
        | "authorityRequirement"
        | "description"
        | "extensionElements"
        | "question"
        | "allowedAnswers"
        | "formalParameter"
        | "encapsulatedLogic"
        | "text" => return None,
        other => Boxed::Unsupported(other.to_string()),
    };
    Some(b)
}
