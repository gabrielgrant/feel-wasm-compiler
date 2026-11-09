fn main() {
    // Export the shadow-stack pointer so a JavaScript host can restore it after
    // a trap unwound runtime frames without running their epilogues.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg=--export=__stack_pointer");
    }
}
