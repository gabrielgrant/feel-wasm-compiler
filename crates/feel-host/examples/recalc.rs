//! `cargo run -p feel-host --example recalc` — incremental recalculation.
use feel_host::recalc::Workbook;

fn main() {
    let mut wb = Workbook::new();
    wb.set_inputs(&[
        ("price", "19.99"),
        ("quantity", "3"),
        ("tax rate", "0.0825"),
        ("customer", r#"{"tier": "gold", "since": 2015}"#),
    ]);
    wb.define("subtotal", "price * quantity").unwrap();
    wb.define(
        "discount",
        r#"if customer.tier = "gold" then subtotal * 0.1 else 0"#,
    )
    .unwrap();
    wb.define("tax", "decimal((subtotal - discount) * tax rate, 2)")
        .unwrap();
    wb.define("total", "subtotal - discount + tax").unwrap();
    wb.define("loyal years", "2024 - customer.since").unwrap();
    wb.recalc_all();
    println!(
        "initial: total = {}, evaluated {:?}",
        wb.value("total").unwrap(),
        wb.last_evaluated
    );
    wb.set_inputs(&[("quantity", "4")]);
    println!(
        "quantity := 4: total = {}, re-evaluated {:?}",
        wb.value("total").unwrap(),
        wb.last_evaluated
    );
    wb.set_inputs(&[("tax rate", "0.07")]);
    println!(
        "tax rate := 0.07: total = {}, re-evaluated {:?}",
        wb.value("total").unwrap(),
        wb.last_evaluated
    );
    // redefining with identical source/options hits the module cache
    wb.define("subtotal", "price * quantity").unwrap();
    println!(
        "module cache: {} entries, {} hits, {} misses",
        wb.cache.len(),
        wb.cache.hits,
        wb.cache.misses
    );
}
