//! ISO record type form coverage.

use selene_core::feature_register::FeatureId;
use selene_gql::{
    GqlType, IsCheckKind, PipelineStatement, RecordType, Statement, ValueExpr,
    ast::format_read_statement, feature_walk, parse,
};

#[test]
fn iso_record_type_forms_parse_to_open_or_closed_ast() {
    assert!(matches!(
        typed_type("RETURN NULL IS TYPED RECORD AS ok"),
        GqlType::Record(RecordType::Open)
    ));
    assert!(matches!(
        typed_type("RETURN NULL IS TYPED ANY RECORD AS ok"),
        GqlType::Record(RecordType::Open)
    ));
    assert!(matches!(
        typed_type("RETURN NULL IS TYPED RECORD{} AS ok"),
        GqlType::Record(RecordType::Closed(fields)) if fields.is_empty()
    ));
    assert!(matches!(
        typed_type("RETURN NULL IS TYPED {} AS ok"),
        GqlType::Record(RecordType::Closed(fields)) if fields.is_empty()
    ));

    let GqlType::Record(RecordType::Closed(fields)) =
        typed_type("RETURN NULL IS TYPED {a :: INT, b :: STRING} AS ok")
    else {
        panic!("bare field-types specification must build a closed RECORD type");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0.as_str(), "a");
    assert_eq!(fields[0].1, GqlType::Integer);
    assert_eq!(fields[1].0.as_str(), "b");
    assert_eq!(fields[1].1, GqlType::String);
}

#[test]
fn iso_record_type_forms_format_to_canonical_record_names() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY RECORD AS ok",
            "RETURN null IS TYPED RECORD AS ok",
        ),
        (
            "RETURN NULL IS TYPED {} AS ok",
            "RETURN null IS TYPED RECORD{} AS ok",
        ),
        (
            "RETURN NULL IS TYPED {a :: INT} AS ok",
            "RETURN null IS TYPED RECORD{a :: INTEGER} AS ok",
        ),
    ] {
        let statement =
            parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
        let formatted = format_read_statement(&statement).expect("statement formats");
        assert_eq!(formatted, expected);
        parse(&formatted).unwrap_or_else(|error| panic!("{formatted} should reparse: {error:?}"));
    }
}

#[test]
fn record_type_forms_stamp_open_and_closed_record_features() {
    let open = parse("RETURN NULL IS TYPED ANY RECORD AS ok").expect("open record parses");
    let open_features = feature_walk(&open)
        .into_iter()
        .map(|use_| use_.feature_id)
        .collect::<Vec<_>>();
    assert!(
        open_features.contains(&FeatureId::GA06),
        "{open_features:?}"
    );
    assert!(
        open_features.contains(&FeatureId::GV45),
        "{open_features:?}"
    );
    assert!(
        open_features.contains(&FeatureId::GV47),
        "{open_features:?}"
    );

    let closed = parse("RETURN NULL IS TYPED {} AS ok").expect("closed unit record parses");
    let closed_features = feature_walk(&closed)
        .into_iter()
        .map(|use_| use_.feature_id)
        .collect::<Vec<_>>();
    assert!(
        closed_features.contains(&FeatureId::GA06),
        "{closed_features:?}"
    );
    assert!(
        closed_features.contains(&FeatureId::GV45),
        "{closed_features:?}"
    );
    assert!(
        closed_features.contains(&FeatureId::GV46),
        "{closed_features:?}"
    );
}

fn typed_type(source: &str) -> GqlType {
    let statement =
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    let Statement::Query(pipeline) = statement else {
        panic!("{source} should parse as a query");
    };
    let PipelineStatement::Return(return_clause) = &pipeline.statements[0] else {
        panic!("{source} should parse as RETURN");
    };
    let ValueExpr::IsCheck {
        kind: IsCheckKind::Typed(ty),
        ..
    } = &return_clause.items[0].expr
    else {
        panic!("{source} should parse as IS TYPED");
    };
    ty.clone()
}
