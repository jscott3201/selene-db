//! Expression Flagger walk.

use selene_core::{DbString, feature_register::FeatureId};

use crate::{
    NonEmpty, ValueExpr,
    ast::{
        expr::{BinaryOp, IntegerLiteralKind, IsCheckKind, Literal},
        types::{GqlType, RecordType},
    },
};

use super::{FeatureUse, query, record_feature};

pub(crate) fn value(value: &ValueExpr, uses: &mut Vec<FeatureUse>) {
    // Variants whose feature surface is not a function of their child
    // `ValueExpr`s — leaves, subqueries (bodies are `MatchClause` /
    // `QueryPipeline`, not `ValueExpr` children), and `IS` predicates (whose
    // `kind`-borne feature must be recorded *between* the operand and the
    // `IS [SOURCE|DESTINATION] OF` value, an ordering `for_each_child` cannot
    // express) — keep explicit handling. The early returns mean these arms own
    // their full traversal and must not fall through to the generic child walk.
    match value {
        ValueExpr::Literal(literal_value) => {
            literal(literal_value, uses);
            return;
        }
        ValueExpr::Variable { .. } => return,
        ValueExpr::Parameter {
            declared_type,
            span,
            ..
        } => {
            record_feature(uses, FeatureId::GE04, *span);
            record_feature(uses, FeatureId::GE05, *span);
            if declared_type.is_some() {
                record_feature(uses, FeatureId::IM_TYPED_PARAMS, *span);
            }
            return;
        }
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            // Preserve the original ordering: operand subtree, then the
            // `kind`-borne feature, then the `IS [SOURCE|DESTINATION] OF` value
            // subtree (`is_check` records the feature before recursing).
            self::value(operand, uses);
            is_check(kind, *span, uses);
            return;
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            query::match_clause(pattern, uses);
            return;
        }
        ValueExpr::ValueSubquery { body, span } => {
            record_feature(uses, FeatureId::GQ18, *span);
            query::query_pipeline(body, uses);
            return;
        }
        // Variants whose own feature surface is recorded before their direct
        // `ValueExpr` children. The feature is emitted here; child recursion is
        // delegated to `for_each_child` below so the per-variant traversal
        // shape lives in exactly one place (`ast/walk.rs`).
        ValueExpr::ListAccess { span, .. } => {
            record_feature(uses, FeatureId::IM_LIST_SUBSCRIPT, *span);
        }
        ValueExpr::ListLiteral { span, .. } => record_feature(uses, FeatureId::GV50, *span),
        ValueExpr::RecordLiteral { span, .. } => record_feature(uses, FeatureId::GV45, *span),
        ValueExpr::BinaryOp { op, span, .. } => {
            if matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Power
            ) {
                record_feature(uses, FeatureId::GA01, *span);
            }
            if *op == BinaryOp::Xor {
                record_feature(uses, FeatureId::GE07, *span);
            }
        }
        ValueExpr::FunctionCall {
            name, args, span, ..
        } => {
            if let Some(feature_id) = scalar_function_feature(name) {
                record_feature(uses, feature_id, *span);
            }
            if is_list_value_function(name, args.len()) {
                record_feature(uses, FeatureId::GV50, *span);
            }
            if let Some(feature_id) = aggregate_function_feature(name) {
                record_feature(uses, feature_id, *span);
            }
        }
        ValueExpr::Trim {
            character,
            source,
            span,
            ..
        } => {
            record_feature(uses, FeatureId::GF06, *span);
            if is_byte_string_expr(source) || character.as_deref().is_some_and(is_byte_string_expr)
            {
                record_feature(uses, FeatureId::GF07, *span);
            }
        }
        ValueExpr::AllDifferent { span, .. } => record_feature(uses, FeatureId::G113, *span),
        ValueExpr::Same { span, .. } => record_feature(uses, FeatureId::G114, *span),
        ValueExpr::PropertyExists { span, .. } => record_feature(uses, FeatureId::G115, *span),
        ValueExpr::Cast {
            target_type, span, ..
        } => {
            // Per ISO/IEC 39075:2024 §20.8 <cast specification>: CAST is the
            // optional feature GA05 "Cast specification" (Annex D Table D.1 row
            // 53), NOT GE08 ("Reference parameters", §17.7). ISO Annex A item 52
            // requires GA05 for any `<cast specification>`. The Cast node records
            // GA05 first, then delegates child recursion to `for_each_child` (via
            // `value_children`), then walks the target type — the type tail must
            // run *after* child recursion to preserve source order.
            record_feature(uses, FeatureId::GA05, *span);
            self::value_children(value, uses);
            gql_type(target_type, *span, uses);
            return;
        }
        // Pure-recursion variants own no feature; they delegate entirely to the
        // generic child walk.
        ValueExpr::PropertyAccess { .. }
        | ValueExpr::UnaryOp { .. }
        | ValueExpr::Normalize { .. }
        | ValueExpr::InList { .. }
        | ValueExpr::InListExpression { .. }
        | ValueExpr::Case { .. } => {}
    }
    self::value_children(value, uses);
}

/// Recurse into every direct child [`ValueExpr`] of `value`, in source order,
/// delegating the per-variant child-enumeration shape to
/// [`ValueExpr::for_each_child`].
fn value_children(value: &ValueExpr, uses: &mut Vec<FeatureUse>) {
    value.for_each_child(&mut |child| self::value(child, uses));
}

fn is_byte_string_expr(value: &ValueExpr) -> bool {
    match value {
        ValueExpr::Literal(Literal::Bytes(_, _)) => true,
        ValueExpr::Cast { target_type, .. } => target_type.as_ref() == &GqlType::Bytes,
        ValueExpr::Parameter {
            declared_type: Some(GqlType::Bytes),
            ..
        } => true,
        _ => false,
    }
}

fn scalar_function_feature(name: &NonEmpty<DbString>) -> Option<FeatureId> {
    if name.len() != 1 {
        return None;
    }
    match name.first().as_str().to_ascii_lowercase().as_str() {
        "element_id" => Some(FeatureId::G100),
        "abs" | "ceil" | "ceiling" | "floor" | "mod" | "sqrt" => Some(FeatureId::GF01),
        "acos" | "asin" | "atan" | "cos" | "cosh" | "cot" | "degrees" | "radians" | "sin"
        | "sinh" | "tan" | "tanh" => Some(FeatureId::GF02),
        "exp" | "ln" | "log" | "log10" | "power" => Some(FeatureId::GF03),
        "path_length" | "elements" => Some(FeatureId::GF04),
        "btrim" | "ltrim" | "rtrim" => Some(FeatureId::GF05),
        "cardinality" => Some(FeatureId::GF12),
        "size" => Some(FeatureId::GF13),
        "duration" | "duration_between" => Some(FeatureId::GV41),
        "uuid" | "uuid_v4" | "uuid_v7" => Some(FeatureId::IM_UUID),
        "json"
        | "json_parse"
        | "json_stringify"
        | "json_type"
        | "json_array"
        | "json_object"
        | "json_array_length"
        | "json_object_keys"
        | "json_contains"
        | "json_merge_patch"
        | "json_patch"
        | "json_get"
        | "json_get_text"
        | "json_get_scalar"
        | "json_get_path"
        | "json_get_path_text"
        | "json_get_path_scalar"
        | "json_has_path" => Some(FeatureId::IM_JSON),
        _ => None,
    }
}

fn is_list_value_function(name: &NonEmpty<DbString>, arity: usize) -> bool {
    if name.len() != 1 {
        return false;
    }
    let function_name = name.first().as_str();
    (arity == 2 && function_name.eq_ignore_ascii_case("trim"))
        || (arity == 1 && function_name.eq_ignore_ascii_case("elements"))
}

fn aggregate_function_feature(name: &NonEmpty<DbString>) -> Option<FeatureId> {
    if name.len() != 1 {
        return None;
    }
    match name.first().as_str().to_ascii_lowercase().as_str() {
        "stddev_pop" | "stddev_samp" | "collect_list" => Some(FeatureId::GF10),
        "percentile_cont" | "percentile_disc" => Some(FeatureId::GF11),
        _ => None,
    }
}

fn literal(value: &Literal, uses: &mut Vec<FeatureUse>) {
    match value {
        Literal::RadixInteger(_, span, kind) => {
            let feature_id = match kind {
                IntegerLiteralKind::Hexadecimal => FeatureId::GL01,
                IntegerLiteralKind::Octal => FeatureId::GL02,
                IntegerLiteralKind::Binary => FeatureId::GL03,
            };
            record_feature(uses, feature_id, *span);
        }
        Literal::Float(_, span) => record_feature(uses, FeatureId::GA01, *span),
        Literal::Uuid(_, span) => record_feature(uses, FeatureId::IM_UUID, *span),
        Literal::String(_, _)
        | Literal::Bytes(_, _)
        | Literal::Bool(_, _)
        | Literal::Integer(_, _)
        | Literal::ZonedDateTime(_, _)
        | Literal::LocalDateTime(_, _)
        | Literal::Date(_, _)
        | Literal::ZonedTime(_, _)
        | Literal::LocalTime(_, _)
        | Literal::Duration(_, _)
        | Literal::Null(_) => {}
    }
}

fn is_check(kind: &IsCheckKind, span: crate::SourceSpan, uses: &mut Vec<FeatureUse>) {
    match kind {
        IsCheckKind::Null | IsCheckKind::TruthValue(_) => {}
        IsCheckKind::Typed(ty) => {
            // ISO/IEC 39075:2024 §19.6 `<value type predicate>` is optional feature
            // GA06. The construct stamp is emitted before the target type features,
            // mirroring the `CAST` GA05 ordering above.
            record_feature(uses, FeatureId::GA06, span);
            gql_type(ty, span, uses);
        }
        IsCheckKind::Normalized(_) => {}
        IsCheckKind::Directed => record_feature(uses, FeatureId::G110, span),
        IsCheckKind::Labeled(_) => {
            record_feature(uses, FeatureId::G111, span);
        }
        IsCheckKind::SourceOf(value) => {
            record_feature(uses, FeatureId::G112, span);
            self::value(value, uses);
        }
        IsCheckKind::DestinationOf(value) => {
            record_feature(uses, FeatureId::G112, span);
            self::value(value, uses);
        }
    }
}

pub(crate) fn gql_type(ty: &GqlType, span: crate::SourceSpan, uses: &mut Vec<FeatureUse>) {
    match ty {
        GqlType::Uuid => record_feature(uses, FeatureId::IM_UUID, span),
        GqlType::Json => record_feature(uses, FeatureId::IM_JSON, span),
        GqlType::Vector => record_feature(uses, FeatureId::IM_VECTOR, span),
        GqlType::String | GqlType::Boolean | GqlType::Integer | GqlType::Float => {}
        GqlType::Uint8 => {
            record_feature(uses, FeatureId::GV01, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int8 => {
            record_feature(uses, FeatureId::GV02, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint16 => {
            record_feature(uses, FeatureId::GV03, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int16 => {
            record_feature(uses, FeatureId::GV04, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::SmallInt => {
            record_feature(uses, FeatureId::GV05, span);
            record_feature(uses, FeatureId::GV18, span);
        }
        GqlType::Uint32 => {
            record_feature(uses, FeatureId::GV06, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int32 => {
            record_feature(uses, FeatureId::GV07, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint64 => {
            record_feature(uses, FeatureId::GV08, span);
            record_feature(uses, FeatureId::GV11, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::BigInt => {
            record_feature(uses, FeatureId::GV10, span);
            record_feature(uses, FeatureId::GV19, span);
        }
        GqlType::Int64 => {
            record_feature(uses, FeatureId::GV12, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Uint128 => {
            record_feature(uses, FeatureId::GV13, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Int128 => {
            record_feature(uses, FeatureId::GV14, span);
            record_feature(uses, FeatureId::GV09, span);
        }
        GqlType::Decimal => record_feature(uses, FeatureId::GV17, span),
        GqlType::Float32 => record_feature(uses, FeatureId::GV21, span),
        GqlType::Float64 => record_feature(uses, FeatureId::GV24, span),
        GqlType::Bytes => {
            record_feature(uses, FeatureId::GV35, span);
        }
        GqlType::Date | GqlType::LocalDateTime | GqlType::LocalTime => {
            record_feature(uses, FeatureId::GV39, span);
        }
        GqlType::ZonedDateTime | GqlType::ZonedTime => {
            record_feature(uses, FeatureId::GV40, span);
        }
        GqlType::Duration => record_feature(uses, FeatureId::GV41, span),
        GqlType::Record(record) => {
            record_feature(uses, FeatureId::GV45, span);
            match record {
                RecordType::Open => record_feature(uses, FeatureId::GV47, span),
                RecordType::Closed(fields) => {
                    record_feature(uses, FeatureId::GV46, span);
                    for (_, ty) in fields {
                        if matches!(ty, GqlType::Record(_)) {
                            record_feature(uses, FeatureId::GV48, span);
                        }
                        gql_type(ty, span, uses);
                    }
                }
            }
        }
        GqlType::List(inner) => {
            record_feature(uses, FeatureId::GV50, span);
            gql_type(inner, span, uses);
        }
        GqlType::Path => record_feature(uses, FeatureId::GV55, span),
        GqlType::GraphRef => record_feature(uses, FeatureId::GV60, span),
        GqlType::TableRef => record_feature(uses, FeatureId::GV61, span),
        GqlType::NodeRef | GqlType::EdgeRef | GqlType::Null | GqlType::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use selene_core::feature_register::FeatureId;

    use crate::ast::ValueExpr;
    use crate::ast::expr::{BinaryOp, IsCheckKind, Literal};
    use crate::ast::span::SourceSpan;

    use super::{FeatureUse, value};

    fn span(offset: u32) -> SourceSpan {
        SourceSpan::new(offset, 1)
    }

    fn int(value: i64, offset: u32) -> ValueExpr {
        ValueExpr::Literal(Literal::Integer(value, span(offset)))
    }

    fn float(value: f64, offset: u32) -> ValueExpr {
        ValueExpr::Literal(Literal::Float(value, span(offset)))
    }

    fn ids(expr: &ValueExpr) -> Vec<FeatureId> {
        let mut uses = Vec::new();
        value(expr, &mut uses);
        uses.into_iter()
            .map(|feature: FeatureUse| feature.feature_id)
            .collect()
    }

    #[test]
    fn nested_child_features_record_transitively() {
        // `list[1 + 2]`: the parent `ListAccess` owns `IM_LIST_SUBSCRIPT`; the
        // `BinaryOp::Add` index child owns `GA01`. Both must surface, proving
        // the `for_each_child` delegation recurses into children.
        let expr = ValueExpr::ListAccess {
            target: ValueExpr::Variable {
                name: selene_core::db_string("list").expect("string fits DB string cap"),
                span: span(0),
            }
            .into(),
            index: ValueExpr::BinaryOp {
                op: BinaryOp::Add,
                lhs: int(1, 5).into(),
                rhs: int(2, 7).into(),
                span: span(5),
            }
            .into(),
            span: span(0),
        };
        let observed = ids(&expr);
        assert!(
            observed.contains(&FeatureId::IM_LIST_SUBSCRIPT),
            "parent ListAccess feature missing: {observed:?}"
        );
        assert!(
            observed.contains(&FeatureId::GA01),
            "nested BinaryOp arithmetic feature missing transitively: {observed:?}"
        );
    }

    #[test]
    fn binary_op_records_parent_before_children() {
        // Ordering is observable: `flag()` reports the *first* unsupported
        // feature. The parent's `GA01` must precede the child float literal's
        // `GA01`, matching the pre-refactor recursion order.
        let expr = ValueExpr::BinaryOp {
            op: BinaryOp::Mul,
            lhs: float(1.5, 3).into(),
            rhs: int(2, 9).into(),
            span: span(0),
        };
        let mut uses = Vec::new();
        value(&expr, &mut uses);
        // First recorded span is the parent BinaryOp span (offset 0), then the
        // child float literal span (offset 3) — both `GA01`.
        assert_eq!(uses[0].feature_id, FeatureId::GA01);
        assert_eq!(uses[0].span.byte_offset, 0);
        assert_eq!(uses[1].feature_id, FeatureId::GA01);
        assert_eq!(uses[1].span.byte_offset, 3);
    }

    #[test]
    fn is_check_source_of_orders_operand_then_kind_then_value() {
        // `(p :: INT8) IS SOURCE OF (e :: INT8)`: the operand subtree's
        // `IM_TYPED_PARAMS`, then the `IS SOURCE OF` `G112`, then the value
        // subtree's `IM_TYPED_PARAMS` must appear in that exact order — the
        // ordering `for_each_child` alone cannot express, hence the explicit
        // `IsCheck` arm.
        let typed_param = |name: &str, offset: u32| ValueExpr::Parameter {
            name: selene_core::db_string(name).expect("string fits DB string cap"),
            declared_type: Some(crate::ast::types::GqlType::Int8),
            span: span(offset),
        };
        let expr = ValueExpr::IsCheck {
            operand: typed_param("p", 1).into(),
            kind: IsCheckKind::SourceOf(typed_param("e", 20).into()),
            negated: false,
            span: span(10),
        };
        let mut uses = Vec::new();
        value(&expr, &mut uses);
        let trace: Vec<(FeatureId, u32)> = uses
            .iter()
            .map(|feature| (feature.feature_id, feature.span.byte_offset))
            .collect();
        // operand: GE04/GE05/IM_TYPED_PARAMS @1, then G112 @10, then value:
        // GE04/GE05/IM_TYPED_PARAMS @20.
        assert_eq!(
            trace,
            vec![
                (FeatureId::GE04, 1),
                (FeatureId::GE05, 1),
                (FeatureId::IM_TYPED_PARAMS, 1),
                (FeatureId::G112, 10),
                (FeatureId::GE04, 20),
                (FeatureId::GE05, 20),
                (FeatureId::IM_TYPED_PARAMS, 20),
            ]
        );
    }

    #[test]
    fn cast_records_feature_then_value_then_target_type() {
        // `CAST(1.5 AS INT8)`: `GA05` (Cast specification, ISO Annex A item 52),
        // then the value subtree's `GA01` (float literal), then the target-type
        // walk's `GV02`/`GV09`. The Cast node's own feature is recorded first and
        // the type tail runs *after* the `for_each_child` value recursion.
        let expr = ValueExpr::Cast {
            value: float(1.5, 5).into(),
            target_type: crate::ast::types::GqlType::Int8.into(),
            span: span(0),
        };
        let observed = ids(&expr);
        assert_eq!(observed.first(), Some(&FeatureId::GA05));
        let ga05 = observed.iter().position(|id| *id == FeatureId::GA05);
        let ga01 = observed.iter().position(|id| *id == FeatureId::GA01);
        let gv02 = observed.iter().position(|id| *id == FeatureId::GV02);
        assert!(
            ga05 < ga01 && ga01 < gv02,
            "expected GA05 < value(GA01) < target-type(GV02): {observed:?}"
        );
    }
}
