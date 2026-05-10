//! Public registry lookup tests.

use selene_core::intern;
use selene_gql::{GqlType, ProcedureRegistry, ProcedureTier};
use selene_pack::ProcedurePackRegistry;

fn name(segments: &[&str]) -> Vec<selene_pack::IStr> {
    segments
        .iter()
        .map(|segment| intern(segment).expect("test string interns"))
        .collect()
}

fn standard_registry() -> ProcedurePackRegistry {
    ProcedurePackRegistry::with_builtins().expect("platform built-ins register cleanly in tests")
}

#[test]
fn empty_registry_returns_none_for_lookup() {
    let registry = ProcedurePackRegistry::empty();

    assert!(registry.lookup(&name(&["selene", "health"])).is_none());
}

#[test]
fn standard_registry_resolves_selene_health_metadata() {
    let registry = standard_registry();
    let metadata = registry
        .lookup(&name(&["selene", "health"]))
        .expect("selene.health is registered");

    assert_eq!(metadata.handle.raw(), 1);
    assert_eq!(metadata.tier, ProcedureTier::Graph);
    assert_eq!(metadata.signature.parameters.len(), 0);
    assert_eq!(metadata.output_schema.columns.len(), 4);
    assert_eq!(metadata.output_schema.columns[0].name.as_str(), "graph_id");
    assert_eq!(metadata.output_schema.columns[0].ty, GqlType::Uint64);
    assert_eq!(
        metadata.output_schema.columns[3].name.as_str(),
        "schema_bound"
    );
    assert_eq!(metadata.output_schema.columns[3].ty, GqlType::Boolean);
}

#[test]
fn standard_registry_lookup_miss_returns_none() {
    let registry = standard_registry();

    assert!(registry.lookup(&name(&["selene", "missing"])).is_none());
}
