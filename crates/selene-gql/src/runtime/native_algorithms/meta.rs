//! Metadata builders shared by the native `algo.*` procedure definitions.
//!
//! These mirror the historical pack's `parameter` / `output` helpers plus its
//! `external_parameter` / `external_output_column` conversion: a nullable
//! parameter carries the `"NULL (use procedure default)"` default-doc and every
//! parameter / column carries the same boilerplate description the pack emitted,
//! so `SHOW PROCEDURES` introspection is byte-identical.

use selene_core::db_string;

use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter};

/// Build a procedure parameter, attaching the pack-era default-doc for nullable
/// (optional) parameters.
///
/// # Panics
///
/// Panics only if a static parameter name exceeds the per-string byte cap
/// (IL013). Procedure metadata is a fixed, short, source-owned name set.
pub(super) fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    let parameter = ProcedureParameter::new(static_db_string(name), ty, nullable)
        .with_description("Procedure parameter.");
    if nullable {
        parameter.with_default_doc("NULL (use procedure default)")
    } else {
        parameter
    }
}

/// Build a procedure output column with the pack-era boilerplate description.
pub(super) fn output(name: &'static str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(static_db_string(name), ty)
        .with_description("Procedure output column.")
}

fn static_db_string(value: &'static str) -> selene_core::DbString {
    db_string(value).expect("static procedure metadata name fits DB string cap")
}
