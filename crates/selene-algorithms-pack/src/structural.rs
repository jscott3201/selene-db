//! Structural algorithm procedure adapters.

use std::sync::Arc;

use selene_algorithms::{
    articulation_points_with_checker, bridges_with_checker, scc_count_with_checker,
    scc_with_checker, topological_sort_with_checker, wcc_count_with_checker, wcc_with_checker,
};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, required_string},
    error::{algorithm_aborted, topo_sort_error},
    state::{AlgorithmsPackState, with_algorithm_projection},
};

static WCC_NAME: [&str; 2] = ["algo", "wcc"];
static SCC_NAME: [&str; 2] = ["algo", "scc"];
static WCC_COUNT_NAME: [&str; 2] = ["algo", "wcc_count"];
static SCC_COUNT_NAME: [&str; 2] = ["algo", "scc_count"];
static TOPO_NAME: [&str; 2] = ["algo", "topological_sort"];
static ARTICULATION_NAME: [&str; 2] = ["algo", "articulation_points"];
static BRIDGES_NAME: [&str; 2] = ["algo", "bridges"];

const WCC_PROC: &str = "algo.wcc";
const SCC_PROC: &str = "algo.scc";
const WCC_COUNT_PROC: &str = "algo.wcc_count";
const SCC_COUNT_PROC: &str = "algo.scc_count";
const TOPO_PROC: &str = "algo.topological_sort";
const ARTICULATION_PROC: &str = "algo.articulation_points";
const BRIDGES_PROC: &str = "algo.bridges";

pub(crate) fn procedures(state: Arc<AlgorithmsPackState>) -> Vec<Arc<dyn ExternalGraphProcedure>> {
    vec![
        Arc::new(WccProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(SccProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(WccCountProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(SccCountProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(TopologicalSortProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(ArticulationPointsProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(BridgesProcedure { state }),
    ]
}

struct WccProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for WccProcedure {
    fn name(&self) -> &'static [&'static str] {
        &WCC_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        node_component_columns()
    }
}

impl ExternalGraphProcedure for WccProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(WCC_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            Ok(ProcedureResult {
                rows: wcc_with_checker(projection, checker)
                    .map_err(algorithm_aborted)?
                    .into_iter()
                    .map(node_component_row)
                    .collect(),
            })
        })
    }
}

struct SccProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for SccProcedure {
    fn name(&self) -> &'static [&'static str] {
        &SCC_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        node_component_columns()
    }
}

impl ExternalGraphProcedure for SccProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(SCC_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            Ok(ProcedureResult {
                rows: scc_with_checker(projection, checker)
                    .map_err(algorithm_aborted)?
                    .into_iter()
                    .map(node_component_row)
                    .collect(),
            })
        })
    }
}

struct WccCountProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for WccCountProcedure {
    fn name(&self) -> &'static [&'static str] {
        &WCC_COUNT_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![output("count", GqlType::Uint64)]
    }
}

impl ExternalGraphProcedure for WccCountProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(WCC_COUNT_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            Ok(single_uint_result(
                wcc_count_with_checker(projection, checker).map_err(algorithm_aborted)? as u64,
            ))
        })
    }
}

struct SccCountProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for SccCountProcedure {
    fn name(&self) -> &'static [&'static str] {
        &SCC_COUNT_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![output("count", GqlType::Uint64)]
    }
}

impl ExternalGraphProcedure for SccCountProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(SCC_COUNT_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            Ok(single_uint_result(
                scc_count_with_checker(projection, checker).map_err(algorithm_aborted)? as u64,
            ))
        })
    }
}

struct TopologicalSortProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for TopologicalSortProcedure {
    fn name(&self) -> &'static [&'static str] {
        &TOPO_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("topo_position", GqlType::Uint64),
        ]
    }
}

impl ExternalGraphProcedure for TopologicalSortProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(TOPO_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = topological_sort_with_checker(projection, checker)
                .map_err(topo_sort_error)?
                .into_iter()
                .map(|(node_id, position)| {
                    vec![Value::NodeRef(node_id), Value::Uint(position as u64)]
                })
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct ArticulationPointsProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ArticulationPointsProcedure {
    fn name(&self) -> &'static [&'static str] {
        &ARTICULATION_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![output("node_id", GqlType::NodeRef)]
    }
}

impl ExternalGraphProcedure for ArticulationPointsProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(ARTICULATION_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            Ok(ProcedureResult {
                rows: articulation_points_with_checker(projection, checker)
                    .map_err(algorithm_aborted)?
                    .into_iter()
                    .map(|node_id| vec![Value::NodeRef(node_id)])
                    .collect(),
            })
        })
    }
}

struct BridgesProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for BridgesProcedure {
    fn name(&self) -> &'static [&'static str] {
        &BRIDGES_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        projection_signature()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("from_node", GqlType::NodeRef),
            output("to_node", GqlType::NodeRef),
        ]
    }
}

impl ExternalGraphProcedure for BridgesProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let projection_name = parse_projection_name(BRIDGES_PROC, args)?;
        let checker = ctx.cancellation_checker();
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
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
}

fn parse_projection_name(
    procedure: &'static str,
    args: &[Value],
) -> Result<String, ProcedureError> {
    expect_arity(procedure, args, 1)?;
    required_string(procedure, args, 0, "projection_name")
}

fn projection_signature() -> Vec<ExternalParameter> {
    vec![parameter("projection_name", GqlType::String, false)]
}

fn node_component_columns() -> Vec<ExternalOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("component_id", GqlType::Uint64),
    ]
}

fn node_component_row((node_id, component_id): (selene_core::NodeId, u64)) -> Vec<Value> {
    vec![Value::NodeRef(node_id), Value::Uint(component_id)]
}

fn single_uint_result(value: u64) -> ProcedureResult {
    ProcedureResult {
        rows: vec![vec![Value::Uint(value)]],
    }
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
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
