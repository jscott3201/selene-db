//! Shared helpers for index-aware optimizer rules.

use selene_core::IStr;

use crate::{
    FilterPredicate, GqlType, LabelExpr, Literal, ValueExpr,
    analyze::BindingId,
    plan::{BindingDef, BindingElement, IndexKey, IndexKind, IndexTarget, optimize::binding_refs},
};

/// Return the single label carried by a label expression.
pub(super) fn single_label(label: &Option<LabelExpr>) -> Option<selene_core::IStr> {
    match label {
        Some(LabelExpr::Single(label)) => Some(label.clone()),
        _ => None,
    }
}

/// Return the flat list of single labels carried by a `Disjunction` label
/// expression, or `None` if the expression is not a flat disjunction of
/// singles (or is anything other than `Disjunction`).
///
/// IMPORTANT — idempotency contract: this helper returns `None` for
/// `LabelExpr::Single`. The `disjunctive_label_expansion` rule wraps a
/// `Disjunction` into a `JoinTree::DisjunctiveScan`; after expansion, every
/// branch's `label_predicate` is `Some(LabelExpr::Single(L_i))`. If this
/// helper returned `Some(vec![L])` for `Single(L)`, the rule would
/// re-fire on every branch in the next optimizer iteration, looping until
/// `max_optimizer_iterations` caps with a wrong plan. Returning `None`
/// makes the rule a no-op on already-expanded branches.
///
/// Returns `None` for:
/// - `LabelExpr::Single(_)` (idempotency — see above; also the
///   single-label-MATCH case the rule should never re-shape).
/// - `LabelExpr::Conjunction(..)` (intersection semantics; out of scope).
/// - `LabelExpr::Negation(..)` (complement semantics; out of scope).
/// - `LabelExpr::Wildcard` (matches any label; expansion would be infinite).
/// - `None` (no label predicate; nothing to expand).
/// - `Disjunction` whose parts contain any non-`Single` inner branch
///   (`A|B&C`, `A|!B`, etc. — F10 fold).
///
/// `LabelExpr::Disjunction` wraps `Vec2OrMore<LabelExpr>`
/// (`ast/util.rs:142-201`), so a returned `Some(parts)` always has
/// `parts.len() >= 2` — no empty/1-element edge case to handle.
pub(super) fn flat_disjunction_singles(
    label: &Option<LabelExpr>,
) -> Option<Vec<selene_core::IStr>> {
    let Some(LabelExpr::Disjunction(parts)) = label else {
        return None;
    };
    parts
        .iter()
        .map(|part| match part {
            LabelExpr::Single(label) => Some(label.clone()),
            _ => None,
        })
        .collect()
}

/// Return the target kind for a binding element.
pub(super) fn target_for_element(element: BindingElement) -> Option<IndexTarget> {
    match element {
        BindingElement::Node => Some(IndexTarget::Node),
        BindingElement::Edge => Some(IndexTarget::Edge),
        BindingElement::Path => None,
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
        Literal::Date(_, _) => Some(IndexKind::Date),
        Literal::LocalDateTime(_, _) => Some(IndexKind::LocalDateTime),
        Literal::Uuid(_, _) => Some(IndexKind::Uuid),
        Literal::Bool(_, _)
        | Literal::ZonedDateTime(_, _)
        | Literal::ZonedTime(_, _)
        | Literal::LocalTime(_, _)
        | Literal::Duration(_, _)
        | Literal::Null(_) => None,
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

/// One equality-shaped property predicate against a node binding.
///
/// Carries the candidate's `property_predicates` index, the property key, and
/// a borrow of the equality value (literal or parameter) so the caller can
/// downstream-validate the value against an index kind via [`compatible_value`].
///
/// Lifted from `composite_index_lookup.rs` so both the composite-index rule
/// and the disjunctive-label-expansion applicability gate (BRIEF-155) can
/// share the equality-candidate collection logic.
#[derive(Clone)]
pub(super) struct EqualityCandidate<'a> {
    /// Position of the candidate within the scan's `property_predicates`.
    pub(super) index: usize,
    /// Property key the equality predicate references.
    pub(super) key: IStr,
    /// Equality value (literal or parameter slot).
    pub(super) value: &'a ValueExpr,
}

/// Collect the equality-shaped node-binding predicates from a scan's
/// `property_predicates`.
///
/// Filters down to:
/// - bindings of [`BindingElement::Node`] (composite indexes are node-only at HEAD).
/// - shape [`binding_refs::PropertyPredicateShape::Equality`].
/// - value either a literal or a parameter slot.
/// - per-key dedup — only the first predicate per property key is kept (any
///   further equalities on the same key stay residual).
pub(super) fn equality_candidates<'a>(
    predicates: &'a [FilterPredicate],
    bindings: &'a [BindingDef],
) -> Vec<EqualityCandidate<'a>> {
    let mut candidates: Vec<EqualityCandidate<'a>> = Vec::new();
    for (index, pred) in predicates.iter().enumerate() {
        let Some(matched) = binding_refs::match_property_predicate(pred, bindings) else {
            continue;
        };
        let binding_id = matched.binding;
        let Some(binding_def) = bindings
            .iter()
            .find(|binding| binding.binding == binding_id)
        else {
            continue;
        };
        if binding_def.element != BindingElement::Node {
            continue;
        }
        if candidates
            .iter()
            .any(|candidate| candidate.key == matched.key)
        {
            continue;
        }
        let binding_refs::PropertyPredicateShape::Equality(value) = matched.shape else {
            continue;
        };
        // Accept either a literal or a parameter slot. Per-component kind
        // checks (literal kind, typed-parameter compatibility) happen in the
        // caller once the per-column IndexKind is known.
        if binding_refs::literal(value).is_none() && binding_refs::parameter(value).is_none() {
            continue;
        }
        candidates.push(EqualityCandidate {
            index,
            key: matched.key,
            value,
        });
    }
    candidates
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
///   matching GqlType variant. `STRING` binds to `Value::String`.
///
/// Untyped parameters (no `$id :: TYPE` declaration) bypass this check and
/// defer to the execute-time `IndexKind` probe on the resolved Value.
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
        // BRIEF-154 PR #175 F2 (Codex P2): admit only `FLOAT64` for
        // `IndexKind::Float`. `GqlType::Float` is width-generic per
        // `parameter_type::validate_declared_type`, which accepts both
        // `Value::Float` and `Value::Float32`; but the runtime index
        // probe rejects `Value::Float32` via `check_value_index_kind`.
        // Admitting `GqlType::Float` here would let a `$p :: FLOAT`
        // parameter optimize through the indexed path and then error
        // `InvalidParameterType` when bound to a `Value::Float32`,
        // while the non-indexed equivalent query would compare normally
        // via `value_compare`. Reject `FLOAT` so it falls back to
        // Linear at plan time and the two paths agree.
        IndexKind::Float => matches!(ty, GqlType::Float64),
        IndexKind::String => matches!(ty, GqlType::String),
        IndexKind::Date => matches!(ty, GqlType::Date),
        IndexKind::LocalDateTime => matches!(ty, GqlType::LocalDateTime),
        IndexKind::Uuid => matches!(ty, GqlType::Uuid),
    }
}
