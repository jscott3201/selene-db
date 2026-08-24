//! DDL keyword-boundary regression coverage.

use selene_gql::{ParserError, parse};
use selene_profile::FeatureId;

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

fn assert_unsupported(source: &str, expected: FeatureId) {
    let error = selene_gql::parse(source).expect_err(source);
    assert_eq!(error.gqlstatus().as_str(), "42N01", "{source}");
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature for {source}");
    };
    assert_eq!(feature_id, expected, "{source}");
}

fn assert_parses(source: &str) {
    parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
}

fn assert_not_implemented(source: &str) {
    let error = selene_gql::parse(source).expect_err(source);
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
        "CREATE SCHEMA IF NOT EXISTS /foo",
        "DROP SCHEMA IF EXISTS /foo",
        "CREATE PROPERTY GRAPH IF NOT EXISTS /foo/g TYPED ANY PROPERTY GRAPH",
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
        "CREATE OR REPLACE GRAPH g ANY",
        "CREATE SCHEMA /foo NEXT CREATE SCHEMA /bar",
        "CREATE GRAPH TYPE t {(Person :Person)}",
    ] {
        assert_not_implemented(source);
    }
}

#[test]
fn guarded_catalog_keywords_admit_iso_forms_and_reject_deferred_clauses() {
    for source in [
        "CREATE SCHEMA /foo",
        "CREATE SCHEMA IF NOT EXISTS /foo",
        "DROP SCHEMA /foo",
        "CREATE GRAPH g ANY",
        "CREATE GRAPH IF NOT EXISTS g ANY",
        "DROP PROPERTY GRAPH IF EXISTS /foo/g",
    ] {
        assert_parses(source);
    }
    // ISO section 12.4 makes the graph type clause mandatory.
    for source in ["CREATE GRAPH g", "CREATE GRAPH IF NOT EXISTS g"] {
        assert_syntax_error(source);
    }
    assert_unsupported("CREATE GRAPH g LIKE other", FeatureId::GG04);
    assert_unsupported("CREATE GRAPH g AS COPY OF other", FeatureId::GG05);
}
