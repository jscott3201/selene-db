//! Shared helpers for index-aware optimizer rules.

use crate::{
    GqlType, LabelExpr, Literal, ValueExpr,
    analyze::BindingId,
    plan::{BindingDef, BindingElement, IndexKey, IndexKind, IndexTarget, optimize::binding_refs},
};

/// Return the single label carried by a label expression.
pub(super) fn single_label(label: &Option<LabelExpr>) -> Option<selene_core::IStr> {
    match label {
        Some(LabelExpr::Single(label)) => Some(*label),
        _ => None,
    }
}

/// Return the target kind for a binding element.
pub(super) fn target_for_element(element: BindingElement) -> Option<IndexTarget> {
    match element {
        BindingElement::Node => Some(IndexTarget::Node),
        BindingElement::Edge => Some(IndexTarget::Edge),
        BindingElement::Path | BindingElement::Alias => None,
    }
}

/// Return the index target and single label for a binding.
pub(super) fn binding_index_target(
    bindings: &[BindingDef],
    binding_id: BindingId,
) -> Option<(IndexTarget, selene_core::IStr)> {
    let binding = bindings
        .iter()
        .find(|binding| binding.binding == binding_id)?;
    Some((
        target_for_element(binding.element)?,
        single_label(&binding.label_predicate)?,
    ))
}

/// Return the index kind represented by a literal.
pub(super) fn literal_index_kind(literal: &Literal) -> Option<IndexKind> {
    match literal {
        Literal::Integer(_, _) => Some(IndexKind::Integer),
        Literal::Float(_, _) => Some(IndexKind::Float),
        Literal::String(_, _) => Some(IndexKind::String),
        Literal::Uuid(_, _) => Some(IndexKind::Uuid),
        Literal::Bool(_, _) | Literal::Null(_) => None,
    }
}

/// Return whether a literal can be served by an index kind.
pub(super) fn literal_matches_kind(literal: &Literal, kind: IndexKind) -> bool {
    literal_index_kind(literal) == Some(kind)
}

/// Resolve `value` to an [`IndexKey`] admissible for the given index kind.
///
/// Literal-side values are kind-checked at plan time via
/// [`literal_matches_kind`]. Parameter-side values are admitted when either
/// untyped (deferred to the execute-time probe per BRIEF-154 §B.5) or
/// typed-compatible per [`gql_type_compatible_with_index_kind`]. Typed
/// incompatibility falls through to `None` so the caller can fall back to a
/// linear scan per Q5.
pub(super) fn compatible_value(value: &ValueExpr, kind: IndexKind) -> Option<IndexKey> {
    if let Some(literal) = binding_refs::literal(value) {
        return literal_matches_kind(literal, kind).then(|| IndexKey::Literal(literal.clone()));
    }
    let param = binding_refs::parameter(value)?;
    if let Some(declared) = param.declared_type
        && !gql_type_compatible_with_index_kind(declared, kind)
    {
        return None;
    }
    Some(IndexKey::Parameter {
        name: param.name,
        declared_type: param.declared_type.cloned(),
        span: param.span,
    })
}

/// Return whether a typed parameter declaration is compatible with the
/// indexed-column kind so the optimizer rule can safely admit a parameter
/// slot at plan time (BRIEF-154 §B.2 F10).
///
/// The mapping mirrors which [`selene_core::Value`] variants the indexed
/// storage actually probes against:
/// - `IndexKind::Integer` keys are `Value::Int` (i64). `GqlType` variants that
///   bind to `Value::Int` per [`crate::runtime::parameter_type::validate_declared_type`]
///   are admissible: `INTEGER | INT8 | INT16 | INT32 | INT64 | SMALLINT | BIGINT`.
///   `INT128` (`Value::Int128`), the unsigned family (`Value::Uint{,128}`), and
///   `DECIMAL` are NOT `Value::Int` — reject.
/// - `IndexKind::Float` keys are `Value::Float` (f64). `FLOAT` and `FLOAT64`
///   bind to `Value::Float`; `FLOAT32` binds to `Value::Float32` and would
///   need an explicit cast, so it is rejected.
/// - `IndexKind::String`, `Date`, `LocalDateTime`, `Uuid` are 1:1 with the
///   matching GqlType variant. `STRING` admits both `Value::String` and
///   `Value::ExternalString`; ExternalString carve-out is handled at execute
///   time via [`selene_core::lookup`] (see BRIEF-153 + §B.3 F4).
///
/// Untyped parameters (no `$id :: TYPE` declaration) bypass this check and
/// defer to the execute-time `IndexKind` probe on the resolved Value.
#[allow(dead_code)] // Commit 2 lights up the caller in `compatible_value`.
pub(super) fn gql_type_compatible_with_index_kind(ty: &GqlType, kind: IndexKind) -> bool {
    match kind {
        IndexKind::Integer => matches!(
            ty,
            GqlType::Integer
                | GqlType::Int8
                | GqlType::Int16
                | GqlType::Int32
                | GqlType::Int64
                | GqlType::SmallInt
                | GqlType::BigInt
        ),
        IndexKind::Float => matches!(ty, GqlType::Float | GqlType::Float64),
        IndexKind::String => matches!(ty, GqlType::String),
        IndexKind::Date => matches!(ty, GqlType::Date),
        IndexKind::LocalDateTime => matches!(ty, GqlType::LocalDateTime),
        IndexKind::Uuid => matches!(ty, GqlType::Uuid),
    }
}
