//! DDL lowering.

use selene_core::intern_with_admission;

use crate::{
    DdlStatement, GqlType, TypePropertyConstraint, TypePropertyDef,
    analyze::{AnalyzedStatement, AnalyzedType},
    plan::{
        BindingTableColumn, BindingTableSchema, CatalogOp, ExecutionPlan, ImplDefinedCaps,
        PipelineOp, PlannedTypePropertyConstraint, PlannedTypePropertyDef, PlannerError,
    },
};

use super::expr;

/// Lower one DDL statement into a catalog plan.
pub(crate) fn lower_ddl(
    statement: &DdlStatement,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let op = match statement {
        DdlStatement::CreateGraph {
            name,
            or_replace,
            if_not_exists,
            span,
        } => CatalogOp::CreateGraph {
            name: *name,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            span: *span,
        },
        DdlStatement::DropGraph {
            name,
            if_exists,
            span,
        } => CatalogOp::DropGraph {
            name: *name,
            if_exists: *if_exists,
            span: *span,
        },
        DdlStatement::CreateNodeType {
            label,
            or_replace,
            if_not_exists,
            extends,
            properties,
            validation_mode,
            span,
        } => CatalogOp::CreateNodeType {
            label: *label,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            extends: *extends,
            properties: lower_property_defs(properties, analyzed)?,
            validation_mode: *validation_mode,
            span: *span,
        },
        DdlStatement::CreateEdgeType {
            label,
            or_replace,
            if_not_exists,
            endpoints,
            properties,
            validation_mode,
            span,
        } => CatalogOp::CreateEdgeType {
            label: *label,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            endpoints: endpoints.clone(),
            properties: lower_property_defs(properties, analyzed)?,
            validation_mode: *validation_mode,
            span: *span,
        },
        DdlStatement::DropNodeType {
            label,
            if_exists,
            span,
        } => CatalogOp::DropNodeType {
            label: *label,
            if_exists: *if_exists,
            span: *span,
        },
        DdlStatement::DropEdgeType {
            label,
            if_exists,
            span,
        } => CatalogOp::DropEdgeType {
            label: *label,
            if_exists: *if_exists,
            span: *span,
        },
        DdlStatement::ShowNodeTypes(span) => CatalogOp::ShowNodeTypes(*span),
        DdlStatement::ShowEdgeTypes(span) => CatalogOp::ShowEdgeTypes(*span),
    };

    Ok(ExecutionPlan {
        pattern_plan: None,
        pipeline: vec![PipelineOp::Catalog(op)],
        output_schema: ddl_output_schema(statement),
        impl_defined_caps: ImplDefinedCaps::default(),
    })
}

fn lower_property_defs(
    defs: &[TypePropertyDef],
    analyzed: &AnalyzedStatement,
) -> Result<Vec<PlannedTypePropertyDef>, PlannerError> {
    defs.iter()
        .map(|def| {
            let constraints = def
                .constraints
                .iter()
                .map(|constraint| lower_property_constraint(constraint, analyzed))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(PlannedTypePropertyDef {
                name: def.name,
                gql_type: def.gql_type.clone(),
                constraints,
                span: def.span,
            })
        })
        .collect()
}

fn lower_property_constraint(
    constraint: &TypePropertyConstraint,
    analyzed: &AnalyzedStatement,
) -> Result<PlannedTypePropertyConstraint, PlannerError> {
    Ok(match constraint {
        TypePropertyConstraint::NotNull(span) => PlannedTypePropertyConstraint::NotNull(*span),
        TypePropertyConstraint::Default(value, span) => PlannedTypePropertyConstraint::Default(
            expr::project_expr(value, None, analyzed)?,
            *span,
        ),
        TypePropertyConstraint::Immutable(span) => PlannedTypePropertyConstraint::Immutable(*span),
        TypePropertyConstraint::Unique(span) => PlannedTypePropertyConstraint::Unique(*span),
        TypePropertyConstraint::Indexed(span) => PlannedTypePropertyConstraint::Indexed(*span),
        TypePropertyConstraint::Searchable(span) => {
            PlannedTypePropertyConstraint::Searchable(*span)
        }
        TypePropertyConstraint::Dictionary(span) => {
            PlannedTypePropertyConstraint::Dictionary(*span)
        }
        TypePropertyConstraint::Fill(value, span) => {
            PlannedTypePropertyConstraint::Fill(*value, *span)
        }
        TypePropertyConstraint::Interval(value, span) => {
            PlannedTypePropertyConstraint::Interval(*value, *span)
        }
        TypePropertyConstraint::Encoding(value, span) => {
            PlannedTypePropertyConstraint::Encoding(*value, *span)
        }
    })
}

fn ddl_output_schema(statement: &DdlStatement) -> BindingTableSchema {
    match statement {
        DdlStatement::ShowNodeTypes(_) | DdlStatement::ShowEdgeTypes(_) => BindingTableSchema {
            columns: vec![
                BindingTableColumn {
                    name: Some(
                        intern_with_admission("label")
                            .expect("static SHOW column name")
                            .0,
                    ),
                    ty: AnalyzedType::Resolved(GqlType::String),
                },
                BindingTableColumn {
                    name: Some(
                        intern_with_admission("definition")
                            .expect("static SHOW column name")
                            .0,
                    ),
                    ty: AnalyzedType::DYNAMIC,
                },
            ],
        },
        _ => BindingTableSchema {
            columns: Vec::new(),
        },
    }
}
