use feel_compiler::{compile, CompileOptions};
use feel_host::{Engine, EvalOptions};
fn main() {
    let out = compile("a + b", &CompileOptions::default());
    let engine = Engine::new();
    let program = engine.load(out.wasm.as_ref().unwrap()).unwrap();
    let mut inst = program.instantiate(&engine, Default::default()).unwrap();
    let r = inst.evaluate(r#"{"a": 0.1, "b": 0.2}"#, &EvalOptions::default());
    println!("{:?}", r.result);
}
