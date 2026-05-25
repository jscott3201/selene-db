//! DDL lowering.

use selene_core::{IStr, intern_with_admission};

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
            extends,
            endpoints,
            properties,
            validation_mode,
            span,
        } => CatalogOp::CreateEdgeType {
            label: *label,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            extends: *extends,
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
        DdlStatement::ShowIndexes(span) => CatalogOp::ShowIndexes(*span),
        DdlStatement::ShowProcedures(span) => CatalogOp::ShowProcedures(*span),
    };

    let next_pipeline_op_id = crate::PipelineOpId::new(1);
    Ok(ExecutionPlan {
        category: analyzed.category,
        pattern_plan: None,
        pipeline: vec![PipelineOp::Catalog(op)],
        output_schema: ddl_output_schema(statement)?,
        impl_defined_caps: ImplDefinedCaps::default(),
        expr_ids: analyzed.expr_ids.clone(),
        subqueries: Default::default(),
        next_expr_id: super::next_expr_id(analyzed),
        next_pipeline_op_id,
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
        TypePropertyConstraint::Indexed { name, span } => PlannedTypePropertyConstraint::Indexed {
            name: *name,
            span: *span,
        },
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

fn ddl_output_schema(statement: &DdlStatement) -> Result<BindingTableSchema, PlannerError> {
    ddl_output_schema_with(statement, intern_with_admission)
}

fn ddl_output_schema_with<F, E>(
    statement: &DdlStatement,
    intern: F,
) -> Result<BindingTableSchema, PlannerError>
where
    F: FnMut(&str) -> Result<(IStr, bool), E>,
{
    match statement {
        DdlStatement::ShowNodeTypes(span) => show_output_schema(
            *span,
            "static SHOW NODE TYPES column 'label'",
            "static SHOW NODE TYPES column 'definition'",
            intern,
        ),
        DdlStatement::ShowEdgeTypes(span) => show_output_schema(
            *span,
            "static SHOW EDGE TYPES column 'label'",
            "static SHOW EDGE TYPES column 'definition'",
            intern,
        ),
        DdlStatement::ShowIndexes(span) => named_output_schema(
            *span,
            &[
                ("name", "static SHOW INDEXES column 'name'"),
                ("label", "static SHOW INDEXES column 'label'"),
                ("property", "static SHOW INDEXES column 'property'"),
                ("kind", "static SHOW INDEXES column 'kind'"),
            ],
            intern,
        ),
        DdlStatement::ShowProcedures(span) => named_output_schema(
            *span,
            &[
                ("name", "static SHOW PROCEDURES column 'name'"),
                ("tier", "static SHOW PROCEDURES column 'tier'"),
                ("mutability", "static SHOW PROCEDURES column 'mutability'"),
                ("signature", "static SHOW PROCEDURES column 'signature'"),
                ("description", "static SHOW PROCEDURES column 'description'"),
                (
                    "since_version",
                    "static SHOW PROCEDURES column 'since_version'",
                ),
                (
                    "capability_required",
                    "static SHOW PROCEDURES column 'capability_required'",
                ),
            ],
            intern,
        ),
        _ => Ok(BindingTableSchema {
            columns: Vec::new(),
        }),
    }
}

fn named_output_schema<F, E>(
    span: crate::SourceSpan,
    names: &[(&'static str, &'static str)],
    mut intern: F,
) -> Result<BindingTableSchema, PlannerError>
where
    F: FnMut(&str) -> Result<(IStr, bool), E>,
{
    let mut columns = Vec::with_capacity(names.len());
    for (name, detail) in names {
        columns.push(BindingTableColumn {
            name: Some(show_column_name(name, detail, span, &mut intern)?),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::String),
        });
    }
    Ok(BindingTableSchema { columns })
}

fn show_output_schema<F, E>(
    span: crate::SourceSpan,
    label_detail: &'static str,
    definition_detail: &'static str,
    mut intern: F,
) -> Result<BindingTableSchema, PlannerError>
where
    F: FnMut(&str) -> Result<(IStr, bool), E>,
{
    Ok(BindingTableSchema {
        columns: vec![
            BindingTableColumn {
                name: Some(show_column_name("label", label_detail, span, &mut intern)?),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            },
            BindingTableColumn {
                name: Some(show_column_name(
                    "definition",
                    definition_detail,
                    span,
                    &mut intern,
                )?),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
        ],
    })
}

fn show_column_name<F, E>(
    value: &'static str,
    detail: &'static str,
    span: crate::SourceSpan,
    admit_name: &mut F,
) -> Result<IStr, PlannerError>
where
    F: FnMut(&str) -> Result<(IStr, bool), E>,
{
    admit_name(value)
        .map(|(name, _was_new)| name)
        .map_err(|_err| PlannerError::InternerCapExhausted { detail, span })
}

#[cfg(test)]
mod defensive_tests {
    use super::*;
    use crate::SourceSpan;

    #[test]
    fn ddl_output_schema_reports_interner_cap_for_static_show_column() {
        let err = ddl_output_schema_with(
            &DdlStatement::ShowNodeTypes(SourceSpan::new(4, 15)),
            |_value| Err(()),
        )
        .expect_err("static SHOW column intern failure is recoverable");

        assert!(matches!(
            err,
            PlannerError::InternerCapExhausted {
                detail: "static SHOW NODE TYPES column 'label'",
                span,
            } if span == SourceSpan::new(4, 15)
        ));
    }
}
