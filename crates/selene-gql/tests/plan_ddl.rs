//! BRIEF-27 DDL planner lowering tests.

use selene_gql::{
    AnalyzedType, CatalogOp, DdlStatement, EmptyProcedureRegistry, GqlType, PipelineOp,
    PlannedTypePropertyConstraint, analyze, parse, plan,
};

fn plan_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn catalog_op(plan: &selene_gql::ExecutionPlan) -> &CatalogOp {
    let [PipelineOp::Catalog(op)] = plan.pipeline.as_slice() else {
        panic!("expected single catalog op, got {:?}", plan.pipeline);
    };
    op
}

#[test]
fn create_node_type_preserves_properties_default_and_validation() {
    let plan = plan_one(
        "CREATE NODE TYPE IF NOT EXISTS :Person EXTENDS :Entity \
         (name :: STRING NOT NULL, age :: INT DEFAULT 0) STRICT",
    );
    let CatalogOp::CreateNodeType {
        if_not_exists,
        extends,
        properties,
        validation_mode,
        ..
    } = catalog_op(&plan)
    else {
        panic!("expected create node type");
    };
    assert!(*if_not_exists);
    assert_eq!(extends.expect("extends").as_str(), "Entity");
    assert_eq!(*validation_mode, Some(selene_gql::ValidationMode::Strict));
    assert_eq!(properties.len(), 2);
    assert!(matches!(
        properties[1].constraints.as_slice(),
        [PlannedTypePropertyConstraint::Default(project, _)]
            if project.ty == AnalyzedType::Resolved(GqlType::Integer)
    ));
}

#[test]
fn create_edge_type_preserves_endpoints() {
    let plan = plan_one("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: DATE)");
    let CatalogOp::CreateEdgeType {
        endpoints,
        properties,
        ..
    } = catalog_op(&plan)
    else {
        panic!("expected create edge type");
    };
    let endpoints = endpoints.as_ref().expect("endpoint spec");
    assert_eq!(endpoints.from_labels[0].as_str(), "Person");
    assert_eq!(endpoints.to_labels[0].as_str(), "Person");
    assert_eq!(properties[0].gql_type, GqlType::Date);
}

#[test]
fn drop_type_and_show_type_plans() {
    let plan = plan_one("DROP EDGE TYPE IF EXISTS :KNOWS");
    assert!(matches!(
        catalog_op(&plan),
        CatalogOp::DropEdgeType {
            if_exists: true,
            ..
        }
    ));

    let plan = plan_one("SHOW NODE TYPES");
    assert!(matches!(catalog_op(&plan), CatalogOp::ShowNodeTypes(_)));
    let columns = &plan.output_schema.columns;
    assert_eq!(columns[0].name.expect("label").as_str(), "label");
    assert_eq!(columns[0].ty, AnalyzedType::Resolved(GqlType::String));
    assert_eq!(columns[1].name.expect("definition").as_str(), "definition");
    assert_eq!(columns[1].ty, AnalyzedType::DYNAMIC);
}

#[test]
fn all_property_constraints_lower() {
    let plan = plan_one(
        "CREATE NODE TYPE :Sensor \
         (v :: STRING NOT NULL DEFAULT 'x' IMMUTABLE UNIQUE INDEXED SEARCHABLE \
          DICTIONARY FILL LOCF INTERVAL '60s' ENCODING RLE)",
    );
    let CatalogOp::CreateNodeType { properties, .. } = catalog_op(&plan) else {
        panic!("expected create node type");
    };
    assert_eq!(properties[0].constraints.len(), 10);
}

#[test]
fn ddl_ast_or_replace_path_still_lowers_for_forward_compat() {
    let name = selene_core::intern("g").expect("test interner");
    let analyzed = selene_gql::AnalyzedStatement {
        statement: selene_gql::AnalyzedStatementKind::Ddl(DdlStatement::CreateGraph {
            name,
            or_replace: true,
            if_not_exists: false,
            span: selene_gql::SourceSpan::new(0, 1),
        }),
        scopes: selene_gql::BindingScopeTree::new(selene_gql::SourceSpan::new(0, 1)),
        references: Vec::new(),
        expr_types: selene_gql::ExprTypeTable::default(),
        expr_ids: selene_gql::ExprIdLookup::default(),
        span: selene_gql::SourceSpan::new(0, 1),
        category: selene_gql::StatementCategory::CatalogModifying,
        write_set: None,
    };
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("plans");
    assert!(matches!(
        catalog_op(&plan),
        CatalogOp::CreateGraph {
            or_replace: true,
            ..
        }
    ));
}

#[test]
fn sentinel_ddl_plan_shape_snapshot() {
    let plan = plan_one("SHOW EDGE TYPES");
    let summary = format!(
        "pipeline={}\ncolumns={}:{}",
        if matches!(catalog_op(&plan), CatalogOp::ShowEdgeTypes(_)) {
            "show-edge-types"
        } else {
            "other"
        },
        plan.output_schema.columns[0].name.expect("label").as_str(),
        plan.output_schema.columns[1]
            .name
            .expect("definition")
            .as_str(),
    );
    insta::assert_snapshot!(summary, @r###"
pipeline=show-edge-types
columns=label:definition
"###);
}
