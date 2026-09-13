//! GQL graph-type lifecycle and closed-graph binding through the facade catalog.

use selene_db::{
    Catalog, CatalogReadSnapshot, CreatePolicy, Database, Error, ErrorKind, ExecutionOutcome,
    GqlStatus, GraphTypeDefinition, NodeTypeDefinition, ObjectPath, PathSegment, SchemaPath,
    WriteSummary,
};

const OMITTED: ExecutionOutcome = ExecutionOutcome::SUCCESSFUL_OMITTED;

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn object(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

fn fixture() -> (Database, ObjectPath) {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let path = object("session_schema", "session_graph");
    catalog
        .create_schema(&schema("session_schema"), CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap();
    (database, path)
}

fn person_type() -> GraphTypeDefinition {
    let name = PathSegment::regular("Person").unwrap();
    GraphTypeDefinition::builder()
        .with_node_type(NodeTypeDefinition::new(name.clone(), vec![name]).unwrap())
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
fn gql_graph_type_lifecycle_binds_and_enforces_a_closed_graph() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /closed").unwrap();

    assert_eq!(
        session
            .execute(
                "CREATE PROPERTY GRAPH TYPE /closed/shape AS { \
                 NODE TYPE Person (), VERTEX TYPE `Memory Type` }",
            )
            .unwrap(),
        OMITTED
    );
    let graph_type = catalog
        .snapshot()
        .resolve_graph_type(&object("closed", "shape"))
        .unwrap();
    assert_eq!(graph_type.node_type_count, 2);

    assert_eq!(
        session
            .execute("CREATE PROPERTY GRAPH /closed/g TYPED /closed/shape")
            .unwrap(),
        OMITTED
    );
    let graph_path = object("closed", "g");
    let graph = catalog.snapshot().resolve_graph(&graph_path).unwrap();
    assert_eq!(graph.graph_type, Some(graph_type.id));
    let handle = database.session(&graph_path).unwrap();
    assert_eq!(
        handle.execute("INSERT (:Person)").unwrap().write_summary(),
        Some(WriteSummary::new(1, None))
    );

    for source in [
        "INSERT (:Undeclared)",
        "INSERT (:Person {name: 'Ada'})",
        "INSERT (:Person)-[:KNOWS]->(:Person)",
    ] {
        let error = handle.execute(source).unwrap_err();
        assert_eq!(
            error.gqlstatus(),
            Some(GqlStatus::GRAPH_TYPE_VIOLATION),
            "{source}: {error}"
        );
        assert_eq!(
            handle.execute("MATCH (n) RETURN n").unwrap().row_count(),
            Some(1),
            "{source} must not publish a partial mutation"
        );
    }

    let before = catalog.snapshot();
    let error = session
        .execute("DROP GRAPH TYPE /closed/shape")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        "referenced graph type",
    );
    assert_unpublished(&catalog, &before, "referenced graph type");

    handle.execute("MATCH (n) DELETE n").unwrap();
    assert_eq!(session.execute("DROP GRAPH /closed/g").unwrap(), OMITTED);
    assert_eq!(
        session.execute("DROP GRAPH TYPE /closed/shape").unwrap(),
        OMITTED
    );
    assert_eq!(session.execute("DROP SCHEMA /closed").unwrap(), OMITTED);
}

#[test]
fn graph_type_strict_and_conditional_outcomes_keep_exact_statuses() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /types").unwrap();
    session
        .execute("CREATE GRAPH TYPE /types/shape { NODE TYPE Person () }")
        .unwrap();
    let before = catalog.snapshot();

    assert_eq!(
        session
            .execute("CREATE GRAPH TYPE IF NOT EXISTS /types/shape { NODE TYPE Other () }")
            .unwrap(),
        OMITTED
    );
    assert_unpublished(&catalog, &before, "CREATE GRAPH TYPE IF NOT EXISTS");
    let error = session
        .execute("CREATE GRAPH TYPE /types/shape { NODE TYPE Person () }")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogObjectAlreadyExists,
        GqlStatus::DUPLICATE_OBJECT,
        "strict duplicate",
    );
    assert_unpublished(&catalog, &before, "strict duplicate");

    assert_eq!(
        session
            .execute("DROP GRAPH TYPE IF EXISTS /types/absent")
            .unwrap(),
        OMITTED
    );
    assert_unpublished(&catalog, &before, "DROP GRAPH TYPE IF EXISTS");
    let error = session
        .execute("DROP GRAPH TYPE /types/absent")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogObjectNotFound,
        GqlStatus::INVALID_REFERENCE,
        "strict missing graph type",
    );
    assert_unpublished(&catalog, &before, "strict missing graph type");

    session
        .execute("CREATE GRAPH TYPE local_shape { NODE TYPE Person () }")
        .unwrap();
    session
        .execute("CREATE GRAPH local_graph TYPED local_shape")
        .unwrap();
    let relative_type = catalog
        .snapshot()
        .resolve_graph_type(&object("session_schema", "local_shape"))
        .unwrap();
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&object("session_schema", "local_graph"))
            .unwrap()
            .graph_type,
        Some(relative_type.id)
    );
    session.execute("DROP GRAPH local_graph").unwrap();
    session.execute("DROP GRAPH TYPE local_shape").unwrap();
}

#[test]
fn graph_type_reference_failures_preserve_shared_namespace_and_parent_rules() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /a").unwrap();
    session.execute("CREATE SCHEMA /b").unwrap();
    session
        .execute("CREATE GRAPH TYPE /a/shape { NODE TYPE Person () }")
        .unwrap();
    session.execute("CREATE GRAPH /a/plain ANY").unwrap();
    let before = catalog.snapshot();

    for source in [
        "CREATE GRAPH TYPE /missing/shape { NODE TYPE Person () }",
        "CREATE GRAPH TYPE IF NOT EXISTS /missing/shape { NODE TYPE Person () }",
        "DROP GRAPH TYPE /missing/shape",
        "DROP GRAPH TYPE IF EXISTS /missing/shape",
        "CREATE GRAPH /a/g TYPED /a/missing",
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
        "CREATE GRAPH TYPE /a/plain { NODE TYPE Person () }",
        "CREATE GRAPH TYPE IF NOT EXISTS /a/plain { NODE TYPE Person () }",
        "DROP GRAPH TYPE /a/plain",
        "DROP GRAPH TYPE IF EXISTS /a/plain",
        "CREATE OR REPLACE GRAPH TYPE /a/plain { NODE TYPE Person () }",
        "CREATE GRAPH /a/g TYPED /a/plain",
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

    let source = "CREATE GRAPH /b/g TYPED /a/shape";
    let error = session.execute(source).unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogReferenceViolation,
        GqlStatus::INVALID_REFERENCE,
        source,
    );
    assert_unpublished(&catalog, &before, source);
    assert!(catalog.snapshot().resolve_graph(&object("b", "g")).is_err());
}

#[test]
fn graph_type_or_replace_is_atomic_fresh_and_restricted_when_referenced() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /revisions").unwrap();
    let path = object("revisions", "shape");

    assert_eq!(
        session
            .execute("CREATE OR REPLACE GRAPH TYPE /revisions/shape { NODE TYPE Person () }")
            .unwrap(),
        OMITTED
    );
    let first = catalog.snapshot().resolve_graph_type(&path).unwrap();
    let before = catalog.snapshot();
    assert_eq!(
        session
            .execute(
                "CREATE OR REPLACE PROPERTY GRAPH TYPE /revisions/shape AS { \
                 NODE TYPE Memory (), NODE TYPE Event () }",
            )
            .unwrap(),
        OMITTED
    );
    let second = catalog.snapshot().resolve_graph_type(&path).unwrap();
    assert!(second.id > first.id);
    assert_eq!(second.node_type_count, 2);
    assert_eq!(
        catalog.snapshot().generation().get(),
        before.generation().get() + 1
    );
    assert!(!catalog.snapshot().shares_state_with(&before));

    let before = catalog.snapshot();
    let error = session
        .execute(
            "CREATE OR REPLACE GRAPH TYPE /revisions/shape { \
             NODE TYPE Person (), NODE TYPE Person () }",
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidGraphType);
    assert_unpublished(&catalog, &before, "invalid OR REPLACE definition");
    assert_eq!(
        catalog.snapshot().resolve_graph_type(&path).unwrap(),
        second
    );

    session.execute("CREATE GRAPH /revisions/swap ANY").unwrap();
    let before = catalog.snapshot();
    session
        .execute("CREATE OR REPLACE GRAPH /revisions/swap TYPED /revisions/shape")
        .unwrap();
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&object("revisions", "swap"))
            .unwrap()
            .graph_type,
        Some(second.id)
    );
    assert_eq!(
        catalog.snapshot().generation().get(),
        before.generation().get() + 1
    );

    session
        .execute("CREATE GRAPH /revisions/g TYPED /revisions/shape")
        .unwrap();
    let before = catalog.snapshot();
    let error = session
        .execute("CREATE OR REPLACE GRAPH TYPE /revisions/shape { NODE TYPE Person () }")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        "referenced OR REPLACE",
    );
    assert_unpublished(&catalog, &before, "referenced OR REPLACE");
    assert_eq!(
        catalog.snapshot().resolve_graph_type(&path).unwrap(),
        second
    );

    let handle = database.session(&object("revisions", "g")).unwrap();
    handle.execute("INSERT (:Memory)").unwrap();
    assert_eq!(
        handle.execute("INSERT (:Person)").unwrap_err().gqlstatus(),
        Some(GqlStatus::GRAPH_TYPE_VIOLATION)
    );
}

#[test]
fn rust_and_gql_graph_type_definitions_produce_equivalent_descriptors() {
    let (via_gql, gql_session_path) = fixture();
    let (via_rust, _) = fixture();
    let session = via_gql.session(&gql_session_path).unwrap();
    let rust = via_rust.catalog();
    let type_path = object("memory", "shape");
    let graph_path = object("memory", "g");

    session.execute("CREATE SCHEMA /memory").unwrap();
    session
        .execute("CREATE GRAPH TYPE /memory/shape { NODE TYPE Person () }")
        .unwrap();
    session
        .execute("CREATE GRAPH /memory/g TYPED /memory/shape")
        .unwrap();

    rust.create_schema(&schema("memory"), CreatePolicy::Strict)
        .unwrap();
    rust.create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
        .unwrap();
    rust.create_graph(&graph_path, Some(&type_path), CreatePolicy::Strict)
        .unwrap();

    let left = via_gql.catalog().snapshot();
    let right = rust.snapshot();
    assert_eq!(left.generation(), right.generation());
    assert_eq!(
        left.resolve_graph_type(&type_path).unwrap(),
        right.resolve_graph_type(&type_path).unwrap()
    );
    assert_eq!(
        left.resolve_graph(&graph_path).unwrap(),
        right.resolve_graph(&graph_path).unwrap()
    );
    for database in [via_gql, via_rust] {
        let handle = database.session(&graph_path).unwrap();
        handle.execute("INSERT (:Person)").unwrap();
        assert_eq!(
            handle.execute("INSERT (:Other)").unwrap_err().gqlstatus(),
            Some(GqlStatus::GRAPH_TYPE_VIOLATION)
        );
    }
}

#[test]
fn rejected_graph_type_commands_cannot_mutate_catalog_or_selected_graph() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("INSERT (:Marker)").unwrap();
    let before = catalog.snapshot();

    for source in [
        "CREATE GRAPH TYPE shape COPY OF other",
        "CREATE GRAPH TYPE shape { NODE TYPE Person ({name STRING}) }",
        "CREATE GRAPH TYPE shape { NODE TYPE Person (:Person =>) }",
        "CREATE GRAPH TYPE shape { EDGE TYPE Knows (Person)-[:KNOWS]->(Person) }",
        "CREATE GRAPH missing_type TYPED absent",
        "CREATE GRAPH TYPE `bad\u{E000}` { NODE TYPE Person () }",
    ] {
        assert!(session.execute(source).is_err(), "{source}");
        assert_unpublished(&catalog, &before, source);
        assert_eq!(
            session.execute("MATCH (n) RETURN n").unwrap().row_count(),
            Some(1),
            "{source} mutated the selected graph"
        );
    }
    assert!(
        catalog
            .snapshot()
            .resolve_graph(&object("session_schema", "missing_type"))
            .is_err()
    );
}
