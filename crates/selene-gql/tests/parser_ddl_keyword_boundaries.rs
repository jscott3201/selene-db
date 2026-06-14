//! DDL keyword-boundary regression coverage.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, parse};

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

fn assert_unsupported(source: &str, expected: FeatureId) {
    let error = parse(source).expect_err(source);
    assert_eq!(error.gqlstatus().as_str(), "42N01", "{source}");
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature for {source}");
    };
    assert_eq!(feature_id, expected, "{source}");
}

fn assert_not_implemented(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::NotImplemented { .. }),
        "{source} must reject as NotImplemented, got {error:?}"
    );
}

#[test]
fn schema_and_create_type_keywords_require_boundaries() {
    for source in [
        "CREATESCHEMA /foo",
        "CREATE SCHEMAIF NOT EXISTS /foo",
        "CREATE SCHEMA IFNOT EXISTS /foo",
        "CREATENODE TYPE :Person ()",
        "CREATE NODETYPE :Person ()",
        "CREATE NODE TYPEIF NOT EXISTS :Person ()",
        "CREATE ORREPLACE NODE TYPE :Person ()",
        "CREATE OR REPLACENODE TYPE :Person ()",
        "CREATE NODE TYPE :Person EXTENDSx :Entity ()",
        "CREATE NODE TYPE :Person () STRICTx",
        "CREATEEDGETYPE :KNOWS ()",
        "CREATE EDGE TYPE :KNOWS (FROMx :Person TO :Person)",
        "CREATE EDGE TYPE :KNOWS (FROM :Person TOx :Person)",
        "CREATE EDGE TYPE :KNOWS () WARNx",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn drop_truncate_and_show_keywords_require_boundaries() {
    for source in [
        "DROPNODE TYPE :Person",
        "DROP NODETYPE :Person",
        "DROP NODE TYPEIF EXISTS :Person",
        "DROP NODE TYPE IFEXISTS :Person",
        "DROP NODE TYPE :Person CASCADEx",
        "DROPEDGE TYPE :KNOWS",
        "DROP EDGE TYPE :KNOWS RESTRICTx",
        "TRUNCATENODE TYPE :Person",
        "TRUNCATE NODETYPE :Person",
        "TRUNCATEEDGE TYPE :KNOWS",
        "SHOWNODE TYPES",
        "SHOW NODETYPES",
        "SHOWEDGE TYPES",
        "SHOWINDEXES",
        "SHOWPROCEDURES",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn graph_index_and_property_constraint_keywords_require_boundaries() {
    for source in [
        "CREATEGRAPH g",
        "CREATE GRAPHg",
        "CREATE ORREPLACE GRAPH g",
        "CREATE GRAPH IFNOT EXISTS g",
        "DROPGRAPH g",
        "DROP GRAPHIF EXISTS g",
        "CREATEINDEX idx ON :Sensor(ts)",
        "CREATE INDEXidx ON :Sensor(ts)",
        "CREATE INDEX idx ONx :Sensor(ts)",
        "DROPINDEX idx",
        "DROP INDEXIF EXISTS idx",
        "CREATE NODE TYPE :Sensor (v :: STRING NOTNULL)",
        "CREATE NODE TYPE :Sensor (v :: INT DEFAULT1)",
        "CREATE NODE TYPE :Sensor (v :: STRING IMMUTABLEUNIQUE)",
        "CREATE NODE TYPE :Sensor (v :: STRING UNIQUEINDEXED)",
        "CREATE NODE TYPE :Sensor (v :: STRING INDEXEDAS idx)",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn guarded_ddl_keywords_still_accept_implemented_forms() {
    for source in [
        "CREATE NODE TYPE :Person ()",
        "CREATE NODE TYPE IF NOT EXISTS :Person EXTENDS :Entity \
         (name :: STRING NOT NULL DEFAULT 'x' IMMUTABLE UNIQUE INDEXED AS name_idx) STRICT",
        "CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: DATE) WARN",
        "DROP NODE TYPE IF EXISTS :Person CASCADE",
        "DROP EDGE TYPE :KNOWS RESTRICT",
        "TRUNCATE NODE TYPE :Person",
        "TRUNCATE EDGE TYPE :KNOWS",
        "SHOW NODE TYPES",
        "SHOW EDGE TYPES",
        "SHOW INDEXES",
        "SHOW PROCEDURES",
        "DROP GRAPH IF EXISTS g",
        "CREATE INDEX IF NOT EXISTS idx ON :Sensor(ts, value)",
        "DROP INDEX IF EXISTS idx",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn guarded_or_replace_keywords_still_preserve_not_implemented_rejection() {
    for source in [
        "CREATE OR REPLACE NODE TYPE :Person ()",
        "CREATE OR REPLACE EDGE TYPE :KNOWS ()",
        "CREATE OR REPLACE GRAPH g",
    ] {
        assert_not_implemented(source);
    }
}

#[test]
fn guarded_schema_keywords_still_preserve_unsupported_feature_rejection() {
    for source in [
        "CREATE SCHEMA /foo",
        "CREATE SCHEMA IF NOT EXISTS /foo",
        "CREATE SCHEMA /foo NEXT CREATE SCHEMA /bar",
    ] {
        assert_unsupported(source, FeatureId::GC02);
    }

    for source in [
        "CREATE GRAPH g",
        "CREATE GRAPH IF NOT EXISTS g",
        "CREATE GRAPH g LIKE other",
        "CREATE GRAPH g AS COPY OF other",
    ] {
        assert_unsupported(source, FeatureId::GC04);
    }
}
