//! Strict graph-type catalog DDL parser contract.

use selene_gql::{
    CatalogObjectReference, DatabaseCatalogCommand, DdlStatement, GqlStatus, IdentifierForm,
    ParserError, SourceSpan, Statement, feature_walk, parse,
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

fn features(source: &str) -> Vec<FeatureId> {
    feature_walk(&parse(source).expect(source))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect()
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
    assert!(message.contains(fragment), "{source}: {message}");
    error
}

fn assert_syntax(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source}: {error:?}"
    );
    assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
}

#[test]
fn create_graph_type_accepts_the_bounded_iso_node_forms() {
    for source in [
        "CREATE GRAPH TYPE t { NODE TYPE Person () }",
        "CREATE GRAPH TYPE t AS { NODE Person () }",
        "CREATE PROPERTY GRAPH TYPE t { VERTEX TYPE Person () }",
        "CREATE GRAPH TYPE t { NODE TYPE Person }",
        "CREATE GRAPH TYPE t { VERTEX Person }",
        "create graph type t { node type Person () }",
    ] {
        let DdlStatement::CreateGraphType {
            reference,
            definition,
            or_replace,
            if_not_exists,
            span,
        } = parse_ddl(source)
        else {
            panic!("{source}: expected CREATE GRAPH TYPE");
        };
        assert_eq!(
            segments(&reference),
            [("t", IdentifierForm::Regular)],
            "{source}"
        );
        assert_eq!(definition.node_types.len(), 1, "{source}");
        assert_eq!(
            definition.node_types[0].name.name.as_str(),
            "Person",
            "{source}"
        );
        assert_eq!(
            definition.node_types[0].name.form,
            IdentifierForm::Regular,
            "{source}"
        );
        assert!(!or_replace, "{source}");
        assert!(!if_not_exists, "{source}");
        assert_eq!(span, SourceSpan::new(0, source.len() as u32), "{source}");
    }
}

#[test]
fn create_graph_type_preserves_paths_forms_order_and_spans() {
    let source = "CREATE PROPERTY GRAPH TYPE IF NOT EXISTS /`my schema`/`type name` AS { NODE TYPE Person (), VERTEX TYPE `Memory Type` }";
    let statement = parse_ddl(source);
    let DdlStatement::CreateGraphType {
        reference,
        definition,
        or_replace,
        if_not_exists,
        span,
    } = &statement
    else {
        panic!("expected CREATE GRAPH TYPE");
    };
    assert!(reference.absolute);
    assert_eq!(
        segments(reference),
        [
            ("my schema", IdentifierForm::Delimited),
            ("type name", IdentifierForm::Delimited)
        ]
    );
    assert!(!or_replace);
    assert!(*if_not_exists);
    assert_eq!(*span, SourceSpan::new(0, source.len() as u32));
    assert_eq!(definition.node_types.len(), 2);
    assert_eq!(definition.node_types[0].name.name.as_str(), "Person");
    assert_eq!(definition.node_types[1].name.name.as_str(), "Memory Type");
    assert_eq!(
        definition.node_types[1].name.form,
        IdentifierForm::Delimited
    );
    assert_eq!(
        &source[definition.node_types[1].span.byte_offset as usize
            ..definition.node_types[1].span.end() as usize],
        "VERTEX TYPE `Memory Type`"
    );
    let Some(DatabaseCatalogCommand::CreateGraphType {
        reference: command_reference,
        definition: command_definition,
        if_not_exists: true,
        or_replace: false,
        ..
    }) = DatabaseCatalogCommand::from_ddl(&statement)
    else {
        panic!("CREATE GRAPH TYPE must lower to a command");
    };
    assert_eq!(command_reference, *reference);
    assert_eq!(command_definition, *definition);
}

#[test]
fn graph_type_replace_and_conditional_formats_are_distinct() {
    let source = "CREATE OR REPLACE PROPERTY GRAPH TYPE t { NODE TYPE Person () }";
    let DdlStatement::CreateGraphType {
        or_replace,
        if_not_exists,
        ..
    } = parse_ddl(source)
    else {
        panic!("expected CREATE GRAPH TYPE");
    };
    assert!(or_replace);
    assert!(!if_not_exists);

    for source in [
        "CREATE OR REPLACE GRAPH TYPE IF NOT EXISTS t { NODE TYPE Person () }",
        "CREATE GRAPH TYPE IF NOT EXISTS OR REPLACE t { NODE TYPE Person () }",
        "CREATE OR GRAPH TYPE t { NODE TYPE Person () }",
        "CREATE REPLACE GRAPH TYPE t { NODE TYPE Person () }",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn drop_graph_type_accepts_property_conditional_and_reference_forms() {
    for source in [
        "DROP GRAPH TYPE t",
        "DROP PROPERTY GRAPH TYPE IF EXISTS /s/t",
        "drop graph type `type name`",
    ] {
        let DdlStatement::DropGraphType {
            reference,
            if_exists,
            span,
        } = parse_ddl(source)
        else {
            panic!("{source}: expected DROP GRAPH TYPE");
        };
        assert_eq!(
            if_exists,
            source.to_ascii_uppercase().contains("IF EXISTS"),
            "{source}"
        );
        assert_eq!(span, SourceSpan::new(0, source.len() as u32), "{source}");
        assert!(!reference.segments.is_empty());
    }
    for source in [
        "DROP GRAPH TYPE",
        "DROP GRAPH TYPE IF NOT EXISTS t",
        "DROP GRAPH TYPE t RESTRICT",
        "DROP GRAPH TYPE t { NODE TYPE Person () }",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn named_graph_type_binding_reaches_create_graph_without_gg01() {
    for source in [
        "CREATE GRAPH g t",
        "CREATE GRAPH g TYPED t",
        "CREATE GRAPH g ::t",
        "CREATE GRAPH /s/g TYPED /s/t",
        "CREATE OR REPLACE PROPERTY GRAPH /s/g :: /s/`type name`",
    ] {
        let statement = parse_ddl(source);
        let DdlStatement::CreateGraph {
            graph_type: Some(graph_type),
            ..
        } = &statement
        else {
            panic!("{source}: expected a named graph type");
        };
        assert!(!graph_type.segments.is_empty(), "{source}");
        assert!(!features(source).contains(&FeatureId::GG01), "{source}");
        assert!(features(source).contains(&FeatureId::GG02), "{source}");
        let Some(DatabaseCatalogCommand::CreateGraph {
            graph_type: Some(command_type),
            ..
        }) = DatabaseCatalogCommand::from_ddl(&statement)
        else {
            panic!("{source}: command lost the graph type");
        };
        assert_eq!(command_type, *graph_type, "{source}");
    }
}

#[test]
fn flagger_records_only_features_written_by_the_catalog_forms() {
    assert_eq!(
        features("CREATE GRAPH TYPE t { NODE TYPE Person () }"),
        [FeatureId::GG02, FeatureId::GG20]
    );
    assert_eq!(
        features("CREATE GRAPH TYPE IF NOT EXISTS t { NODE TYPE Person () }"),
        [FeatureId::GG02, FeatureId::GG20, FeatureId::GC03]
    );
    assert_eq!(features("DROP GRAPH TYPE t"), [FeatureId::GG02]);
    assert_eq!(
        features("DROP GRAPH TYPE IF EXISTS t"),
        [FeatureId::GG02, FeatureId::GC03]
    );
    assert_eq!(
        features("CREATE GRAPH g TYPED t"),
        [FeatureId::GC04, FeatureId::GG02]
    );
    assert_eq!(
        features("CREATE GRAPH IF NOT EXISTS g TYPED t"),
        [FeatureId::GC04, FeatureId::GG02, FeatureId::GC05]
    );
}

#[test]
fn adjacent_valid_graph_type_sources_are_rejected_before_a_command_exists() {
    for (source, fragment) in [
        ("CREATE GRAPH TYPE t COPY OF u", "COPY OF"),
        ("CREATE GRAPH TYPE t AS COPY OF /s/u", "COPY OF"),
        (
            "CREATE GRAPH TYPE t { NODE TYPE Person ({name STRING}) }",
            "property types",
        ),
        (
            "CREATE GRAPH TYPE t { NODE TYPE Person ({}) }",
            "property types",
        ),
        (
            "CREATE GRAPH TYPE t { NODE TYPE Person (:Person) }",
            "labels",
        ),
        ("CREATE GRAPH TYPE t { NODE TYPE Person (p) }", "aliases"),
        ("CREATE GRAPH TYPE t { (p :Person) }", "labels"),
        ("CREATE GRAPH TYPE t { NODE TYPE Person AS p }", "aliases"),
        (
            "CREATE GRAPH TYPE t { NODE TYPE Person (), EDGE TYPE Knows (Person)-[:KNOWS]->(Person) }",
            "edge types",
        ),
    ] {
        let error = assert_not_implemented(source, fragment);
        let ParserError::NotImplemented { hint, .. } = error else {
            unreachable!()
        };
        assert!(
            hint.as_deref()
                .is_some_and(|hint| hint.contains("property-free")),
            "{source}"
        );
    }

    let source = "CREATE GRAPH TYPE t { NODE TYPE Person (:Person =>) }";
    let error = parse(source).expect_err(source);
    let ParserError::UnsupportedFeature {
        feature_id, span, ..
    } = error
    else {
        panic!("expected GG21 rejection");
    };
    assert_eq!(feature_id, FeatureId::GG21);
    assert_eq!(
        &source[span.byte_offset as usize..span.end() as usize],
        ":Person =>"
    );

    let source = "CREATE GRAPH TYPE t LIKE g";
    let error = parse(source).expect_err(source);
    assert!(matches!(
        error,
        ParserError::UnsupportedFeature {
            feature_id: FeatureId::GG04,
            ..
        }
    ));
}

#[test]
fn malformed_graph_type_shapes_remain_syntax_errors() {
    for source in [
        "CREATE GRAPH TYPE t",
        "CREATE GRAPH TYPE t {}",
        "CREATE GRAPH TYPE t { NODE TYPE Person ( }",
        "CREATE GRAPH TYPE t { NODE TYPE Person (), }",
        "CREATE GRAPH TYPE t { NODE TYPE Person (),, NODE TYPE City () }",
        "CREATE GRAPH TYPE t COPY OF",
        "CREATE GRAPH TYPE t AS",
        "CREATE GRAPH TYPE IF EXISTS t { NODE TYPE Person () }",
    ] {
        assert_syntax(source);
    }
}

#[test]
fn graph_type_next_chains_are_observed_then_rejected_as_one_statement() {
    for source in [
        "CREATE GRAPH TYPE t { NODE TYPE Person () } NEXT DROP GRAPH TYPE t",
        "DROP GRAPH TYPE t NEXT CREATE GRAPH TYPE t { NODE TYPE Person () }",
    ] {
        let error = assert_not_implemented(source, "section 12.1");
        let ParserError::NotImplemented { span, .. } = error else {
            unreachable!()
        };
        assert_eq!(span, SourceSpan::new(0, source.len() as u32));
    }
}
