use std::{rc::Rc, sync::Arc};

use selene_core::{CancellationChecker, GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_graph::{IndexProvider, SharedGraph};

use super::*;
use crate::{BindingTableRegistry, ImplDefinedCaps};

fn churned_graph() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = db_string("CompactionNode").expect("label fits");
    let edge = db_string("COMPACTION_EDGE").expect("edge label fits");
    {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .expect("create a");
            let b = mutator
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .expect("create b");
            let c = mutator
                .create_node(LabelSet::single(label), PropertyMap::new())
                .expect("create c");
            mutator
                .create_edge(edge.clone(), a, b, PropertyMap::new())
                .expect("create edge a->b");
            mutator
                .create_edge(edge, b, c, PropertyMap::new())
                .expect("create edge b->c");
            mutator.delete_node(b).expect("delete b");
        }
        txn.commit().expect("commit churn");
    }
    shared
}

#[test]
fn compaction_stats_reports_reclaimable_rows() {
    let shared = churned_graph();
    let snapshot = shared.read();
    let caps = ImplDefinedCaps::default();
    let providers: Vec<Arc<dyn IndexProvider>> = Vec::new();
    let ctx = GraphContext::new(
        snapshot.as_ref(),
        &caps,
        &providers,
        CancellationChecker::disabled(),
        Rc::new(BindingTableRegistry::new()),
    );

    let result = execute_stats(&ctx, &[]).expect("stats executes");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0],
        vec![
            Value::Uint(3),
            Value::Uint(2),
            Value::Uint(1),
            Value::Uint(2),
            Value::Uint(0),
            Value::Uint(2),
            Value::Uint(5),
            Value::Uint(2),
            Value::Uint(3),
            Value::Uint(6_000),
            Value::Bool(false),
            Value::Bool(false),
        ]
    );
}

#[test]
fn compact_reports_before_reclaimed_and_after_stats() {
    let shared = churned_graph();
    let caps = ImplDefinedCaps::default();
    let ctx = MaintenanceContext::new(
        &shared,
        &caps,
        CancellationChecker::disabled(),
        Rc::new(BindingTableRegistry::new()),
    );

    let result = execute_compact(&ctx, &[]).expect("compact executes");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0],
        vec![
            Value::Uint(3),
            Value::Uint(2),
            Value::Uint(1),
            Value::Uint(2),
            Value::Uint(0),
            Value::Uint(2),
            Value::Uint(5),
            Value::Uint(2),
            Value::Uint(3),
            Value::Uint(6_000),
            Value::Bool(false),
            Value::Bool(false),
            Value::Uint(1),
            Value::Uint(2),
            Value::Uint(2),
            Value::Uint(2),
            Value::Uint(0),
            Value::Uint(0),
            Value::Uint(0),
            Value::Uint(0),
            Value::Uint(2),
            Value::Uint(2),
            Value::Uint(0),
            Value::Uint(0),
            Value::Bool(false),
            Value::Bool(true),
        ]
    );
    assert!(shared.compaction_stats().is_dense());
}

#[test]
fn compaction_procedures_reject_arguments() {
    let shared = churned_graph();
    let snapshot = shared.read();
    let caps = ImplDefinedCaps::default();
    let providers: Vec<Arc<dyn IndexProvider>> = Vec::new();
    let graph_ctx = GraphContext::new(
        snapshot.as_ref(),
        &caps,
        &providers,
        CancellationChecker::disabled(),
        Rc::new(BindingTableRegistry::new()),
    );
    let maintenance_ctx = MaintenanceContext::new(
        &shared,
        &caps,
        CancellationChecker::disabled(),
        Rc::new(BindingTableRegistry::new()),
    );

    let stats_err = execute_stats(&graph_ctx, &[Value::Bool(true)]).unwrap_err();
    assert!(matches!(stats_err, ProcedureError::InvalidArgument { .. }));
    let compact_err = execute_compact(&maintenance_ctx, &[Value::Bool(true)]).unwrap_err();
    assert!(matches!(
        compact_err,
        ProcedureError::InvalidArgument { .. }
    ));
}
