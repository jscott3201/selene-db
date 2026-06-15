use super::*;

#[test]
fn parse_binary_expression_precedence() {
    let item = only_item("RETURN 1 + 2 * 3");
    let ValueExpr::BinaryOp {
        op: BinaryOp::Add,
        rhs,
        ..
    } = &item.expr
    else {
        panic!("expected addition");
    };
    assert!(matches!(
        **rhs,
        ValueExpr::BinaryOp {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn parse_function_aggregate_star_and_distinct() {
    let count_star = only_item("RETURN count(*)").expr;
    assert!(matches!(
        count_star,
        ValueExpr::FunctionCall {
            star: true,
            distinct: false,
            ref args,
            ..
        } if args.is_empty()
    ));

    let count_distinct = only_item("RETURN count(DISTINCT n)").expr;
    assert!(matches!(
        count_distinct,
        ValueExpr::FunctionCall {
            star: false,
            distinct: true,
            ref args,
            ..
        } if args.len() == 1
    ));

    let percentile = only_item("RETURN percentile_cont(n, 0.5)").expr;
    assert!(matches!(
        percentile,
        ValueExpr::FunctionCall {
            ref name,
            star: false,
            distinct: false,
            ref args,
            ..
        } if name.len() == 1 && name.first().as_str() == "percentile_cont" && args.len() == 2
    ));

    assert!(matches!(
        only_item("RETURN percentile_cont(DISTINCT n, 0.5)").expr,
        ValueExpr::FunctionCall {
            ref name,
            star: false,
            distinct: true,
            ref args,
            ..
        } if name.len() == 1 && name.first().as_str() == "percentile_cont" && args.len() == 2
    ));
}

#[test]
fn parse_current_datetime_keyword_functions() {
    assert_function_call("RETURN CURRENT_DATE", "current_date");
    assert_function_call("RETURN CURRENT_TIME", "current_time");
    assert_function_call("RETURN CURRENT_TIMESTAMP", "current_timestamp");
    assert_function_call("RETURN LOCAL_TIMESTAMP", "local_datetime");
    assert_function_call("RETURN LOCAL_TIME", "local_time");
    assert_function_call("RETURN LOCAL_TIME()", "local_time");

    assert_function_call("RETURN local_datetime()", "local_datetime");
    assert_function_call("RETURN local_time()", "local_time");
    assert_function_call_with_args("RETURN LOCAL_TIME('12:34:56')", "local_time", 1);

    for source in [
        "RETURN CURRENT_DATE()",
        "RETURN CURRENT_TIME()",
        "RETURN CURRENT_TIMESTAMP()",
    ] {
        assert!(parse(source).is_err(), "{source} must reject parentheses");
    }
}

#[test]
fn parse_list_record_and_case_expressions() {
    assert!(matches!(
        only_item("RETURN [1, 2]").expr,
        ValueExpr::ListLiteral { ref items, .. } if items.len() == 2
    ));
    assert!(matches!(
        only_item("RETURN {name: 'Alice'}").expr,
        ValueExpr::RecordLiteral { ref fields, .. } if fields.len() == 1
    ));
    assert!(matches!(
        only_item("RETURN CASE WHEN true THEN 1 ELSE 0 END").expr,
        ValueExpr::Case { ref branches, else_branch: Some(_), .. } if branches.len() == 1
    ));
}

#[test]
fn parse_predicate_expression_family() {
    assert!(matches!(
        only_item("RETURN n IS NOT NULL").expr,
        ValueExpr::IsCheck { negated: true, .. }
    ));
    assert!(matches!(
        only_item("RETURN n.name STARTS WITH 'A'").expr,
        ValueExpr::BinaryOp {
            op: BinaryOp::StartsWith,
            ..
        }
    ));
    assert!(matches!(
        only_item("RETURN PROPERTY_EXISTS(n, 'name')").expr,
        ValueExpr::PropertyExists { .. }
    ));
}

#[test]
fn binding_table_reference_type_preserves_field_types_before_feature_gate() {
    let item = only_unflagged_item(
        "RETURN NULL IS TYPED BINDING TABLE { id :: INT, payload :: RECORD { ok :: BOOL } } AS ok",
    );
    let ValueExpr::IsCheck {
        kind: IsCheckKind::Typed(GqlType::TableRef(BindingTableType::Closed(fields))),
        ..
    } = &item.expr
    else {
        panic!("expected typed predicate over closed binding table reference type");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0.as_str(), "id");
    assert_eq!(fields[0].1, GqlType::Integer);
    assert_eq!(fields[1].0.as_str(), "payload");
    assert!(matches!(fields[1].1, GqlType::Record(_)));
}

#[test]
fn qualified_function_names_preserve_segment_boundaries() {
    // `foo."bar.baz"` and `foo.bar.baz` would collide if the AST stored
    // the qualified name as a single dotted string. The Vec<DbString> path
    // keeps them distinguishable so namespaced procedure calls resolve
    // to the right thing.
    let bare = only_item("RETURN foo.bar.baz()").expr;
    let ValueExpr::FunctionCall {
        name: name_three, ..
    } = &bare
    else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_three.len(), 3);

    let quoted = only_item("RETURN foo.\"bar.baz\"()").expr;
    let ValueExpr::FunctionCall { name: name_two, .. } = &quoted else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_two.len(), 2);

    // Single-segment bare name is still one segment.
    let single = only_item("RETURN count(*)").expr;
    let ValueExpr::FunctionCall { name: name_one, .. } = &single else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_one.len(), 1);
}
