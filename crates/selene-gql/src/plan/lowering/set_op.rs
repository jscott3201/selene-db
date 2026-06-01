//! Set-composition arm combinability checks (ISO/IEC 39075:2024 §14.2).

use crate::{
    SetOp, SourceSpan,
    plan::{BindingTableSchema, PlannerError},
};

/// Enforce ISO §14.2 SR v: the columns of two set-composition arms shall be
/// column name-equal (and column-combinable).
///
/// selene binds each arm in an independent projection scope, so without this
/// check `RETURN 1 AS x UNION ALL RETURN 2 AS y` would silently relabel the
/// right arm to the left arm's schema. Column name-equality is the positional
/// realization of the §4.15.4 field-name bijection over the ordered binding
/// table: each position's names must match (both unnamed counts as equal, so
/// `RETURN 1 UNION RETURN 2` stays legal). Differing column counts are reported
/// by the runtime `assert_compatible_schemas`; here we focus on the names of
/// the positions the two arms share.
pub(super) fn assert_arms_column_name_equal(
    op: SetOp,
    lhs: &BindingTableSchema,
    rhs: &BindingTableSchema,
    span: SourceSpan,
) -> Result<(), PlannerError> {
    for (position, (lhs_col, rhs_col)) in lhs.columns.iter().zip(&rhs.columns).enumerate() {
        if lhs_col.name != rhs_col.name {
            return Err(PlannerError::SetOpArmsNotCombinable {
                op: op_name(op),
                position,
                lhs: lhs_col.name.clone(),
                rhs: rhs_col.name.clone(),
                span,
            });
        }
    }
    Ok(())
}

const fn op_name(op: SetOp) -> &'static str {
    match op {
        SetOp::Union | SetOp::UnionAll => "UNION",
        SetOp::Intersect | SetOp::IntersectAll => "INTERSECT",
        SetOp::Except | SetOp::ExceptAll => "EXCEPT",
        SetOp::Otherwise => "OTHERWISE",
    }
}
