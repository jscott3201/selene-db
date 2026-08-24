//! Database-catalog DDL parser contract (ISO/IEC 39075:2024 §12.2–§12.5).
//!
//! Every accepted form, every structured rejection with its exact GQLSTATUS
//! and feature ID, identifier forms, keyword boundaries, and full-statement
//! spans are pinned here so reviewers can find them in one place.

use selene_gql::{
    CatalogObjectReference, DdlStatement, GqlStatus, IdentifierForm, ParserError, SourceSpan,
    Statement, feature_walk, parse,
};
use selene_profile::FeatureId;

fn parse_ddl(source: &str) -> DdlStatement {
    match parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}")) {
        Statement::Ddl(statement) => statement,
        other => panic!("{source}: expected DDL, got {other:?}"),
    }
}

fn segments(reference: &CatalogObjectReference) -> Vec<(&str, IdentifierForm)> {
    reference
        .segments
        .iter()
        .map(|segment| (segment.name.as_str(), segment.form))
        .collect()
}

fn full_span(source: &str) -> SourceSpan {
    SourceSpan::new(0, source.len() as u32)
}

fn features(source: &str) -> Vec<FeatureId> {
    feature_walk(&parse(source).expect(source))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect()
}

fn assert_unsupported(source: &str, expected: FeatureId) {
    let error = parse(source).expect_err(source);
    assert_eq!(
        error.gqlstatus(),
        GqlStatus::FEATURE_NOT_SUPPORTED,
        "{source}"
    );
    let ParserError::UnsupportedFeature {
        feature_id,
        display_name,
        ..
    } = error
    else {
        panic!("{source}: expected UnsupportedFeature, got {error:?}");
    };
    assert_eq!(feature_id, expected, "{source}");
    assert_eq!(
        display_name,
        selene_profile::capability(expected)
            .expect("generated capability")
            .name,
        "{source}"
    );
}

fn assert_not_implemented(source: &str, fragment: &str) -> ParserError {
    let error = parse(source).expect_err(source);
    assert_eq!(
        error.gqlstatus(),
        GqlStatus::FEATURE_NOT_SUPPORTED,
        "{source}"
    );
    let ParserError::NotImplemented { message, .. } = &error else {
        panic!("{source}: expected NotImplemented, got {error:?}");
    };
    assert!(
        message.contains(fragment),
        "{source}: message {message:?} lacks {fragment:?}"
    );
    error
}

fn assert_syntax(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source}: expected SyntaxError, got {error:?}"
    );
    assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
}

#[test]
fn create_schema_requires_an_absolute_reference_and_tags_forms() {
    let source = "CREATE SCHEMA /memory";
    let DdlStatement::CreateSchema {
        reference,
        if_not_exists,
        span,
    } = parse_ddl(source)
    else {
        panic!("expected CREATE SCHEMA");
    };
    assert!(reference.absolute);
    assert_eq!(segments(&reference), [("memory", IdentifierForm::Regular)]);
    assert_eq!(reference.span, SourceSpan::new(14, 7));
    assert!(!if_not_exists);
    assert_eq!(span, full_span(source));

    let source = "CREATE SCHEMA IF NOT EXISTS /`my schema`";
    let DdlStatement::CreateSchema {
        reference,
        if_not_exists,
        span,
    } = parse_ddl(source)
    else {
        panic!("expected CREATE SCHEMA");
    };
    assert!(if_not_exists);
    assert_eq!(
        segments(&reference),
        [("my schema", IdentifierForm::Delimited)]
    );
    assert_eq!(span, full_span(source));

    // Directory depth is a facade decision (42002), not a parse error.
    let DdlStatement::CreateSchema { reference, .. } = parse_ddl("CREATE SCHEMA /a/b") else {
        panic!("expected CREATE SCHEMA");
    };
    assert_eq!(
        segments(&reference),
        [
            ("a", IdentifierForm::Regular),
            ("b", IdentifierForm::Regular)
        ]
    );

    for source in [
        "CREATE SCHEMA memory",
        "CREATE SCHEMA",
        "CREATE SCHEMA /",
        "CREATE SCHEMA / memory",
        "CREATE SCHEMA /a /b",
        "CREATE SCHEMA IF EXISTS /memory",
        "CREATE SCHEMA /CURRENT_SCHEMA",
        "CREATE SCHEMA /HOME_SCHEMA",
        "CREATE SCHEMA /.",
        "CREATE SCHEMA /..",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn drop_schema_mirrors_create_schema_forms() {
    let source = "DROP SCHEMA IF EXISTS /memory";
    let DdlStatement::DropSchema {
        reference,
        if_exists,
        span,
    } = parse_ddl(source)
    else {
        panic!("expected DROP SCHEMA");
    };
    assert!(reference.absolute);
    assert!(if_exists);
    assert_eq!(segments(&reference), [("memory", IdentifierForm::Regular)]);
    assert_eq!(span, full_span(source));

    let source = "DROP SCHEMA /\"a/b\"";
    let DdlStatement::DropSchema {
        if_exists, span, ..
    } = parse_ddl(source)
    else {
        panic!("expected DROP SCHEMA");
    };
    assert!(!if_exists);
    assert_eq!(span, full_span(source));

    for source in [
        "DROP SCHEMA memory",
        "DROP SCHEMA IF NOT EXISTS /memory",
        "DROP SCHEMA",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn create_graph_accepts_every_open_graph_type_spelling() {
    for source in [
        "CREATE GRAPH g ANY",
        "CREATE GRAPH g TYPED ANY",
        "CREATE GRAPH g :: ANY",
        "CREATE GRAPH g ::ANY",
        "CREATE GRAPH g ANY GRAPH",
        "CREATE GRAPH g ANY PROPERTY GRAPH",
        "CREATE GRAPH g TYPED ANY PROPERTY GRAPH",
        "CREATE PROPERTY GRAPH g ANY",
        "CREATE PROPERTY GRAPH IF NOT EXISTS g ANY PROPERTY GRAPH",
        "create graph g any",
    ] {
        let DdlStatement::CreateGraph {
            reference,
            or_replace,
            if_not_exists,
            span,
        } = parse_ddl(source)
        else {
            panic!("{source}: expected CREATE GRAPH");
        };
        assert!(!reference.absolute, "{source}");
        assert_eq!(segments(&reference), [("g", IdentifierForm::Regular)]);
        assert!(!or_replace, "{source}");
        assert_eq!(if_not_exists, source.contains("IF NOT EXISTS"), "{source}");
        assert_eq!(span, full_span(source), "{source}");
    }
}

#[test]
fn create_graph_references_are_absolute_or_current_schema_relative() {
    let DdlStatement::CreateGraph { reference, .. } =
        parse_ddl("CREATE GRAPH /memory/episodes ANY")
    else {
        panic!("expected CREATE GRAPH");
    };
    assert!(reference.absolute);
    assert_eq!(
        segments(&reference),
        [
            ("memory", IdentifierForm::Regular),
            ("episodes", IdentifierForm::Regular)
        ]
    );
    assert_eq!(reference.span, SourceSpan::new(13, 16));

    let DdlStatement::CreateGraph { reference, .. } =
        parse_ddl("CREATE GRAPH /\"a/b\"/`back``tick` ANY")
    else {
        panic!("expected CREATE GRAPH");
    };
    assert_eq!(
        segments(&reference),
        [
            ("a/b", IdentifierForm::Delimited),
            ("back`tick", IdentifierForm::Delimited)
        ]
    );

    let DdlStatement::CreateGraph { reference, .. } = parse_ddl("CREATE GRAPH `my graph` ANY")
    else {
        panic!("expected CREATE GRAPH");
    };
    assert!(!reference.absolute);
    assert_eq!(
        segments(&reference),
        [("my graph", IdentifierForm::Delimited)]
    );

    // One- and three-segment absolute references parse; the facade rejects
    // them as invalid references (42002).
    for (source, expected) in [("CREATE GRAPH /g ANY", 1), ("CREATE GRAPH /a/b/g ANY", 3)] {
        let DdlStatement::CreateGraph { reference, .. } = parse_ddl(source) else {
            panic!("{source}: expected CREATE GRAPH");
        };
        assert!(reference.absolute);
        assert_eq!(reference.segments.len(), expected, "{source}");
    }
}

#[test]
fn create_graph_without_a_type_clause_is_a_syntax_error() {
    for source in [
        "CREATE GRAPH g",
        "CREATE GRAPH IF NOT EXISTS g",
        "CREATE PROPERTY GRAPH /s/g",
    ] {
        let error = parse(source).expect_err(source);
        let ParserError::SyntaxError { message, span, .. } = &error else {
            panic!("{source}: expected SyntaxError, got {error:?}");
        };
        assert!(message.contains("section 12.4"), "{source}: {message}");
        assert_eq!(*span, full_span(source), "{source}");
        assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR);
    }
}

#[test]
fn create_graph_unsupported_clauses_are_rejected_with_their_feature() {
    assert_unsupported("CREATE GRAPH g LIKE h", FeatureId::GG04);
    assert_unsupported("CREATE GRAPH /s/g LIKE /s/h", FeatureId::GG04);
    assert_unsupported("CREATE GRAPH g {(Person :Person)}", FeatureId::GG03);
    assert_unsupported(
        "CREATE GRAPH g ::{(Person :Person {name STRING})}",
        FeatureId::GG03,
    );
    assert_unsupported(
        "CREATE GRAPH g TYPED PROPERTY GRAPH {(City :City)}",
        FeatureId::GG03,
    );
    assert_unsupported("CREATE GRAPH g ANY AS COPY OF h", FeatureId::GG05);
    assert_unsupported("CREATE GRAPH g AS COPY OF h", FeatureId::GG05);
    assert_unsupported("CREATE GRAPH g TYPED t AS COPY OF h", FeatureId::GG05);
    assert_unsupported("CREATE GRAPH g LIKE h AS COPY OF h", FeatureId::GG05);
    for source in [
        "CREATE GRAPH g TYPED t",
        "CREATE GRAPH g ::t",
        "CREATE GRAPH g :: /s/t",
        "CREATE GRAPH g t",
        "CREATE GRAPH g /s/t",
    ] {
        assert_not_implemented(source, "GG02");
    }
}

#[test]
fn unsupported_clause_spans_point_at_the_clause() {
    let error = parse("CREATE GRAPH g ANY AS COPY OF h").expect_err("copy rejected");
    let ParserError::UnsupportedFeature { span, .. } = error else {
        panic!("expected UnsupportedFeature");
    };
    assert_eq!(span, SourceSpan::new(19, 12));
    let error = parse("CREATE GRAPH g LIKE h").expect_err("like rejected");
    let ParserError::UnsupportedFeature { span, .. } = error else {
        panic!("expected UnsupportedFeature");
    };
    assert_eq!(span, SourceSpan::new(15, 6));
}

#[test]
fn or_replace_keeps_its_not_implemented_rejection() {
    for source in [
        "CREATE OR REPLACE GRAPH g ANY",
        "CREATE OR REPLACE PROPERTY GRAPH g ANY",
    ] {
        let error = assert_not_implemented(source, "OR REPLACE");
        let ParserError::NotImplemented { span, .. } = error else {
            unreachable!()
        };
        assert_eq!(span, full_span(source), "{source}");
    }
    // OR REPLACE and IF NOT EXISTS are alternatives in ISO section 12.4.
    assert_syntax("CREATE OR REPLACE GRAPH IF NOT EXISTS g ANY");
}

#[test]
fn drop_graph_accepts_property_if_exists_and_both_reference_forms() {
    let source = "DROP PROPERTY GRAPH IF EXISTS /memory/episodes";
    let DdlStatement::DropGraph {
        reference,
        if_exists,
        span,
    } = parse_ddl(source)
    else {
        panic!("expected DROP GRAPH");
    };
    assert!(reference.absolute);
    assert!(if_exists);
    assert_eq!(
        segments(&reference),
        [
            ("memory", IdentifierForm::Regular),
            ("episodes", IdentifierForm::Regular)
        ]
    );
    assert_eq!(span, full_span(source));

    let DdlStatement::DropGraph {
        reference,
        if_exists,
        ..
    } = parse_ddl("DROP GRAPH `default`")
    else {
        panic!("expected DROP GRAPH");
    };
    assert!(!reference.absolute);
    assert!(!if_exists);
    assert_eq!(
        segments(&reference),
        [("default", IdentifierForm::Delimited)]
    );

    for source in [
        "DROP GRAPH",
        "DROP GRAPH g ANY",
        "DROP GRAPH IF NOT EXISTS g",
        "DROP GRAPH CURRENT_SCHEMA",
        "DROP GRAPH HOME_SCHEMA",
        "DROP GRAPH .",
        "DROP GRAPH ..",
        "DROP GRAPH ./g",
        "DROP GRAPH ../s/g",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn graph_type_statements_are_guarded_until_part_two() {
    for source in [
        "CREATE GRAPH TYPE t {(Person :Person)}",
        "CREATE PROPERTY GRAPH TYPE IF NOT EXISTS /s/t AS COPY OF u",
        "CREATE OR REPLACE GRAPH TYPE t LIKE g",
        "CREATE GRAPH TYPE",
        "DROP GRAPH TYPE t",
        "DROP PROPERTY GRAPH TYPE IF EXISTS /s/t",
    ] {
        let error = assert_not_implemented(source, "GRAPH TYPE");
        let ParserError::NotImplemented { span, hint, .. } = error else {
            unreachable!()
        };
        assert_eq!(span, full_span(source), "{source}");
        assert!(
            hint.as_deref().is_some_and(|hint| hint.contains("part 2")),
            "{source}"
        );
    }
    // TYPE is not reserved: a graph named `types` is still a graph.
    let DdlStatement::CreateGraph { reference, .. } = parse_ddl("CREATE GRAPH types ANY") else {
        panic!("expected CREATE GRAPH");
    };
    assert_eq!(segments(&reference), [("types", IdentifierForm::Regular)]);
    let DdlStatement::DropGraph { reference, .. } = parse_ddl("DROP GRAPH `type`") else {
        panic!("expected DROP GRAPH");
    };
    assert_eq!(segments(&reference), [("type", IdentifierForm::Delimited)]);
}

#[test]
fn next_chained_catalog_statements_are_rejected_as_not_implemented() {
    for source in [
        "CREATE SCHEMA /a NEXT CREATE SCHEMA /b",
        "CREATE SCHEMA /a NEXT CREATE GRAPH /a/g ANY",
        "DROP GRAPH g NEXT DROP SCHEMA /a",
        "CREATE GRAPH g ANY NEXT DROP GRAPH g NEXT DROP GRAPH g",
    ] {
        let error = assert_not_implemented(source, "section 12.1");
        let ParserError::NotImplemented { span, .. } = error else {
            unreachable!()
        };
        assert_eq!(span, full_span(source), "{source}");
    }
    for source in [
        "CREATE SCHEMA /a NEXTCREATE SCHEMA /b",
        "CREATE SCHEMA /a NEXTx CREATE SCHEMA /b",
        "CREATE SCHEMA /a NEXT",
        "CREATE SCHEMA /a NEXT RETURN 1",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn catalog_keywords_require_word_boundaries() {
    for source in [
        "CREATE SCHEMAX /a",
        "CREATE SCHEMA IFNOT EXISTS /a",
        "DROP SCHEMAX /a",
        "CREATE GRAPHX g ANY",
        "CREATE GRAPH g ANY PROPERTYX GRAPH",
        "CREATE GRAPH g ANY PROPERTY GRAPHX",
        "CREATE PROPERTYX GRAPH g ANY",
        "DROP PROPERTYX GRAPH g",
        "DROP GRAPHX g",
        "DROP GRAPH IFEXISTS g",
    ] {
        assert_syntax(source);
    }
    // `ANYX`, `TYPEDANY`, and `TYPEX` are identifiers: the first two read as a
    // closed graph type reference and the third names a graph whose type
    // clause `t` is a reference (GG02, not implemented), never as keywords.
    for source in [
        "CREATE GRAPH g ANYX",
        "CREATE GRAPH g TYPEDANY",
        "CREATE GRAPH TYPEX t",
    ] {
        assert_not_implemented(source, "GG02");
    }
}

#[test]
fn flagger_stamps_iso_catalog_features_and_the_drop_graph_bridge_flag() {
    assert_eq!(features("CREATE SCHEMA /a"), [FeatureId::GC01]);
    assert_eq!(
        features("CREATE SCHEMA IF NOT EXISTS /a"),
        [FeatureId::GC01, FeatureId::GC02]
    );
    assert_eq!(features("DROP SCHEMA /a"), [FeatureId::GC01]);
    assert_eq!(
        features("DROP SCHEMA IF EXISTS /a"),
        [FeatureId::GC01, FeatureId::GC02]
    );
    assert_eq!(
        features("CREATE GRAPH g ANY"),
        [FeatureId::GC04, FeatureId::GG01]
    );
    assert_eq!(
        features("CREATE GRAPH IF NOT EXISTS g ANY"),
        [FeatureId::GC04, FeatureId::GG01, FeatureId::GC05]
    );
    // DROP GRAPH also stamps the IM_DROP_GRAPH bridge flag until M02-PR05
    // deletes the bootstrap factory reset.
    assert_eq!(
        features("DROP GRAPH g"),
        [FeatureId::GC04, FeatureId::IM_DROP_GRAPH]
    );
    assert_eq!(
        features("DROP GRAPH IF EXISTS g"),
        [FeatureId::GC04, FeatureId::GC05, FeatureId::IM_DROP_GRAPH]
    );
}

#[test]
fn explain_wraps_catalog_ddl_without_executing_it() {
    let statement = parse("EXPLAIN CREATE GRAPH g ANY").expect("EXPLAIN parses");
    assert!(matches!(statement, Statement::Explain { .. }));
}
