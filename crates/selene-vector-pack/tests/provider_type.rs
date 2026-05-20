//! Provider downcast rejection tests for vector-pack adapters.

use std::sync::Arc;

use selene_core::{Change, GraphId};
use selene_gql::{ExecutorError, ProcedureError, analyze, execute_statement, parse, plan};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{HnswConfig, HnswProvider};
use selene_vector_pack::VectorPack;

fn registry(pack: &VectorPack) -> ProcedurePackRegistry {
    pack.registry_with_builtins()
        .expect("vector pack registers cleanly")
}

fn execute_err(
    graph: &SharedGraph,
    registry: &ProcedurePackRegistry,
    source: &str,
) -> ExecutorError {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    let plan = plan(&analyzed, registry).expect("test input plans");
    let mut session = selene_gql::Session::new(graph);
    execute_statement(&plan, &mut session, registry).expect_err("statement errors")
}

#[test]
fn vector_search_rejects_non_hnsw_vect_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = SharedGraph::builder(GraphId::new(8_708))
        .with_provider(Arc::new(WrongVectProvider) as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");
    let err = execute_err(
        &graph,
        &registry,
        "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL, NULL) YIELD node_id",
    );

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_search_rejects_hnsw_only_graph() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(8_709))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");

    let err = execute_err(
        &graph,
        &registry,
        "CALL vector.ivf_search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL, NULL) YIELD node_id",
    );

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_stats_rejects_hnsw_only_graph() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let provider = Arc::new(HnswProvider::new(HnswConfig::new(4).unwrap()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(8_710))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");

    let err = execute_err(
        &graph,
        &registry,
        "CALL vector.ivf_stats('default') YIELD state",
    );

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_search_rejects_non_ivf_ivfp_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = SharedGraph::builder(GraphId::new(8_711))
        .with_provider(Arc::new(WrongIvfpProvider) as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");

    let err = execute_err(
        &graph,
        &registry,
        "CALL vector.ivf_search('default', [1.0, 0.0, 0.0, 0.0], 1, NULL, NULL, NULL) YIELD node_id",
    );

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn ivf_stats_rejects_non_ivf_ivfp_provider() {
    let pack = VectorPack::new();
    let registry = registry(&pack);
    let graph = SharedGraph::builder(GraphId::new(8_712))
        .with_provider(Arc::new(WrongIvfpProvider) as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds");

    let err = execute_err(
        &graph,
        &registry,
        "CALL vector.ivf_stats('default') YIELD state",
    );

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

struct WrongVectProvider;

impl IndexProvider for WrongVectProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"VECT")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

struct WrongIvfpProvider;

impl IndexProvider for WrongIvfpProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"IVFP")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}
