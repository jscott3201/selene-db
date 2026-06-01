//! Native structural procedures (`algo.wcc`, `algo.scc`, `algo.wcc_count`,
//! `algo.scc_count`, `algo.topological_sort`, `algo.articulation_points`,
//! `algo.bridges`).
//!
//! Ported from the historical procedure-pack structural adapter. Each procedure
//! takes a single `projection_name` argument; output columns and row shapes are
//! preserved verbatim.

use selene_algorithms::{
    articulation_points_with_checker, bridges_with_checker, scc_count_with_checker,
    scc_with_checker, topological_sort_with_checker, wcc_count_with_checker, wcc_with_checker,
};
use selene_core::{CancellationChecker, NodeId, Value};
use selene_graph::SeleneGraph;

use super::args::{expect_arity, required_string};
use super::error::{algorithm_aborted, topo_sort_error};
use super::meta::{output, parameter};
use super::state::{AlgorithmCatalogs, with_projection};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const WCC_PROC: &str = "algo.wcc";
const SCC_PROC: &str = "algo.scc";
const WCC_COUNT_PROC: &str = "algo.wcc_count";
const SCC_COUNT_PROC: &str = "algo.scc_count";
const TOPO_PROC: &str = "algo.topological_sort";
const ARTICULATION_PROC: &str = "algo.articulation_points";
const BRIDGES_PROC: &str = "algo.bridges";

pub(super) fn projection_signature() -> Vec<ProcedureParameter> {
    vec![parameter("projection_name", GqlType::String, false)]
}

pub(super) fn node_component_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("component_id", GqlType::Uint64),
    ]
}

pub(super) fn count_columns() -> Vec<ProcedureOutputColumn> {
    vec![output("count", GqlType::Uint64)]
}

pub(super) fn topo_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("topo_position", GqlType::Uint64),
    ]
}

pub(super) fn node_id_columns() -> Vec<ProcedureOutputColumn> {
    vec![output("node_id", GqlType::NodeRef)]
}

pub(super) fn bridges_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("from_node", GqlType::NodeRef),
        output("to_node", GqlType::NodeRef),
    ]
}

pub(super) fn wcc(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(WCC_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(ProcedureResult {
            rows: wcc_with_checker(projection, checker)
                .map_err(algorithm_aborted)?
                .into_iter()
                .map(node_component_row)
                .collect(),
        })
    })
}

pub(super) fn scc(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(SCC_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(ProcedureResult {
            rows: scc_with_checker(projection, checker)
                .map_err(algorithm_aborted)?
                .into_iter()
                .map(node_component_row)
                .collect(),
        })
    })
}

pub(super) fn wcc_count(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(WCC_COUNT_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(single_uint_result(
            wcc_count_with_checker(projection, checker).map_err(algorithm_aborted)? as u64,
        ))
    })
}

pub(super) fn scc_count(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(SCC_COUNT_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(single_uint_result(
            scc_count_with_checker(projection, checker).map_err(algorithm_aborted)? as u64,
        ))
    })
}

pub(super) fn topological_sort(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(TOPO_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = topological_sort_with_checker(projection, checker)
            .map_err(topo_sort_error)?
            .into_iter()
            .map(|(node_id, position)| vec![Value::NodeRef(node_id), Value::Uint(position as u64)])
            .collect();
        Ok(ProcedureResult { rows })
    })
}

pub(super) fn articulation_points(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(ARTICULATION_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(ProcedureResult {
            rows: articulation_points_with_checker(projection, checker)
                .map_err(algorithm_aborted)?
                .into_iter()
                .map(|node_id| vec![Value::NodeRef(node_id)])
                .collect(),
        })
    })
}

pub(super) fn bridges(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let projection_name = parse_projection_name(BRIDGES_PROC, args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        Ok(ProcedureResult {
            rows: bridges_with_checker(projection, checker)
                .map_err(algorithm_aborted)?
                .into_iter()
                .map(|(from_node, to_node)| {
                    vec![Value::NodeRef(from_node), Value::NodeRef(to_node)]
                })
                .collect(),
        })
    })
}

fn parse_projection_name(
    procedure: &'static str,
    args: &[Value],
) -> Result<String, ProcedureError> {
    expect_arity(procedure, args, 1)?;
    required_string(procedure, args, 0, "projection_name")
}

fn node_component_row((node_id, component_id): (NodeId, u64)) -> Vec<Value> {
    vec![Value::NodeRef(node_id), Value::Uint(component_id)]
}

fn single_uint_result(value: u64) -> ProcedureResult {
    ProcedureResult {
        rows: vec![vec![Value::Uint(value)]],
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{Value, intern};

    use super::*;

    fn projection_name() -> Value {
        Value::String(intern("p").expect("test string interns"))
    }

    #[test]
    fn structural_args_accept_one_string() {
        let name = parse_projection_name(WCC_PROC, &[projection_name()])
            .expect("one string projection name parses");

        assert_eq!(name, "p");
    }

    #[test]
    fn structural_args_reject_missing_projection_name() {
        let err = parse_projection_name(WCC_PROC, &[])
            .expect_err("missing projection_name must be rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn structural_args_reject_null_projection_name() {
        let err = parse_projection_name(WCC_PROC, &[Value::Null])
            .expect_err("NULL projection_name must be rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }
}
