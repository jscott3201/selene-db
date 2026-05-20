//! External registration and metadata contract tests.

mod common;

use std::sync::Arc;

use common::{execute_ok, graph_with_edges, istr, rows};
use selene_algorithms_pack::{ALGO_PROCEDURE_NAMES, AlgorithmsPack};
use selene_core::Value;
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureRegistry, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
    ExternalProcedurePack, ProcedurePackRegistry, RegistryError,
};

static TEST_NAME: [&str; 2] = ["test", "answer"];

struct TestProcedure;

impl ExternalProcedureMetadata for TestProcedure {
    fn name(&self) -> &'static [&'static str] {
        &TEST_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        Vec::new()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![ExternalOutputColumn::new("value", GqlType::Integer)]
    }
}

impl ExternalGraphProcedure for TestProcedure {
    fn execute(
        &self,
        _ctx: &GraphContext<'_>,
        _args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        Ok(ProcedureResult {
            rows: vec![vec![Value::Int(42)]],
        })
    }
}

#[test]
fn external_pack_can_register_without_lifting_platform_visibility() {
    let registry = ProcedurePackRegistry::builder()
        .with_external_pack(ExternalProcedurePack::new(
            "test-pack",
            vec![Arc::new(TestProcedure)],
        ))
        .build()
        .expect("external pack registers");
    let (graph, _) = graph_with_edges(7_101, &[]);

    let table = rows(execute_ok(
        "CALL test.answer() YIELD value",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::Int(42)]);
}

#[test]
fn procedure_pack_registry_rejects_duplicate_pack_name() {
    let err = ProcedurePackRegistry::builder()
        .with_external_pack(ExternalProcedurePack::new("dup", Vec::new()))
        .with_external_pack(ExternalProcedurePack::new("dup", Vec::new()))
        .build()
        .expect_err("duplicate pack names are rejected");

    assert!(matches!(err, RegistryError::PackConflict { .. }));
}

#[test]
fn capability_required_is_none_for_every_algorithms_pack_procedure() {
    let pack = AlgorithmsPack::new();
    let registry = pack.registry_with_builtins().expect("registry builds");

    for name in ALGO_PROCEDURE_NAMES {
        let interned: Vec<_> = name.iter().map(|segment| istr(segment)).collect();
        let metadata = registry
            .lookup(&interned)
            .unwrap_or_else(|| panic!("missing procedure {name:?}"));
        assert_eq!(metadata.capability_required, None);
    }
}
