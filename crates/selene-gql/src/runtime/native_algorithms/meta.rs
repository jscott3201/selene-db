//! Metadata builders shared by the native `algo.*` procedure definitions.
//!
//! These mirror the historical pack's `parameter` / `output` helpers plus its
//! `external_parameter` / `external_output_column` conversion: a nullable
//! parameter carries the `"NULL (use procedure default)"` default-doc and every
//! parameter / column carries the same boilerplate description the pack emitted,
//! so `SHOW PROCEDURES` introspection is byte-identical.

use selene_core::intern_with_admission;

use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter};

/// Build a procedure parameter, attaching the pack-era default-doc for nullable
/// (optional) parameters.
///
/// # Panics
///
/// Panics only if the global interner is exhausted while interning a static
/// parameter name. Procedure names are static and few; this mirrors the pack
/// registry, which surfaced interner exhaustion as a construction error. The
/// native registry is built once at engine startup from a fixed name set.
pub(super) fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    let parameter = ProcedureParameter::new(intern_static(name), ty, nullable)
        .with_description("Procedure parameter.");
    if nullable {
        parameter.with_default_doc("NULL (use procedure default)")
    } else {
        parameter
    }
}

/// Build a procedure output column with the pack-era boilerplate description.
pub(super) fn output(name: &'static str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(intern_static(name), ty).with_description("Procedure output column.")
}

fn intern_static(value: &'static str) -> selene_core::IStr {
    intern_with_admission(value)
        .map(|(istr, _was_new)| istr)
        .expect("static procedure metadata name interns")
}
