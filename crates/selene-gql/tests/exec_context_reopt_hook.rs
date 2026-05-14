//! TxContext adaptive re-optimization hook tests.

mod exec_common;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use selene_core::GraphId;
use selene_gql::{
    AdaptiveOptimizer, Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry,
    ImplDefinedCaps, PipelineOpId, TxContext, execute_pipeline,
};
use selene_graph::SeleneGraph;

struct CountingHook {
    observed: AtomicU64,
}

impl AdaptiveOptimizer for CountingHook {
    fn observe_cardinality(&self, _op: PipelineOpId, rows: u64) {
        self.observed.fetch_add(rows, Ordering::Relaxed);
    }
}

#[test]
fn tx_context_default_reopt_hook_is_none() {
    let caps = ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);

    assert!(ctx.reopt_hook().is_none());
}

#[test]
fn tx_context_with_reopt_hook_carries_through_dispatch() {
    let caps = ImplDefinedCaps::default();
    let hook = CountingHook {
        observed: AtomicU64::new(0),
    };
    let mut ctx = TxContext::read_only_with_reopt(
        Arc::new(SeleneGraph::new(GraphId::new(1_001))),
        &caps,
        &EmptyProcedureRegistry,
        &[],
        &hook,
    );
    let plan = exec_common::planned("RETURN 1 AS n");
    let input = BindingTable::new(
        BindingTableSchema { columns: vec![] },
        vec![Binding::empty()],
    );

    let output = execute_pipeline(&plan.pipeline, input, &mut ctx).expect("pipeline executes");

    assert!(ctx.reopt_hook().is_some());
    assert_eq!(output.row_count(), 1);
    assert_eq!(hook.observed.load(Ordering::Relaxed), 0);
}
