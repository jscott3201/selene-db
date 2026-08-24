//! Public catalog lifecycle, identity, policy, and graph-type contracts.

use selene_db::{
    Catalog, CatalogPath, CatalogReadSnapshot, CreateOutcome, CreatePolicy, Database, DropOutcome,
    DropPolicy, ErrorKind, ExecutionOutcome, GqlStatus, GraphTypeDefinition, NodeTypeDefinition,
    ObjectPath, PathSegment, SchemaPath, Session,
};

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).expect("fixture schema path is valid")
}

fn object(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).expect("fixture object path is valid")
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
        .with_node_type(
            NodeTypeDefinition::new(
                PathSegment::regular("MemoryType").unwrap(),
                vec![PathSegment::regular("Memory").unwrap()],
            )
            .unwrap(),
        )
        .build()
        .expect("fixture graph type is valid")
}

#[test]
fn multiple_schemas_resolve_list_and_select_same_named_graphs() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    for name in ["zeta", "alpha"] {
        catalog
            .create_schema(&schema(name), CreatePolicy::Strict)
            .unwrap();
        catalog
            .create_graph(&object(name, "shared"), None, CreatePolicy::Strict)
            .unwrap();
    }

    let snapshot = catalog.snapshot();
    let names = snapshot
        .schemas()
        .unwrap()
        .into_iter()
        .map(|descriptor| descriptor.path.schema().canonical().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["alpha", "zeta"]);
    for name in ["alpha", "zeta"] {
        let path = object(name, "shared");
        assert_eq!(snapshot.graphs(&schema(name)).unwrap().len(), 1);
        assert_eq!(snapshot.resolve_graph(&path).unwrap().path, path);
        database
            .session(&path)
            .unwrap()
            .execute("INSERT (:Marker)")
            .unwrap();
    }
    assert_eq!(
        database
            .session(&object("alpha", "shared"))
            .unwrap()
            .execute("MATCH (n) RETURN n")
            .unwrap(),
        ExecutionOutcome::Rows { row_count: 1 }
    );
}

#[test]
fn typed_paths_accept_mixed_validated_segments_without_filesystem_semantics() {
    let catalog_path = CatalogPath::regular("selene").unwrap();
    assert_eq!(catalog_path.to_string(), "/selene");
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema_path = SchemaPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::delimited("space schema").unwrap(),
    );
    catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap();
    let graph_path = ObjectPath::new(
        PathSegment::regular("selene").unwrap(),
        PathSegment::delimited("space schema").unwrap(),
        PathSegment::delimited("graph/name").unwrap(),
    );
    catalog
        .create_graph(&graph_path, None, CreatePolicy::Strict)
        .unwrap();
    assert_eq!(
        catalog.snapshot().resolve_graph(&graph_path).unwrap().path,
        graph_path
    );
    assert_eq!(
        graph_path.to_string(),
        "/selene/`space schema`/`graph/name`"
    );
}

#[test]
fn canonical_duplicates_cross_kind_conflicts_and_wrong_paths_are_structured() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let composed = SchemaPath::regular("selene", "Café").unwrap();
    let decomposed = SchemaPath::regular("selene", "Cafe\u{301}").unwrap();
    catalog
        .create_schema(&composed, CreatePolicy::Strict)
        .unwrap();
    assert!(matches!(
        catalog
            .create_schema(&decomposed, CreatePolicy::IfNotExists)
            .unwrap(),
        CreateOutcome::AlreadyExists(_)
    ));
    for name in ["Case", "case"] {
        catalog
            .create_schema(&schema(name), CreatePolicy::Strict)
            .unwrap();
    }
    assert_ne!(
        catalog
            .snapshot()
            .resolve_schema(&schema("Case"))
            .unwrap()
            .id,
        catalog
            .snapshot()
            .resolve_schema(&schema("case"))
            .unwrap()
            .id
    );

    let shared = ObjectPath::regular("selene", "Café", "shared").unwrap();
    catalog
        .create_graph(&shared, None, CreatePolicy::Strict)
        .unwrap();
    let conflict = catalog
        .create_graph_type(&shared, person_type(), CreatePolicy::IfNotExists)
        .unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::CatalogObjectWrongKind);
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph_type(&shared)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectWrongKind
    );

    let missing_parent = object("missing", "g");
    for error in [
        catalog
            .create_graph(&missing_parent, None, CreatePolicy::IfNotExists)
            .unwrap_err(),
        catalog
            .create_graph_type(&missing_parent, person_type(), CreatePolicy::IfNotExists)
            .unwrap_err(),
        catalog
            .drop_graph(&missing_parent, DropPolicy::IfExists)
            .unwrap_err(),
        catalog
            .drop_graph_type(&missing_parent, DropPolicy::IfExists)
            .unwrap_err(),
    ] {
        assert_eq!(error.kind(), ErrorKind::CatalogObjectNotFound);
    }
    let wrong_catalog = ObjectPath::regular("other", "Café", "shared").unwrap();
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&wrong_catalog)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectNotFound
    );
}

#[test]
fn strict_and_conditional_outcomes_preserve_generation_on_noops() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema_path = schema("lifecycle");
    let initial = catalog.snapshot().generation();
    let CreateOutcome::Created(created) = catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap()
    else {
        panic!("strict creation should create")
    };
    assert_eq!(catalog.snapshot().generation().get(), initial.get() + 1);
    let before_noop = catalog.snapshot();
    assert!(matches!(
        catalog
            .create_schema(&schema_path, CreatePolicy::IfNotExists)
            .unwrap(),
        CreateOutcome::AlreadyExists(existing) if existing.id == created.id
    ));
    assert!(before_noop.shares_state_with(&catalog.snapshot()));
    assert_eq!(
        catalog
            .create_schema(&schema_path, CreatePolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectAlreadyExists
    );
    assert!(before_noop.shares_state_with(&catalog.snapshot()));

    let graph_path = object("lifecycle", "g");
    let CreateOutcome::Created(first_graph) = catalog
        .create_graph(&graph_path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let graph_noop = catalog.snapshot();
    assert!(matches!(
        catalog
            .create_graph(&graph_path, None, CreatePolicy::IfNotExists)
            .unwrap(),
        CreateOutcome::AlreadyExists(_)
    ));
    assert!(graph_noop.shares_state_with(&catalog.snapshot()));
    assert_eq!(
        catalog
            .create_graph(&graph_path, None, CreatePolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectAlreadyExists
    );
    assert!(graph_noop.shares_state_with(&catalog.snapshot()));
    assert!(matches!(
        catalog.drop_graph(&graph_path, DropPolicy::Strict).unwrap(),
        DropOutcome::Dropped(_)
    ));
    let absent = catalog.snapshot();
    assert!(matches!(
        catalog
            .drop_graph(&graph_path, DropPolicy::IfExists)
            .unwrap(),
        DropOutcome::NotFound
    ));
    assert!(absent.shares_state_with(&catalog.snapshot()));
    assert_eq!(
        catalog
            .drop_graph(&graph_path, DropPolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectNotFound
    );
    assert!(absent.shares_state_with(&catalog.snapshot()));
    let CreateOutcome::Created(recreated_graph) = catalog
        .create_graph(&graph_path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    assert!(recreated_graph.id.get() > first_graph.id.get());
    catalog.drop_graph(&graph_path, DropPolicy::Strict).unwrap();

    let type_path = object("lifecycle", "type");
    let CreateOutcome::Created(first_type) = catalog
        .create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let type_noop = catalog.snapshot();
    assert!(matches!(
        catalog
            .create_graph_type(&type_path, person_type(), CreatePolicy::IfNotExists)
            .unwrap(),
        CreateOutcome::AlreadyExists(_)
    ));
    assert!(type_noop.shares_state_with(&catalog.snapshot()));
    assert_eq!(
        catalog
            .create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectAlreadyExists
    );
    assert!(type_noop.shares_state_with(&catalog.snapshot()));
    assert!(matches!(
        catalog
            .drop_graph_type(&type_path, DropPolicy::Strict)
            .unwrap(),
        DropOutcome::Dropped(_)
    ));
    let type_absent = catalog.snapshot();
    assert!(matches!(
        catalog
            .drop_graph_type(&type_path, DropPolicy::IfExists)
            .unwrap(),
        DropOutcome::NotFound
    ));
    assert!(type_absent.shares_state_with(&catalog.snapshot()));
    assert_eq!(
        catalog
            .drop_graph_type(&type_path, DropPolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectNotFound
    );
    assert!(type_absent.shares_state_with(&catalog.snapshot()));
    let CreateOutcome::Created(recreated_type) = catalog
        .create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    assert!(recreated_type.id.get() > first_type.id.get());
    catalog
        .drop_graph_type(&type_path, DropPolicy::Strict)
        .unwrap();

    assert!(matches!(
        catalog
            .drop_schema(&schema_path, DropPolicy::Strict)
            .unwrap(),
        DropOutcome::Dropped(_)
    ));
    assert!(matches!(
        catalog
            .drop_schema(&schema_path, DropPolicy::IfExists)
            .unwrap(),
        DropOutcome::NotFound
    ));
    let schema_absent = catalog.snapshot();
    assert_eq!(
        catalog
            .drop_schema(&schema_path, DropPolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogObjectNotFound
    );
    assert!(schema_absent.shares_state_with(&catalog.snapshot()));
    let CreateOutcome::Created(recreated_schema) = catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    assert!(recreated_schema.id.get() > created.id.get());
}

#[test]
fn restrict_reports_graph_schema_and_graph_type_dependencies() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema_path = schema("restricted");
    catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap();
    let type_path = object("restricted", "memory_type");
    let CreateOutcome::Created(type_descriptor) = catalog
        .create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
        .unwrap()
    else {
        panic!("graph type should be created")
    };
    assert_eq!(type_descriptor.node_type_count, 2);
    for name in ["one", "two"] {
        catalog
            .create_graph(
                &object("restricted", name),
                Some(&type_path),
                CreatePolicy::Strict,
            )
            .unwrap();
    }
    let snapshot = catalog.snapshot();
    let graph_types = snapshot.graph_types(&schema_path).unwrap();
    assert_eq!(graph_types.len(), 1);
    assert_eq!(graph_types[0], type_descriptor);
    let bound_graphs = snapshot.graphs(&schema_path).unwrap();
    assert_eq!(bound_graphs.len(), 2);
    assert!(
        bound_graphs
            .iter()
            .all(|graph| graph.graph_type == Some(type_descriptor.id))
    );
    catalog
        .create_schema(&schema("other"), CreatePolicy::Strict)
        .unwrap();
    assert_eq!(
        catalog
            .create_graph(
                &object("other", "cross_schema"),
                Some(&type_path),
                CreatePolicy::Strict,
            )
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogReferenceViolation
    );
    let reference_error = catalog
        .drop_graph_type(&type_path, DropPolicy::Strict)
        .unwrap_err();
    assert_eq!(reference_error.kind(), ErrorKind::CatalogRestrictViolation);
    assert!(reference_error.message().contains("2 graphs"));
    assert_eq!(
        catalog
            .drop_schema(&schema_path, DropPolicy::Strict)
            .unwrap_err()
            .kind(),
        ErrorKind::CatalogRestrictViolation
    );

    let graph = database.session(&object("restricted", "one")).unwrap();
    graph.execute("INSERT (:Person)").unwrap();
    let nonempty = catalog
        .drop_graph(&object("restricted", "one"), DropPolicy::Strict)
        .unwrap_err();
    assert_eq!(nonempty.kind(), ErrorKind::CatalogRestrictViolation);
    assert!(nonempty.message().contains("1 nodes, 0 edges"));

    let edge_path = object("restricted", "edges");
    catalog
        .create_graph(&edge_path, None, CreatePolicy::Strict)
        .unwrap();
    database
        .session(&edge_path)
        .unwrap()
        .execute("INSERT (:A)-[:E]->(:B)")
        .unwrap();
    let edge_error = catalog
        .drop_graph(&edge_path, DropPolicy::Strict)
        .unwrap_err();
    assert_eq!(edge_error.kind(), ErrorKind::CatalogRestrictViolation);
    assert!(edge_error.message().contains("2 nodes, 1 edges"));
}

#[test]
fn bound_graph_enforces_declared_node_types() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("typed"), CreatePolicy::Strict)
        .unwrap();
    let type_path = object("typed", "closed");
    catalog
        .create_graph_type(&type_path, person_type(), CreatePolicy::Strict)
        .unwrap();
    let graph_path = object("typed", "g");
    let graph_descriptor = match catalog
        .create_graph(&graph_path, Some(&type_path), CreatePolicy::Strict)
        .unwrap()
    {
        CreateOutcome::Created(descriptor) => descriptor,
        CreateOutcome::AlreadyExists(_) | CreateOutcome::Replaced { .. } => unreachable!(),
    };
    assert!(graph_descriptor.graph_type.is_some());
    let graph = database.session(&graph_path).unwrap();
    graph.execute("INSERT (:Person)").unwrap();
    let violation = graph.execute("INSERT (:Undeclared)").unwrap_err();
    assert_eq!(violation.gqlstatus(), Some(GqlStatus::GRAPH_TYPE_VIOLATION));
}

#[test]
fn old_session_never_aliases_same_path_recreation() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("stale"), CreatePolicy::Strict)
        .unwrap();
    let path = object("stale", "g");
    let first = catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap();
    let CreateOutcome::Created(first_descriptor) = first else {
        unreachable!()
    };
    let old = database.session(&path).unwrap();
    catalog.drop_graph(&path, DropPolicy::Strict).unwrap();
    assert_eq!(
        old.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    let CreateOutcome::Created(second_descriptor) = catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    assert_ne!(first_descriptor.id, second_descriptor.id);
    assert_eq!(
        old.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
    assert_eq!(
        database
            .session(&path)
            .unwrap()
            .execute("RETURN 1")
            .unwrap(),
        ExecutionOutcome::Rows { row_count: 1 }
    );
}

#[test]
fn initial_catalog_is_empty_and_first_user_ids_start_at_one() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let initial = catalog.snapshot();
    assert_eq!(initial.generation().get(), 1);
    assert!(initial.schemas().unwrap().is_empty());

    let CreateOutcome::Created(created_schema) = catalog
        .create_schema(&schema("first"), CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let type_name = PathSegment::regular("Person").unwrap();
    let definition = GraphTypeDefinition::builder()
        .with_node_type(NodeTypeDefinition::new(type_name.clone(), vec![type_name]).unwrap())
        .build()
        .unwrap();
    let CreateOutcome::Created(created_type) = catalog
        .create_graph_type(&object("first", "shape"), definition, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let CreateOutcome::Created(created_graph) = catalog
        .create_graph(&object("first", "graph"), None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(created_schema.id.get(), 1);
    assert_eq!(created_type.id.get(), 1);
    assert_eq!(created_graph.id.get(), 1);
}

#[test]
fn shared_handles_are_send_sync_and_session_is_send() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send_static<T: Send + 'static>() {}

    assert_send_sync::<Database>();
    assert_send_sync::<Catalog>();
    assert_send_sync::<CatalogReadSnapshot>();
    assert_send_static::<Session>();
}
