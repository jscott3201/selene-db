//! `CREATE OR REPLACE GRAPH` through a selected session and the Rust
//! `CreatePolicy::OrReplace` policy (ISO/IEC 39075:2024 §12.4 GR2).
//!
//! Replacement is one publication: the old graph is dropped under the
//! `DROP GRAPH` admission and the new graph is created with a fresh identity
//! in the same state swap. These tests pin the outcomes, statuses, identity
//! and generation accounting, session staleness, current-graph replacement, and
//! Rust/GQL symmetry.

use selene_db::{
    Catalog, CatalogReadSnapshot, CreateOutcome, CreatePolicy, Database, DropPolicy, Error,
    ErrorKind, ExecutionOutcome, GqlStatus, GraphDescriptor, GraphTypeDefinition,
    NodeTypeDefinition, ObjectPath, PathSegment, SchemaPath, WriteSummary,
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

fn resolve(catalog: &Catalog, path: &ObjectPath) -> GraphDescriptor {
    catalog.snapshot().resolve_graph(path).unwrap()
}

#[test]
fn or_replace_creates_when_absent_and_replaces_with_a_fresh_identity() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /memory").unwrap();
    let path = graph("memory", "episodes");

    // Absent: an ordinary create.
    let before = catalog.snapshot();
    assert_eq!(
        session
            .execute("CREATE OR REPLACE GRAPH /memory/episodes ANY")
            .unwrap(),
        OMITTED
    );
    let first = resolve(&catalog, &path);
    assert_eq!(
        catalog.snapshot().generation().get(),
        before.generation().get() + 1
    );
    let old_session = database.session(&path).unwrap();

    // Existing and empty: replaced in exactly one publication.
    let before = catalog.snapshot();
    assert_eq!(
        session
            .execute("CREATE OR REPLACE GRAPH /memory/episodes ANY")
            .unwrap(),
        OMITTED
    );
    let second = resolve(&catalog, &path);
    assert_ne!(second.id, first.id, "replacement allocates a new identity");
    assert!(second.id > first.id);
    assert_eq!(second.path, first.path);
    assert_eq!(
        catalog.snapshot().generation().get(),
        before.generation().get() + 1,
        "replace advances the generation exactly once"
    );
    assert!(!catalog.snapshot().shares_state_with(&before));
    assert_eq!(
        catalog.snapshot().graphs(&schema("memory")).unwrap().len(),
        1,
        "the old descriptor is gone from the same publication"
    );

    // Old sessions are stale; a new session works on the new graph.
    assert_eq!(
        old_session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    let new_session = database.session(&path).unwrap();
    assert_eq!(
        new_session.execute("INSERT (:Note)").unwrap(),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    // The retained pre-replace snapshot still resolves the old identity.
    assert_eq!(before.resolve_graph(&path).unwrap().id, first.id);
}

#[test]
fn every_or_replace_spelling_executes_and_repeats_with_increasing_ids() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /s").unwrap();
    let mut last_id = None;
    for source in [
        "CREATE OR REPLACE GRAPH /s/g ANY",
        "CREATE OR REPLACE PROPERTY GRAPH /s/g TYPED ANY PROPERTY GRAPH",
        "CREATE OR REPLACE GRAPH /s/g :: ANY GRAPH",
        "create or replace property graph /s/g any",
        "CREATE OR REPLACE GRAPH /s/g ANY",
    ] {
        let before = catalog.snapshot();
        assert_eq!(session.execute(source).unwrap(), OMITTED, "{source}");
        let descriptor = resolve(&catalog, &graph("s", "g"));
        assert!(
            last_id.is_none_or(|previous| descriptor.id > previous),
            "{source}: ids must increase across replacements"
        );
        assert_eq!(
            catalog.snapshot().generation().get(),
            before.generation().get() + 1,
            "{source}"
        );
        assert!(descriptor.graph_type.is_none(), "{source}");
        last_id = Some(descriptor.id);
    }
    // The current-schema-relative spelling resolves beside the selected graph.
    session.execute("CREATE GRAPH local_g ANY").unwrap();
    let first = resolve(&catalog, &graph("session_schema", "local_g"));
    assert_eq!(
        session
            .execute("CREATE OR REPLACE GRAPH local_g ANY")
            .unwrap(),
        OMITTED
    );
    assert!(resolve(&catalog, &graph("session_schema", "local_g")).id > first.id);
}

#[test]
fn or_replace_failures_publish_nothing_and_keep_the_old_graph() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    session.execute("CREATE SCHEMA /memory").unwrap();
    session.execute("CREATE GRAPH /memory/full ANY").unwrap();
    database
        .session(&graph("memory", "full"))
        .unwrap()
        .execute("INSERT (:Person)-[:KNOWS]->(:Person)")
        .unwrap();
    catalog
        .create_graph_type(
            &graph("memory", "shape"),
            person_type(),
            CreatePolicy::Strict,
        )
        .unwrap();
    let full = resolve(&catalog, &graph("memory", "full"));
    let before = catalog.snapshot();

    // RESTRICT: the existing graph has live nodes and edges.
    let source = "CREATE OR REPLACE GRAPH /memory/full ANY";
    let error = session.execute(source).unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogRestrictViolation,
        GqlStatus::DEPENDENT_OBJECT_ERROR,
        source,
    );
    assert!(error.message().contains("2 nodes, 1 edges"), "{error}");
    assert_unpublished(&catalog, &before, source);
    assert_eq!(resolve(&catalog, &graph("memory", "full")).id, full.id);
    assert_eq!(
        database
            .session(&graph("memory", "full"))
            .unwrap()
            .execute("MATCH (n) RETURN n")
            .unwrap(),
        ExecutionOutcome::Rows { row_count: 2 }
    );

    // Wrong kind in the shared namespace (§17.2 SR2d(i)(1)).
    let source = "CREATE OR REPLACE GRAPH /memory/shape ANY";
    let error = session.execute(source).unwrap_err();
    assert_error(
        &error,
        ErrorKind::CatalogObjectWrongKind,
        GqlStatus::INVALID_REFERENCE,
        source,
    );
    assert_unpublished(&catalog, &before, source);
    assert!(
        catalog
            .snapshot()
            .resolve_graph_type(&graph("memory", "shape"))
            .is_ok()
    );

    // Missing schema parent.
    for source in [
        "CREATE OR REPLACE GRAPH /absent/g ANY",
        "CREATE OR REPLACE GRAPH /absent/g/deep ANY",
        "CREATE OR REPLACE GRAPH /g ANY",
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

    // Unsupported clauses are still rejected before any command exists.
    for source in [
        "CREATE OR REPLACE GRAPH /memory/full LIKE default",
        "CREATE OR REPLACE GRAPH /memory/full ANY AS COPY OF default",
        "CREATE OR REPLACE GRAPH /memory/full ::{(Person :Person)}",
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
    let error = session
        .execute("CREATE OR REPLACE GRAPH IF NOT EXISTS /memory/full ANY")
        .unwrap_err();
    assert_error(
        &error,
        ErrorKind::InvalidGql,
        GqlStatus::SYNTAX_ERROR,
        "OR REPLACE with IF NOT EXISTS",
    );
    assert_unpublished(&catalog, &before, "OR REPLACE with IF NOT EXISTS");
}

#[test]
fn or_replace_of_the_current_graph_invalidates_the_selected_session() {
    let (database, session_path) = fixture();
    let session = database.session(&session_path).unwrap();
    let catalog = database.catalog();
    let original = resolve(&catalog, &session_path);
    let before = catalog.snapshot();
    assert_eq!(
        session
            .execute("CREATE OR REPLACE GRAPH session_graph ANY")
            .unwrap(),
        OMITTED
    );
    let replacement = resolve(&catalog, &session_path);
    assert!(replacement.id > original.id);
    assert_eq!(
        catalog.snapshot().generation().get(),
        before.generation().get() + 1
    );
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
fn rust_or_replace_matches_gql_and_reports_the_same_failures() {
    let (via_gql, gql_session_path) = fixture();
    let (via_rust, _) = fixture();
    let session = via_gql.session(&gql_session_path).unwrap();
    let rust = via_rust.catalog();
    let path = graph("memory", "episodes");

    session.execute("CREATE SCHEMA /memory").unwrap();
    session
        .execute("CREATE GRAPH /memory/episodes ANY")
        .unwrap();
    session
        .execute("CREATE OR REPLACE GRAPH /memory/episodes ANY")
        .unwrap();
    rust.create_schema(&schema("memory"), CreatePolicy::Strict)
        .unwrap();
    let CreateOutcome::Created(original) = rust
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        panic!("first create publishes")
    };
    let CreateOutcome::Replaced { dropped, created } = rust
        .create_graph(&path, None, CreatePolicy::OrReplace)
        .unwrap()
    else {
        panic!("replace reports both descriptors")
    };
    assert_eq!(dropped, original);
    assert_ne!(created.id, dropped.id);
    assert_eq!(created.path, dropped.path);
    assert!(created.created_at > dropped.created_at);
    assert_eq!(rust.snapshot().resolve_graph(&path).unwrap(), created);
    // A fresh graph under OrReplace is an ordinary create.
    assert!(matches!(
        rust.create_graph(&graph("memory", "other"), None, CreatePolicy::OrReplace)
            .unwrap(),
        CreateOutcome::Created(_)
    ));
    session
        .execute("CREATE OR REPLACE GRAPH /memory/other ANY")
        .unwrap();

    // Same sequence on both sides: descriptors are field-equal.
    let left = via_gql.catalog().snapshot();
    let right = rust.snapshot();
    assert_eq!(left.generation(), right.generation());
    for target in [path.clone(), graph("memory", "other")] {
        assert_eq!(
            left.resolve_graph(&target).unwrap(),
            right.resolve_graph(&target).unwrap(),
            "{target}"
        );
    }

    // Negative symmetry.
    via_gql
        .session(&path)
        .unwrap()
        .execute("INSERT (:Person)")
        .unwrap();
    via_rust
        .session(&path)
        .unwrap()
        .execute("INSERT (:Person)")
        .unwrap();
    rust.create_graph_type(
        &graph("memory", "shape"),
        person_type(),
        CreatePolicy::Strict,
    )
    .unwrap();
    via_gql
        .catalog()
        .create_graph_type(
            &graph("memory", "shape"),
            person_type(),
            CreatePolicy::Strict,
        )
        .unwrap();
    type RustCall = Box<dyn Fn(&Catalog) -> Error>;
    let cases: [(&str, RustCall); 3] = [
        (
            "CREATE OR REPLACE GRAPH /memory/episodes ANY",
            Box::new(|catalog| {
                catalog
                    .create_graph(&graph("memory", "episodes"), None, CreatePolicy::OrReplace)
                    .unwrap_err()
            }),
        ),
        (
            "CREATE OR REPLACE GRAPH /memory/shape ANY",
            Box::new(|catalog| {
                catalog
                    .create_graph(&graph("memory", "shape"), None, CreatePolicy::OrReplace)
                    .unwrap_err()
            }),
        ),
        (
            "CREATE OR REPLACE GRAPH /absent/g ANY",
            Box::new(|catalog| {
                catalog
                    .create_graph(&graph("absent", "g"), None, CreatePolicy::OrReplace)
                    .unwrap_err()
            }),
        ),
    ];
    let before_gql = via_gql.catalog().snapshot();
    let before_rust = rust.snapshot();
    for (source, rust_call) in cases {
        let gql = session.execute(source).unwrap_err();
        let native = rust_call(&rust);
        assert_eq!(gql.kind(), native.kind(), "{source}");
        assert_eq!(gql.gqlstatus(), native.gqlstatus(), "{source}");
        assert!(gql.gqlstatus().is_some(), "{source}");
        assert_unpublished(&via_gql.catalog(), &before_gql, source);
        assert_unpublished(&rust, &before_rust, source);
    }
}

#[test]
fn or_replace_is_rejected_for_schemas_and_supported_for_graph_types() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("memory"), CreatePolicy::Strict)
        .unwrap();
    let before = catalog.snapshot();
    for (label, error) in [
        (
            "existing schema",
            catalog
                .create_schema(&schema("memory"), CreatePolicy::OrReplace)
                .unwrap_err(),
        ),
        (
            "absent schema",
            catalog
                .create_schema(&schema("fresh"), CreatePolicy::OrReplace)
                .unwrap_err(),
        ),
    ] {
        assert_error(
            &error,
            ErrorKind::FeatureNotSupported,
            GqlStatus::FEATURE_NOT_SUPPORTED,
            label,
        );
        assert!(error.message().contains("OR REPLACE"), "{label}: {error}");
        assert_unpublished(&catalog, &before, label);
    }
    assert!(catalog.snapshot().resolve_schema(&schema("fresh")).is_err());
    let CreateOutcome::Created(first_type) = catalog
        .create_graph_type(
            &graph("memory", "shape"),
            person_type(),
            CreatePolicy::OrReplace,
        )
        .unwrap()
    else {
        unreachable!()
    };
    let CreateOutcome::Replaced {
        dropped,
        created: second_type,
    } = catalog
        .create_graph_type(
            &graph("memory", "shape"),
            person_type(),
            CreatePolicy::OrReplace,
        )
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(dropped, first_type);
    assert!(second_type.id > first_type.id);
    // Dropping a replaced graph afterwards works like any other drop.
    catalog
        .create_graph(&graph("memory", "g"), None, CreatePolicy::OrReplace)
        .unwrap();
    catalog
        .create_graph(&graph("memory", "g"), None, CreatePolicy::OrReplace)
        .unwrap();
    assert!(matches!(
        catalog
            .drop_graph(&graph("memory", "g"), DropPolicy::Strict)
            .unwrap(),
        selene_db::DropOutcome::Dropped(_)
    ));
    assert!(
        catalog
            .snapshot()
            .graphs(&schema("memory"))
            .unwrap()
            .is_empty()
    );
}
