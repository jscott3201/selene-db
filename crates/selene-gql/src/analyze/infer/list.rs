//! List type metadata helpers.

use crate::GqlType;

type MeetFn = fn(&GqlType, &GqlType) -> Option<GqlType>;

/// Return the result type for list concatenation, summing bounds when both
/// operands have static maximum cardinalities.
pub(crate) fn list_concat_type(lhs: &GqlType, rhs: &GqlType, meet: MeetFn) -> Option<GqlType> {
    let (Some(lhs_inner), Some(rhs_inner)) = (list_element_type(lhs), list_element_type(rhs))
    else {
        return None;
    };
    let max_len = match (list_max_len(lhs), list_max_len(rhs)) {
        (Some(lhs_max), Some(rhs_max)) => lhs_max.checked_add(rhs_max),
        _ => None,
    };
    meet(lhs_inner, rhs_inner).map(|inner| list_type(inner, max_len))
}

/// Return the common list type for branch/list unification, widening bounded
/// operands to the larger maximum cardinality.
pub(crate) fn list_union_type(lhs: &GqlType, rhs: &GqlType, meet: MeetFn) -> Option<GqlType> {
    let (Some(lhs_inner), Some(rhs_inner)) = (list_element_type(lhs), list_element_type(rhs))
    else {
        return None;
    };
    let max_len = match (list_max_len(lhs), list_max_len(rhs)) {
        (Some(lhs_max), Some(rhs_max)) => Some(lhs_max.max(rhs_max)),
        _ => None,
    };
    meet(lhs_inner, rhs_inner).map(|ty| list_type(ty, max_len))
}

/// Return the element type for either bounded or unbounded list value types.
pub(crate) fn list_element_type(ty: &GqlType) -> Option<&GqlType> {
    match ty {
        GqlType::List(inner)
        | GqlType::BoundedList {
            element_type: inner,
            ..
        } => Some(inner),
        _ => None,
    }
}

/// Return the maximum cardinality for bounded list value types.
pub(crate) fn list_max_len(ty: &GqlType) -> Option<u64> {
    match ty {
        GqlType::BoundedList { max_len, .. } => Some(*max_len),
        _ => None,
    }
}

/// Build a bounded list when `max_len` is present, otherwise an unbounded list.
pub(crate) fn list_type(element_type: GqlType, max_len: Option<u64>) -> GqlType {
    match max_len {
        Some(max_len) => GqlType::BoundedList {
            element_type: Box::new(element_type),
            max_len,
        },
        None => GqlType::List(Box::new(element_type)),
    }
}
