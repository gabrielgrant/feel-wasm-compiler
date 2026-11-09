//! DMN integration layer for the FEEL → Wasm compiler.
//!
//! * [`model`] reads DMN XML (DRG elements, item definitions, boxed
//!   expressions);
//! * [`lower`] lowers a model to a FEEL context expression, including
//!   decision tables and their hit policies (orchestration lives here, not in
//!   the FEEL core);
//! * [`host_dt`] evaluates decision tables by orchestrating separately
//!   compiled modules (input expressions → candidate values → unary-test
//!   modules → output expressions) from the host;
//! * [`tck`] runs the DMN TCK against the generated Wasm.
//!
//! Scope: this crate provides enough of DMN to give FEEL its DMN context for
//! conformance testing. It is not a full DMN platform (no DMN XML schema
//! validation, DI, or authority requirements).

pub mod host_dt;
pub mod lower;
pub mod model;
pub mod tck;
