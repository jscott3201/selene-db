//! GQL database-catalog DDL through selected facade sessions.
//!
//! Every statement here reaches catalog state only through the same
//! `Catalog` service Rust callers use. The tests pin reference resolution,
//! outcomes, GQLSTATUS codes, no-op publication behavior, unsupported-clause
//! rejection, selected-schema resolution, and Rust/GQL descriptor equivalence.

use selene_db::{
    Catalog, CatalogReadSnapshot, CreateOutcome, CreatePolicy, Database, DropPolicy, Error,
    ErrorKind, ExecutionOutcome, GqlStatus, GraphTypeDefinition, NodeTypeDefinition, ObjectPath,
    PathSegment, SchemaPath, Session, WriteSummary,
};

const OMITTED: ExecutionOutcome = ExecutionOutcome::OmittedResult {
    status: GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
};

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn graph(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

fn fixture() -> (Database, ObjectPath) {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let path = graph("session_schema", "session_graph");
    catalog
        .create_schema(&schema("session_schema"), CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap();
    (database, path)
}

fn person_type() -> GraphTypeDefinition {
    GraphTypeDefinition::builder()
        .with_node_type(
            NodeTypeDefinition::new(
                PathSegment::regular("PersonType").unwrap(),
                vec![PathSegment::regular("Person").unwrap()],
            )
            .unwrap(),
        )
        .build()
        .unwrap()
}

fn assert_error(error: &Error, kind: ErrorKind, status: GqlStatus, source: &str) {
    assert_eq!(error.kind(), kind, "{source}: {error}");
    assert_eq!(error.gqlstatus(), Some(status), "{source}: {error}");
}

fn assert_unpublished(catalog: &Catalog, before: &CatalogReadSnapshot, source: &str) {
    let after = catalog.snapshot();
    assert!(after.shares_state_with(before), "{source} published state");
    assert_eq!(after.generation(), before.generation(), "{source}");
}

#[test]
fn absolute_and_current_schema_relative_references_round_trip() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();

    assert_eq!(session.execute("CREATE SCHEMA /memory").unwrap(), OMITTED);
    assert_eq!(
        session
            .execute("CREATE GRAPH /memory/episodes ANY")
            .unwrap(),
        OMITTED
    );
    assert_eq!(
        session
            .execute("CREATE PROPERTY GRAPH scratch TYPED ANY PROPERTY GRAPH")
            .unwrap(),
        OMITTED
    );
    let snapshot = catalog.snapshot();
    snapshot.resolve_schema(&schema("memory")).unwrap();
    snapshot
        .resolve_graph(&graph("memory", "episodes"))
        .unwrap();
    let local = snapshot
        .resolve_graph(&graph("session_schema", "scratch"))
        .unwrap();
    assert!(local.graph_type.is_none());
    assert_eq!(local.path.to_string(), "/selene/session_schema/scratch");

    let handle = database
        .session(&graph("session_schema", "scratch"))
        .unwrap();
    assert_eq!(
        handle.execute("INSERT (:Note)").unwrap(),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    handle.execute("MATCH (n:Note) DELETE n").unwrap();

    assert_eq!(session.execute("DROP GRAPH scratch").unwrap(), OMITTED);
    assert_eq!(
        session
            .execute("DROP PROPERTY GRAPH /memory/episodes")
            .unwrap(),
        OMITTED
    );
    assert_eq!(session.execute("DROP SCHEMA /memory").unwrap(), OMITTED);
    let snapshot = catalog.snapshot();
    assert!(snapshot.resolve_schema(&schema("memory")).is_err());
    assert!(
        snapshot
            .resolve_graph(&graph("session_schema", "scratch"))
            .is_err()
    );
    assert_eq!(
        handle.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
}

#[test]
fn delimited_segments_keep_their_spelling_and_canonical_identity() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();

    session.execute("CREATE SCHEMA /`my schema`").unwrap();
    session
        .execute("CREATE GRAPH /`my schema`/\"a/b\" ANY")
        .unwrap();
    session.execute("CREATE GRAPH `back``tick` ANY").unwrap();
    let path = ObjectPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::delimited("my schema").unwrap(),
        PathSegment::delimited("a/b").unwrap(),
    );
    let descriptor = catalog.snapshot().resolve_graph(&path).unwrap();
    assert_eq!(descriptor.path.object().display(), "a/b");
    assert_eq!(descriptor.path.to_string(), "/selene/`my schema`/`a/b`");
    let tick = ObjectPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::regular("session_schema").unwrap(),
        PathSegment::delimited("back`tick").unwrap(),
    );
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&tick)
            .unwrap()
            .path
            .to_string(),
        "/selene/session_schema/`back``tick`"
    );

    // A delimited spelling of a regular name is the same canonical object.
    session.execute("CREATE GRAPH plain ANY").unwrap();
    let error = session.execute("CREATE GRAPH `plain` ANY").unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogObjectAlreadyExists,
        GqlStatus::DUPLICATE_OBJECT,
        "delimited duplicate",
    );
    assert_eq!(session.execute("DROP GRAPH \"plain\"").unwrap(), OMITTED);
}

#[test]
fn canonical_equivalence_rejects_duplicates_and_case_distinct_names_coexist() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();

    session.execute("CREATE GRAPH \"Cafe\u{301}\" ANY").unwrap();
    let before = catalog.snapshot();
    let error = session
        .execute("CREATE GRAPH \"Caf\u{E9}\" ANY")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogObjectAlreadyExists,
        GqlStatus::DUPLICATE_OBJECT,
        "NFC duplicate",
    );
    assert_unpublished(&catalog, &before, "NFC duplicate");
    assert_eq!(
        session
            .execute("CREATE GRAPH IF NOT EXISTS \"Caf\u{E9}\" ANY")
            .unwrap(),
        OMITTED
    );
    assert_unpublished(&catalog, &before, "NFC duplicate IF NOT EXISTS");

    session.execute("CREATE GRAPH cafe ANY").unwrap();
    session.execute("CREATE GRAPH CAFE ANY").unwrap();
    let names = catalog
        .snapshot()
        .graphs(&schema("session_schema"))
        .unwrap()
        .into_iter()
        .map(|graph| graph.path.object().display().to_owned())
        .collect::<Vec<_>>();
    // Display keeps the source spelling; only canonical identity is NFC.
    assert_eq!(names, ["CAFE", "Cafe\u{301}", "cafe", "session_graph"]);
}

#[test]
fn invalid_names_and_predefined_references_never_create_objects() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    let before = catalog.snapshot();

    for source in [
        "CREATE SCHEMA /\"pri\u{E000}vate\"",
        "CREATE GRAPH \"pri\u{E000}vate\" ANY",
        "CREATE GRAPH /public/\"\u{E000}\" ANY",
    ] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::InvalidCatalogName,
            GqlStatus::SYNTAX_ERROR,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    // Predefined schema references are reserved words or unparseable and
    // therefore never reach resolution as a graph name.
    for source in [
        "CREATE GRAPH CURRENT_SCHEMA ANY",
        "CREATE GRAPH /CURRENT_SCHEMA/g ANY",
        "CREATE GRAPH HOME_SCHEMA ANY",
        "CREATE GRAPH . ANY",
        "CREATE GRAPH ../public/g ANY",
        "DROP GRAPH CURRENT_SCHEMA",
    ] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::InvalidGql,
            GqlStatus::SYNTAX_ERROR,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    let names = catalog
        .snapshot()
        .graphs(&schema("session_schema"))
        .unwrap()
        .into_iter()
        .map(|graph| graph.path.object().display().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["session_graph"]);
}

#[test]
fn missing_parents_wrong_kinds_and_strict_failures_report_exact_statuses() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    catalog
        .create_graph_type(
            &graph("session_schema", "t"),
            person_type(),
            CreatePolicy::Strict,
        )
        .unwrap();
    let before = catalog.snapshot();

    for source in [
        "CREATE GRAPH /nope/g ANY",
        "CREATE GRAPH IF NOT EXISTS /nope/g ANY",
        "DROP GRAPH /nope/g",
        "DROP GRAPH IF EXISTS /nope/g",
        "DROP GRAPH nope",
        "DROP SCHEMA /nope",
        "CREATE SCHEMA /a/b",
        "CREATE SCHEMA IF NOT EXISTS /a/b",
        "DROP SCHEMA IF EXISTS /a/b",
        "CREATE GRAPH /a/b/g ANY",
        "DROP GRAPH IF EXISTS /a/b/g",
        "CREATE GRAPH /g ANY",
        "DROP GRAPH /g",
    ] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::CatalogObjectNotFound,
            GqlStatus::INVALID_REFERENCE,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    for source in [
        "CREATE GRAPH t ANY",
        "CREATE GRAPH IF NOT EXISTS t ANY",
        "DROP GRAPH t",
    ] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::CatalogObjectWrongKind,
            GqlStatus::INVALID_REFERENCE,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    session.execute("CREATE SCHEMA /dup").unwrap();
    session.execute("CREATE GRAPH /dup/g ANY").unwrap();
    let before = catalog.snapshot();
    for source in ["CREATE SCHEMA /dup", "CREATE GRAPH /dup/g ANY"] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::CatalogObjectAlreadyExists,
            GqlStatus::DUPLICATE_OBJECT,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
}

#[test]
fn conditional_noops_report_their_condition_without_publishing() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /memory").unwrap();
    session.execute("CREATE GRAPH /memory/g ANY").unwrap();
    let before = catalog.snapshot();

    for source in [
        "CREATE SCHEMA IF NOT EXISTS /memory",
        "CREATE GRAPH IF NOT EXISTS /memory/g ANY",
        "DROP SCHEMA IF EXISTS /absent",
    ] {
        assert_eq!(session.execute(source).unwrap(), OMITTED, "{source}");
        assert_unpublished(&catalog, &before, source);
    }
    for source in [
        "DROP GRAPH IF EXISTS absent",
        "DROP GRAPH IF EXISTS /memory/absent",
    ] {
        assert_eq!(
            session.execute(source).unwrap(),
            ExecutionOutcome::OmittedResult {
                status: GqlStatus::GRAPH_DOES_NOT_EXIST
            },
            "{source}"
        );
        assert_unpublished(&catalog, &before, source);
    }
}

#[test]
fn unsupported_clauses_are_rejected_before_any_catalog_change() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    let before = catalog.snapshot();

    for source in [
        "CREATE GRAPH g LIKE default",
        "CREATE GRAPH g ANY AS COPY OF default",
        "CREATE GRAPH g ::{(Person :Person)}",
        "CREATE OR REPLACE GRAPH g LIKE default",
        "CREATE OR REPLACE GRAPH TYPE t {(Person :Person)}",
        "CREATE GRAPH TYPE t {(Person :Person)}",
        "CREATE GRAPH TYPE t COPY OF other",
        "CREATE SCHEMA /a NEXT CREATE GRAPH /a/g ANY",
    ] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::FeatureNotSupported,
            GqlStatus::FEATURE_NOT_SUPPORTED,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    for source in ["CREATE GRAPH g", "CREATE SCHEMA memory"] {
        let error = session.execute(source).unwrap_err();
        assert_error(
            &error,
            ErrorKind::InvalidGql,
            GqlStatus::SYNTAX_ERROR,
            source,
        );
        assert_unpublished(&catalog, &before, source);
    }
    // EXPLAIN renders the plan and executes nothing.
    assert!(matches!(
        session.execute("EXPLAIN CREATE GRAPH g ANY").unwrap(),
        ExecutionOutcome::Rows { .. }
    ));
    assert_unpublished(&catalog, &before, "EXPLAIN");
    assert!(
        catalog
            .snapshot()
            .resolve_graph(&graph("session_schema", "g"))
            .is_err()
    );
}

#[test]
fn restrict_violations_report_the_dependent_object_class() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /memory").unwrap();
    session.execute("CREATE GRAPH /memory/g ANY").unwrap();
    database
        .session(&graph("memory", "g"))
        .unwrap()
        .execute("INSERT (:Person)")
        .unwrap();
    let before = catalog.snapshot();

    let error = session.execute("DROP GRAPH /memory/g").unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        "nonempty graph",
    );
    assert!(error.message().contains("1 nodes, 0 edges"));
    assert_unpublished(&catalog, &before, "nonempty graph");
    let error = session
        .execute("DROP SCHEMA IF EXISTS /memory")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        "nonempty schema",
    );
    assert_unpublished(&catalog, &before, "nonempty schema");
}

#[test]
fn gql_drop_of_current_graph_obeys_restrict_and_invalidates_the_session() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("INSERT (:Person)").unwrap();
    let before = catalog.snapshot();
    let error = session.execute("DROP GRAPH session_graph").unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        "nonempty current graph",
    );
    assert_unpublished(&catalog, &before, "nonempty current graph");

    session.execute("MATCH (n) DELETE n").unwrap();
    assert_eq!(
        session.execute("DROP GRAPH session_graph").unwrap(),
        OMITTED
    );
    assert!(catalog.snapshot().resolve_graph(&session_path).is_err());
    assert_eq!(
        session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );

    catalog
        .create_graph(&session_path, None, CreatePolicy::Strict)
        .unwrap();
    assert_eq!(
        session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    database
        .session(&session_path)
        .unwrap()
        .execute("RETURN 1")
        .unwrap();
}

#[test]
fn old_sessions_stay_stale_across_gql_drop_and_same_path_recreate() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE GRAPH g ANY").unwrap();
    let path = graph("session_schema", "g");
    let first = catalog.snapshot().resolve_graph(&path).unwrap();
    let old = database.session(&path).unwrap();
    assert_eq!(session.execute("DROP GRAPH g").unwrap(), OMITTED);
    assert_eq!(
        old.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    // The identical source recreates the path with a fresh identity.
    assert_eq!(session.execute("CREATE GRAPH g ANY").unwrap(), OMITTED);
    let second = catalog.snapshot().resolve_graph(&path).unwrap();
    assert_ne!(first.id, second.id);
    assert!(second.id > first.id);
    assert_eq!(
        old.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    let fresh = database.session(&path).unwrap();
    fresh.execute("RETURN 1").unwrap();
}

#[test]
fn rust_and_gql_paths_produce_field_equivalent_descriptors() {
    let (via_gql, gql_session_path) = fixture();
    let (via_rust, _) = fixture();
    let session = via_gql.session(&gql_session_path).unwrap();
    let rust = via_rust.catalog();

    session.execute("CREATE SCHEMA /memory").unwrap();
    session.execute("CREATE SCHEMA /`my schema`").unwrap();
    session
        .execute("CREATE GRAPH /memory/episodes ANY")
        .unwrap();
    session
        .execute("CREATE GRAPH /`my schema`/\"a/b\" ANY")
        .unwrap();
    session.execute("CREATE GRAPH scratch ANY").unwrap();

    let my_schema = SchemaPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::delimited("my schema").unwrap(),
    );
    let ab = ObjectPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::delimited("my schema").unwrap(),
        PathSegment::delimited("a/b").unwrap(),
    );
    rust.create_schema(&schema("memory"), CreatePolicy::Strict)
        .unwrap();
    rust.create_schema(&my_schema, CreatePolicy::Strict)
        .unwrap();
    rust.create_graph(&graph("memory", "episodes"), None, CreatePolicy::Strict)
        .unwrap();
    rust.create_graph(&ab, None, CreatePolicy::Strict).unwrap();
    rust.create_graph(
        &graph("session_schema", "scratch"),
        None,
        CreatePolicy::Strict,
    )
    .unwrap();

    let left = via_gql.catalog().snapshot();
    let right = rust.snapshot();
    assert_eq!(left.generation(), right.generation());
    for path in [schema("memory"), my_schema.clone()] {
        let a = left.resolve_schema(&path).unwrap();
        let b = right.resolve_schema(&path).unwrap();
        assert_eq!(a, b, "{path}");
        assert_eq!(a.path.schema().display(), path.schema().display());
    }
    for path in [
        graph("memory", "episodes"),
        ab.clone(),
        graph("session_schema", "scratch"),
    ] {
        let a = left.resolve_graph(&path).unwrap();
        let b = right.resolve_graph(&path).unwrap();
        assert_eq!(a, b, "{path}");
        assert_eq!(a.path.object().display(), path.object().display());
        assert_eq!(a.path.object().canonical(), path.object().canonical());
    }
    assert_eq!(
        left.schemas().unwrap().len(),
        right.schemas().unwrap().len()
    );

    // Negative symmetry: both paths reject the same cases identically.
    type RustCall = Box<dyn Fn(&Catalog) -> Error>;
    let cases: [(&str, RustCall); 4] = [
        (
            "DROP SCHEMA /session_schema",
            Box::new(|catalog| {
                catalog
                    .drop_schema(&schema("session_schema"), DropPolicy::Strict)
                    .unwrap_err()
            }),
        ),
        (
            "CREATE GRAPH /memory/episodes ANY",
            Box::new(|catalog| {
                catalog
                    .create_graph(&graph("memory", "episodes"), None, CreatePolicy::Strict)
                    .unwrap_err()
            }),
        ),
        (
            "DROP GRAPH /memory/absent",
            Box::new(|catalog| {
                catalog
                    .drop_graph(&graph("memory", "absent"), DropPolicy::Strict)
                    .unwrap_err()
            }),
        ),
        (
            "CREATE GRAPH /absent/g ANY",
            Box::new(|catalog| {
                catalog
                    .create_graph(&graph("absent", "g"), None, CreatePolicy::IfNotExists)
                    .unwrap_err()
            }),
        ),
    ];
    for (source, rust_call) in cases {
        let gql = session.execute(source).unwrap_err();
        let rust = rust_call(&rust);
        assert_eq!(gql.kind(), rust.kind(), "{source}");
        assert_eq!(gql.gqlstatus(), rust.gqlstatus(), "{source}");
        assert!(gql.gqlstatus().is_some(), "{source}");
    }
}

#[test]
fn relative_ddl_resolves_against_each_selected_graph_schema() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    for schema_name in ["alpha", "beta"] {
        catalog
            .create_schema(&schema(schema_name), CreatePolicy::Strict)
            .unwrap();
        catalog
            .create_graph(&graph(schema_name, "selected"), None, CreatePolicy::Strict)
            .unwrap();
    }
    let alpha = database.session(&graph("alpha", "selected")).unwrap();
    let beta = database.session(&graph("beta", "selected")).unwrap();
    alpha.execute("CREATE GRAPH child ANY").unwrap();
    alpha
        .execute("CREATE GRAPH TYPE shape { NODE TYPE Alpha () }")
        .unwrap();
    beta.execute("CREATE GRAPH child ANY").unwrap();
    beta.execute("CREATE GRAPH TYPE shape { NODE TYPE Beta () }")
        .unwrap();

    let snapshot = catalog.snapshot();
    for schema_name in ["alpha", "beta"] {
        snapshot
            .resolve_graph(&graph(schema_name, "child"))
            .unwrap();
        snapshot
            .resolve_graph_type(&graph(schema_name, "shape"))
            .unwrap();
    }
}

#[test]
fn sessions_share_catalog_state_and_stay_usable_after_catalog_errors() {
    let (database, session_path) = fixture();
    let first = database.session(&session_path).unwrap();
    let second: Session = database.session(&session_path).unwrap();
    first.execute("CREATE SCHEMA /shared").unwrap();
    assert_eq!(
        second.execute("CREATE GRAPH /shared/g ANY").unwrap(),
        OMITTED
    );
    second.execute("DROP GRAPH /shared/absent").unwrap_err();
    assert_eq!(
        second.execute("INSERT (:Person)").unwrap(),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        first.execute("MATCH (n:Person) RETURN n").unwrap(),
        ExecutionOutcome::Rows { row_count: 1 }
    );
}

#[test]
fn facade_statuses_match_engine_statuses_where_both_exist() {
    for (facade, engine) in [
        (
            GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
            selene_gql::GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
        ),
        (
            GqlStatus::GRAPH_DOES_NOT_EXIST,
            selene_gql::GqlStatus::GRAPH_DOES_NOT_EXIST,
        ),
        (GqlStatus::SYNTAX_ERROR, selene_gql::GqlStatus::SYNTAX_ERROR),
        (
            GqlStatus::INVALID_REFERENCE,
            selene_gql::GqlStatus::INVALID_REFERENCE,
        ),
        (
            GqlStatus::FEATURE_NOT_SUPPORTED,
            selene_gql::GqlStatus::FEATURE_NOT_SUPPORTED,
        ),
        (
            GqlStatus::DUPLICATE_OBJECT,
            selene_gql::GqlStatus::DUPLICATE_OBJECT,
        ),
        (
            GqlStatus::DEPENDENT_OBJECT_ERROR,
            selene_gql::GqlStatus::DEPENDENT_OBJECT_ERROR,
        ),
        (
            GqlStatus::GRAPH_TYPE_VIOLATION,
            selene_gql::GqlStatus::GRAPH_TYPE_VIOLATION,
        ),
    ] {
        assert_eq!(facade.as_str(), engine.as_str());
        assert!(selene_core::gqlstatus_name(facade.as_str()).is_some());
    }
    assert_eq!(ErrorKind::StaleGraphSelection.gqlstatus(), None);
    assert_eq!(ErrorKind::StaleSessionReference.gqlstatus(), None);
}

#[test]
fn rust_create_outcomes_are_unchanged_by_the_gql_router() {
    let (database, session_path) = fixture();
    let catalog = database.catalog();
    database
        .session(&session_path)
        .unwrap()
        .execute("CREATE SCHEMA /memory")
        .unwrap();
    assert!(matches!(
        catalog
            .create_schema(&schema("memory"), CreatePolicy::IfNotExists)
            .unwrap(),
        CreateOutcome::AlreadyExists(_)
    ));
}
