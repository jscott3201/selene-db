//! Shared helpers for index-aware optimizer rules.

use crate::{
    LabelExpr, Literal,
    analyze::BindingId,
    plan::{BindingDef, BindingElement, IndexKind, IndexTarget},
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
        Literal::Bool(_, _) | Literal::Null(_) => None,
    }
}

/// Return whether a literal can be served by an index kind.
pub(super) fn literal_matches_kind(literal: &Literal, kind: IndexKind) -> bool {
    literal_index_kind(literal) == Some(kind)
}
