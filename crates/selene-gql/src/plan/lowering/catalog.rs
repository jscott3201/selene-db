//! DDL lowering.

use selene_core::{IStr, intern};

use crate::{
    DdlStatement, GqlStatus, GqlType, KeyLabelSet, SourceSpan, TypePropertyConstraint,
    TypePropertyDef,
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
    caps: &ImplDefinedCaps,
) -> Result<ExecutionPlan, PlannerError> {
    let op = match statement {
        DdlStatement::CreateGraph {
            name,
            or_replace,
            if_not_exists,
            span,
        } => CatalogOp::CreateGraph {
            name: name.clone(),
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            span: *span,
        },
        DdlStatement::DropGraph {
            name,
            if_exists,
            span,
        } => CatalogOp::DropGraph {
            name: name.clone(),
            if_exists: *if_exists,
            span: *span,
        },
        DdlStatement::CreateNodeType {
            label,
            key_label_set,
            or_replace,
            if_not_exists,
            extends,
            properties,
            validation_mode,
            span,
        } => CatalogOp::CreateNodeType {
            label: label.clone(),
            key_labels: resolve_key_labels(key_label_set.as_ref(), Element::Node, caps, *span)?,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            extends: extends.clone(),
            properties: lower_property_defs(properties, analyzed)?,
            validation_mode: *validation_mode,
            span: *span,
        },
        DdlStatement::CreateEdgeType {
            label,
            key_label_set,
            or_replace,
            if_not_exists,
            extends,
            endpoints,
            properties,
            validation_mode,
            span,
        } => CatalogOp::CreateEdgeType {
            label: label.clone(),
            key_labels: resolve_key_labels(key_label_set.as_ref(), Element::Edge, caps, *span)?,
            or_replace: *or_replace,
            if_not_exists: *if_not_exists,
            extends: extends.clone(),
            endpoints: endpoints.clone(),
            properties: lower_property_defs(properties, analyzed)?,
            validation_mode: *validation_mode,
            span: *span,
        },
        DdlStatement::DropNodeType {
            label,
            if_exists,
            behavior,
            span,
        } => CatalogOp::DropNodeType {
            label: label.clone(),
            if_exists: *if_exists,
            behavior: *behavior,
            span: *span,
        },
        DdlStatement::DropEdgeType {
            label,
            if_exists,
            behavior,
            span,
        } => CatalogOp::DropEdgeType {
            label: label.clone(),
            if_exists: *if_exists,
            behavior: *behavior,
            span: *span,
        },
        DdlStatement::TruncateNodeType { label, span } => CatalogOp::TruncateNodeType {
            label: label.clone(),
            span: *span,
        },
        DdlStatement::TruncateEdgeType { label, span } => CatalogOp::TruncateEdgeType {
            label: label.clone(),
            span: *span,
        },
        DdlStatement::CreateIndex {
            name,
            label,
            properties,
            if_not_exists,
            span,
        } => CatalogOp::CreateIndex {
            name: name.clone(),
            label: label.clone(),
            properties: properties.clone(),
            if_not_exists: *if_not_exists,
            span: *span,
        },
        DdlStatement::DropIndex {
            name,
            if_exists,
            span,
        } => CatalogOp::DropIndex {
            name: name.clone(),
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

/// Node-vs-edge element-type discriminator for IL003 key-label-set diagnostics.
#[derive(Clone, Copy)]
enum Element {
    Node,
    Edge,
}

impl Element {
    const fn name(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Edge => "edge",
        }
    }

    /// IL003 key-label-set cardinality bounds for this element kind.
    const fn bounds(self, caps: &ImplDefinedCaps) -> (u32, u32) {
        match self {
            Self::Node => (caps.node_key_label_set_min, caps.node_key_label_set_max),
            Self::Edge => (caps.edge_key_label_set_min, caps.edge_key_label_set_max),
        }
    }

    /// Spec-defined GQLSTATUS for a below-minimum cardinality (§18.2 SR10 /
    /// §18.3 SR11).
    const fn below_minimum_status(self) -> GqlStatus {
        match self {
            Self::Node => GqlStatus::NODE_TYPE_KEY_LABELS_BELOW_MINIMUM,
            Self::Edge => GqlStatus::EDGE_TYPE_KEY_LABELS_BELOW_MINIMUM,
        }
    }

    /// Spec-defined GQLSTATUS for an above-maximum cardinality (§18.2 SR11 /
    /// §18.3 SR12).
    const fn above_maximum_status(self) -> GqlStatus {
        match self {
            Self::Node => GqlStatus::NODE_TYPE_KEY_LABELS_EXCEED_MAXIMUM,
            Self::Edge => GqlStatus::EDGE_TYPE_KEY_LABELS_EXCEED_MAXIMUM,
        }
    }
}

/// Resolve an explicit `<...type key label set>` (Feature GG21) into the catalog
/// key-label-set vector, enforcing the IL003 cardinality cap (ISO/IEC
/// 39075:2024 §18.2 SR10/SR11 for nodes, §18.3 SR11/SR12 for edges) and
/// rejecting the deferred separate-implied-label-set shape.
///
/// Returns an empty vector for the bare `:Name` element-type-name form
/// (`key_label_set == None`): the executor then keys on the singleton
/// `LabelSet::single(label)` (the implied key label set per §18.2 SR5c), exactly
/// as before GG21.
fn resolve_key_labels(
    key_label_set: Option<&KeyLabelSet>,
    element: Element,
    caps: &ImplDefinedCaps,
    fallback_span: SourceSpan,
) -> Result<Vec<IStr>, PlannerError> {
    let Some(key_label_set) = key_label_set else {
        return Ok(Vec::new());
    };
    let span = if key_label_set.span == SourceSpan::default() {
        fallback_span
    } else {
        key_label_set.span
    };

    // The separate-implied-label-set shape (`:Key => :Implied`) needs the
    // §18.2 SR7/SR8 union + containment identification, which is deferred.
    if !key_label_set.implied_labels.is_empty() {
        return Err(PlannerError::SeparateImpliedLabelSet {
            element: element.name(),
            span,
        });
    }

    // IL003: the effective key-label-set cardinality must fall in [min, max].
    // selene-db's default min == max == 1 (singleton), so the empty bare
    // `<implies>` (cardinality 0) and any multi-label phrase (cardinality > 1)
    // are rejected here with the spec-defined GQLSTATUS.
    let cardinality = key_label_set.labels.len();
    let (min, max) = element.bounds(caps);
    if cardinality < min as usize {
        return Err(PlannerError::KeyLabelSetCardinality {
            element: element.name(),
            actual: cardinality,
            min,
            max,
            status: element.below_minimum_status(),
            span,
        });
    }
    if cardinality > max as usize {
        return Err(PlannerError::KeyLabelSetCardinality {
            element: element.name(),
            actual: cardinality,
            min,
            max,
            status: element.above_maximum_status(),
            span,
        });
    }

    Ok(key_label_set.labels.clone())
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
                name: def.name.clone(),
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
            name: name.clone(),
            span: *span,
        },
    })
}

fn ddl_output_schema(statement: &DdlStatement) -> Result<BindingTableSchema, PlannerError> {
    ddl_output_schema_with(statement, intern)
}

fn ddl_output_schema_with<F, E>(
    statement: &DdlStatement,
    intern: F,
) -> Result<BindingTableSchema, PlannerError>
where
    F: FnMut(&str) -> Result<IStr, E>,
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
    F: FnMut(&str) -> Result<IStr, E>,
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
    F: FnMut(&str) -> Result<IStr, E>,
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
    F: FnMut(&str) -> Result<IStr, E>,
{
    admit_name(value).map_err(|_err| PlannerError::InternerCapExhausted { detail, span })
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
