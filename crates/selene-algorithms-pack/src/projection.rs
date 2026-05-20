//! Projection-catalog procedure adapters.

use std::sync::{Arc, Arc as StdArc};

use selene_algorithms::{GraphProjection, ProjectionConfig};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, nullable_istr, nullable_istr_list, required_string},
    error::algorithm_error,
    state::AlgorithmsPackState,
};

static BUILD_NAME: [&str; 2] = ["algo", "projection_build"];
static GET_NAME: [&str; 2] = ["algo", "projection_get"];
static DROP_NAME: [&str; 2] = ["algo", "projection_drop"];
static LIST_NAME: [&str; 2] = ["algo", "projection_list"];

const BUILD_PROC: &str = "algo.projection_build";
const GET_PROC: &str = "algo.projection_get";
const DROP_PROC: &str = "algo.projection_drop";
const LIST_PROC: &str = "algo.projection_list";

pub(crate) fn procedures(state: Arc<AlgorithmsPackState>) -> Vec<Arc<dyn ExternalGraphProcedure>> {
    vec![
        Arc::new(ProjectionBuild {
            state: Arc::clone(&state),
        }),
        Arc::new(ProjectionGet {
            state: Arc::clone(&state),
        }),
        Arc::new(ProjectionDrop {
            state: Arc::clone(&state),
        }),
        Arc::new(ProjectionList { state }),
    ]
}

struct ProjectionBuild {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ProjectionBuild {
    fn name(&self) -> &'static [&'static str] {
        &BUILD_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("name", GqlType::String, false),
            parameter(
                "node_labels",
                GqlType::List(Box::new(GqlType::String)),
                true,
            ),
            parameter(
                "edge_labels",
                GqlType::List(Box::new(GqlType::String)),
                true,
            ),
            parameter("weight_property", GqlType::String, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalGraphProcedure for ProjectionBuild {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        expect_arity(BUILD_PROC, args, 4)?;
        let config = ProjectionConfig {
            name: required_string(BUILD_PROC, args, 0, "name")?,
            node_labels: nullable_istr_list(BUILD_PROC, args, 1, "node_labels")?,
            edge_labels: nullable_istr_list(BUILD_PROC, args, 2, "edge_labels")?,
            weight_property: nullable_istr(BUILD_PROC, args, 3, "weight_property")?,
        };
        let graph_id = ctx.snapshot().graph_id();
        self.state.with_catalog(graph_id, |catalog| {
            catalog
                .project(ctx.snapshot(), &config, None)
                .map_err(algorithm_error)?;
            Ok(unit_result())
        })
    }
}

struct ProjectionGet {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ProjectionGet {
    fn name(&self) -> &'static [&'static str] {
        &GET_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![parameter("name", GqlType::String, false)]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        projection_output_columns()
    }
}

impl ExternalGraphProcedure for ProjectionGet {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        expect_arity(GET_PROC, args, 1)?;
        let name = required_string(GET_PROC, args, 0, "name")?;
        let graph_id = ctx.snapshot().graph_id();
        self.state.with_catalog(graph_id, |catalog| {
            catalog
                .ensure_fresh(ctx.snapshot(), &name)
                .map_err(algorithm_error)?;
            let projection = catalog.get(&name).ok_or_else(|| {
                algorithm_error(selene_algorithms::AlgorithmsError::NoSuchProjection {
                    name: name.clone(),
                })
            })?;
            Ok(ProcedureResult {
                rows: vec![projection_row(projection.projection())],
            })
        })
    }
}

struct ProjectionDrop {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ProjectionDrop {
    fn name(&self) -> &'static [&'static str] {
        &DROP_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![parameter("name", GqlType::String, false)]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalGraphProcedure for ProjectionDrop {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        expect_arity(DROP_PROC, args, 1)?;
        let name = required_string(DROP_PROC, args, 0, "name")?;
        let graph_id = ctx.snapshot().graph_id();
        self.state.with_catalog(graph_id, |catalog| {
            catalog.drop_projection(&name);
            Ok(unit_result())
        })
    }
}

struct ProjectionList {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ProjectionList {
    fn name(&self) -> &'static [&'static str] {
        &LIST_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        Vec::new()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        projection_output_columns()
    }
}

impl ExternalGraphProcedure for ProjectionList {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        expect_arity(LIST_PROC, args, 0)?;
        let graph_id = ctx.snapshot().graph_id();
        self.state.with_catalog(graph_id, |catalog| {
            let mut names = catalog.names();
            names.sort();
            let snapshots: Vec<ProjectionSnapshot> = names
                .iter()
                .map(|name| {
                    catalog
                        .ensure_fresh(ctx.snapshot(), name)
                        .map_err(algorithm_error)?;
                    let projection = catalog.get(name).ok_or_else(|| {
                        algorithm_error(selene_algorithms::AlgorithmsError::NoSuchProjection {
                            name: name.clone(),
                        })
                    })?;
                    Ok(ProjectionSnapshot::from(projection.projection()))
                })
                .collect::<Result<_, ProcedureError>>()?;
            let rows = snapshots.into_iter().map(projection_snapshot_row).collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct ProjectionSnapshot {
    name: StdArc<str>,
    generation: u64,
    node_count: u64,
    edge_count: u64,
}

impl From<&GraphProjection> for ProjectionSnapshot {
    fn from(projection: &GraphProjection) -> Self {
        Self {
            name: StdArc::<str>::from(projection.name()),
            generation: projection.generation(),
            node_count: projection.node_count() as u64,
            edge_count: projection.edge_count() as u64,
        }
    }
}

fn projection_snapshot_row(snapshot: ProjectionSnapshot) -> Vec<Value> {
    vec![
        Value::ExternalString(snapshot.name),
        Value::Uint(snapshot.generation),
        Value::Uint(snapshot.node_count),
        Value::Uint(snapshot.edge_count),
    ]
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter::new(name, ty, nullable)
}

fn projection_output_columns() -> Vec<ExternalOutputColumn> {
    vec![
        output("name", GqlType::String),
        output("generation", GqlType::Uint64),
        output("node_count", GqlType::Uint64),
        output("edge_count", GqlType::Uint64),
    ]
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn::new(name, ty)
}

fn projection_row(projection: &GraphProjection) -> Vec<Value> {
    projection_snapshot_row(ProjectionSnapshot::from(projection))
}

fn unit_result() -> ProcedureResult {
    ProcedureResult {
        rows: vec![Vec::new()],
    }
}
